#!/usr/bin/env python3
"""Agent view PTY journey against a fake Ollama (stdlib only; run after cargo build).

`/go` plans two tasks (task 2 depends on task 1). Task 1's first generation streams
some code and is held: F6 selects its row, Enter opens its agent view with the
streamed code, and a typed note steers it. The fixture observes the cancelled
request, then a new request that starts with the owner's instruction; it passes and
autopilot accepts it. Task 2's generation is held until autopilot is paused, then
fails; `/watch 2` opens it, a note repairs it with that reason, and resuming
autopilot accepts the repair and integrates both tasks on one local branch.
"""
import http.server
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import termios
import threading
import time
import unittest

from inference_terminal_smoke import Terminal

HEADER = ('OWNER INSTRUCTION (from the repository owner; follow it within the '
          'approved files and check below)')
STEER = 'use the integer 42'
REPAIR = 'notes.txt must contain the word ok'
CHECK_CALC = ['/usr/bin/python3', '-B', '-c', 'from calc import answer; assert answer() == 42']
CHECK_NOTES = ['/usr/bin/python3', '-B', '-c',
               "from pathlib import Path; assert 'ok' in Path('notes.txt').read_text()"]


def chunk(content, done=False):
    return (json.dumps({'message': {'content': content}, 'done': done}) + '\n').encode()


class AgentServer:
    def __init__(self):
        self.lock = threading.Lock()
        self.requests = []
        self.bodies = {}
        self.errors = []
        self.release_notes = threading.Event()
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{"models":[{"name":"fixture"}]}')

            def closed(self):
                """The client dropped the connection (a cancelled generation)."""
                readable, _, _ = select.select([self.connection], [], [], 0.05)
                if not readable:
                    return False
                try:
                    return self.connection.recv(1, socket.MSG_PEEK) == b''
                except OSError:
                    return True

            def do_POST(self):
                if self.path == '/api/generate':
                    self.rfile.read(int(self.headers.get('Content-Length', 0)))
                    body = b'{"done":true,"done_reason":"load"}\n'
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                try:
                    request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    prompt = request['messages'][-1]['content']
                    if request['messages'][0]['content'].startswith('Act as Frontier Architect'):
                        marker = 'PLAN'
                    elif prompt.startswith(HEADER) and 'Repair #1:' in prompt:
                        marker = 'OWNER_CALC'
                    elif prompt.startswith(HEADER) and 'Repair #2:' in prompt:
                        marker = 'OWNER_NOTES'
                    elif prompt.startswith('Implement this task: Make answer'):
                        marker = 'CALC'
                    elif prompt.startswith('Implement this task: Write notes'):
                        marker = 'NOTES'
                    else:
                        raise AssertionError(f'Unexpected request: {prompt[:160]!r}')
                    with fixture.lock:
                        fixture.requests.append(marker)
                        fixture.bodies[marker] = prompt
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/x-ndjson')
                    self.end_headers()
                    if marker == 'PLAN':
                        self.wfile.write(chunk(json.dumps({'tasks': [
                            {'title': 'Make answer return 42', 'acceptance': ['answer() returns 42'],
                             'model': request['model'], 'dependencies': [],
                             'policy': {'files': ['calc.py'], 'check': CHECK_CALC}},
                            {'title': 'Write notes', 'acceptance': ['notes say ok'],
                             'model': request['model'], 'dependencies': [1],
                             'policy': {'files': ['notes.txt'], 'check': CHECK_NOTES}},
                        ]}), True))
                    elif marker == 'CALC':
                        # Stream the start of an answer, then hold until the client leaves.
                        self.wfile.write(chunk('=== FILE: calc.py ===\ndef answer():\n'))
                        self.wfile.flush()
                        deadline = time.monotonic() + 90
                        while not self.closed():
                            if time.monotonic() > deadline:
                                raise AssertionError('Held generation was never cancelled')
                        with fixture.lock:
                            fixture.requests.append('CANCELLED')
                        return
                    elif marker == 'OWNER_CALC':
                        self.wfile.write(chunk('=== FILE: calc.py ===\ndef answer():\n'))
                        self.wfile.write(chunk('    return 42\n=== END FILE ===\n', True))
                    elif marker == 'NOTES':
                        if not fixture.release_notes.wait(90):
                            raise AssertionError('Notes worker was never released')
                        self.wfile.write(chunk('=== FILE: notes.txt ===\nnothing yet\n=== END FILE ===\n', True))
                    else:
                        self.wfile.write(chunk('=== FILE: notes.txt ===\nok\n=== END FILE ===\n', True))
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
                except Exception as error:  # surfaced by the test loop
                    with fixture.lock:
                        fixture.errors.append(repr(error))

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def markers(self):
        with self.lock:
            return list(self.requests)

    def body(self, marker):
        with self.lock:
            return self.bodies.get(marker, '')

    def close(self):
        self.release_notes.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


class AgentTerminalSmoke(unittest.TestCase):
    def setUp(self):
        fallback = Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'
        self.binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(fallback))).resolve()
        self.assertTrue(self.binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {self.binary}')
        temporary = tempfile.TemporaryDirectory(prefix='alfredo agent journey ')
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.workspace, self.state = root / 'workspace', root / 'state'
        self.workspace.mkdir()
        (self.workspace / '.gitattributes').write_text('* text eol=lf\n')
        (self.workspace / 'calc.py').write_text('def answer():\n    return 0\n')
        self.git_env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
        for args in [
            ['init', '-q', '--template=', '--initial-branch=main'],
            ['config', 'user.name', 'Fixture'],
            ['config', 'user.email', 'fixture@example.invalid'],
            ['add', '.'],
            ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm', 'fixture'],
        ]:
            self.git(*args)
        self.fixture = AgentServer()
        self.addCleanup(self.fixture.close)
        self.terminal = None

    def tearDown(self):
        if self.terminal is not None:
            self.terminal.close()

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.workspace), *args], env=self.git_env, check=True,
                              timeout=10, capture_output=True, text=True).stdout.strip()

    def wait_until(self, label, predicate, timeout=60):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.terminal.pump()
            self.assertFalse(self.fixture.errors, self.fixture.errors)
            if predicate():
                return
            time.sleep(0.02)
        self.fail(f'Timed out: {label}\nRequests: {self.fixture.markers()}\n'
                  f'Tasks: {self.statuses()}\n{self.terminal.screen()}')

    def screen_has(self, text, timeout=60):
        self.wait_until(f'screen contains {text!r}', lambda: text in self.terminal.screen(), timeout)

    def statuses(self):
        paths = list(self.state.glob('rust-tasks-v1/*/tasks.json'))
        if not paths:
            return []
        return [(task['id'], task['status'], task.get('repair_of'))
                for task in json.loads(paths[0].read_text())['tasks']]

    def highlighted(self, text):
        return any(line.startswith('│›') and text in line for line in self.terminal.screen().splitlines())

    def test_steer_a_streaming_worker_and_repair_a_failed_one_from_the_agent_view(self):
        head = self.git('rev-parse', 'HEAD')
        self.terminal = terminal = Terminal(self.binary, self.fixture.endpoint, self.workspace,
                                            self.state, 'agents', height=36, width=140)
        self.screen_has('ALFREDO')
        terminal.send('/go Make answer return 42 and write notes\r')
        self.wait_until('first generation streaming', lambda: 'CALC' in self.fixture.markers())

        # F6, select the running worker's row, Enter: its agent view shows the stream.
        terminal.send(b'\x1b[17~')
        self.screen_has('Enter open')
        for _ in range(8):
            if self.highlighted('#1 Make answer'):
                break
            terminal.send(b'\x1b[B')
            time.sleep(0.05)
            terminal.pump()
        self.wait_until('task #1 row highlighted', lambda: self.highlighted('#1 Make answer'), 5)
        terminal.send('\r')
        self.screen_has('Agent · worker #1 · running')
        self.screen_has('To worker #1 · Enter send · Esc back')
        self.screen_has('▸ calc.py')
        self.screen_has('def answer():')

        terminal.send(f'{STEER}\r')
        self.screen_has('Steering #1')
        self.wait_until('owner request after the cancelled one',
                        lambda: 'OWNER_CALC' in self.fixture.markers())
        markers = self.fixture.markers()
        self.assertEqual(markers[:4], ['PLAN', 'CALC', 'CANCELLED', 'OWNER_CALC'], markers)
        self.assertTrue(self.fixture.body('OWNER_CALC').startswith(
            f'{HEADER}\n{STEER}\n\nImplement this task: Repair #1: Owner: {STEER}\n'),
            self.fixture.body('OWNER_CALC')[:300])
        self.screen_has('You → repair #3')
        # The steered run passes and autopilot accepts it; the dependent task starts.
        self.wait_until('steer accepted', lambda: (3, 'accepted', 1) in self.statuses())
        self.assertIn((1, 'cancelled', None), self.statuses())

        # Task 2 is held; pause autopilot, then let it fail.
        self.wait_until('notes generation', lambda: 'NOTES' in self.fixture.markers())
        terminal.send(b'\x1b[15~')  # F5
        self.screen_has('paused')
        self.fixture.release_notes.set()
        self.wait_until('notes failed', lambda: (2, 'failed', None) in self.statuses())

        terminal.send('/watch 2\r')
        self.screen_has('Agent · worker #2 · failed')
        self.screen_has('To worker #2 · Enter send · Esc back')
        terminal.send(f'{REPAIR}\r')
        self.screen_has('Repairing #2 with your note')
        self.wait_until('owner repair requested', lambda: 'OWNER_NOTES' in self.fixture.markers())
        self.assertTrue(self.fixture.body('OWNER_NOTES').startswith(f'{HEADER}\n{REPAIR}\n\n'))
        self.assertLess(self.fixture.body('OWNER_NOTES').index(REPAIR),
                        self.fixture.body('OWNER_NOTES').index('WHAT IS STILL FAILING'))
        self.wait_until('repair checked', lambda: (4, 'review-ready', 2) in self.statuses())
        self.screen_has('You → repair #4')

        # Resuming autopilot accepts the instructed repair and integrates.
        terminal.send(b'\x1b[15~')
        self.screen_has('git switch alfredo/go-', 60)
        self.assertEqual(self.statuses(), [
            (1, 'cancelled', None), (2, 'failed', None), (3, 'accepted', 1), (4, 'accepted', 2)])
        self.screen_has('Autopilot done   2/2 accepted   1 repair   git switch')
        branch = self.git('for-each-ref', '--format=%(refname:short)', 'refs/heads/alfredo/').split()[0]
        self.assertEqual(self.git('show', f'{branch}:calc.py'), 'def answer():\n    return 42')
        self.assertEqual(self.git('show', f'{branch}:notes.txt'), 'ok')
        self.assertEqual(self.git('rev-parse', 'HEAD'), head)
        self.assertEqual(self.git('status', '--porcelain'), '')

        # Esc leaves the agent view.
        terminal.send(b'\x1b')
        self.wait_until('agent view closed',
                        lambda: 'To worker #2' not in self.terminal.screen(), 10)
        terminal.expected_exit = True
        terminal.send(b'\x11')
        self.assertEqual(terminal.process.wait(timeout=10), 0)
        self.assertEqual(termios.tcgetattr(terminal.slave), terminal.original)


if __name__ == '__main__':
    unittest.main()
