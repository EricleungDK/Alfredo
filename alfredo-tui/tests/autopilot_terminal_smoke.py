#!/usr/bin/env python3
"""Autopilot PTY journey against a fake Ollama (stdlib only; run after cargo build).

`/go` plans two tasks (task 2 depends on task 1). Task 1's first attempt fails its
check and the automatic repair passes. The dependent worker is held so the user
can pause with F5, quit, restart (restored paused, no replay), then `/resume`
to finish on one local integration branch while the source HEAD stays put.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import termios
import threading
import time
import unittest

from inference_terminal_smoke import Terminal

CHECK_CALC = ['/usr/bin/python3', '-B', '-c', 'from calc import answer; assert answer() == 42']
CHECK_APP = ['/usr/bin/python3', '-B', '-c', "from app import main; assert main() == 'answer=42'"]


class AutopilotServer:
    def __init__(self):
        self.lock = threading.Lock()
        self.requests = []
        self.errors = []
        self.release_app = threading.Event()
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{"models":[{"name":"fixture"}]}')

            def do_POST(self):
                if self.path == '/api/generate':
                    # Startup/model-selection preload: loads the model, generates nothing.
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
                        content = json.dumps({'tasks': [
                            {'title': 'Make answer return 42', 'acceptance': ['answer() returns 42'],
                             'model': request['model'], 'dependencies': [],
                             'policy': {'files': ['calc.py'], 'check': CHECK_CALC}},
                            {'title': 'Add app reporting the answer', 'acceptance': ['main() reports answer=42'],
                             'model': request['model'], 'dependencies': [1],
                             'policy': {'files': ['app.py'], 'check': CHECK_APP}},
                        ]})
                    elif prompt.startswith('Implement this task: Repair #1:'):
                        marker = 'REPAIR'
                        content = json.dumps({'files': [{'path': 'calc.py', 'content': 'def answer():\n    return 42\n'}]})
                    elif prompt.startswith('Implement this task: Make answer'):
                        marker = 'CALC'
                        content = json.dumps({'files': [{'path': 'calc.py', 'content': 'def answer():\n    return 41\n'}]})
                    elif prompt.startswith('Implement this task: Add app'):
                        marker = 'APP'
                        content = json.dumps({'files': [{'path': 'app.py', 'content':
                            "from calc import answer\n\n\ndef main():\n    return f'answer={answer()}'\n"}]})
                    else:
                        raise AssertionError(f'Unexpected request: {prompt[:120]!r}')
                    with fixture.lock:
                        fixture.requests.append(marker)
                    if marker == 'APP' and not fixture.release_app.wait(60):
                        raise AssertionError('App worker was never released')
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/x-ndjson')
                    self.end_headers()
                    self.wfile.write((json.dumps({'message': {'content': content}, 'done': True}) + '\n').encode())
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

    def close(self):
        self.release_app.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


class AutopilotTerminalSmoke(unittest.TestCase):
    def setUp(self):
        fallback = Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'
        self.binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(fallback))).resolve()
        self.assertTrue(self.binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {self.binary}')
        temporary = tempfile.TemporaryDirectory(prefix='alfredo autopilot journey ')
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
        self.fixture = AutopilotServer()
        self.addCleanup(self.fixture.close)
        self.terminals = []
        self.addCleanup(lambda: [terminal.close() for terminal in self.terminals])

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.workspace), *args], env=self.git_env, check=True,
                              timeout=10, capture_output=True, text=True).stdout.strip()

    def terminal(self, resume):
        terminal = Terminal(self.binary, self.fixture.endpoint, self.workspace, self.state,
                            'autopilot', resume=resume)
        self.terminals.append(terminal)
        self.screen_has(terminal, 'ALFREDO')
        return terminal

    def wait_until(self, label, predicate, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for terminal in self.terminals:
                terminal.pump()
            self.assertFalse(self.fixture.errors, self.fixture.errors)
            if predicate():
                return
            time.sleep(0.02)
        screens = '\n\n'.join(t.screen() for t in self.terminals if not t.closed)
        self.fail(f'Timed out: {label}\nRequests: {self.fixture.markers()}\nTasks: {self.statuses()}\n{screens}')

    def screen_has(self, terminal, text, timeout=30):
        self.wait_until(f'screen contains {text!r}', lambda: text in terminal.screen(), timeout)

    def statuses(self):
        paths = list(self.state.glob('rust-tasks-v1/*/tasks.json'))
        if not paths:
            return []
        return [(task['id'], task['status'], task.get('repair_of')) for task in json.loads(paths[0].read_text())['tasks']]

    def quit(self, terminal):
        terminal.expected_exit = True
        terminal.send(b'\x11')
        self.assertEqual(terminal.process.wait(timeout=10), 0)
        self.assertEqual(termios.tcgetattr(terminal.slave), terminal.original)
        terminal.close()

    def test_go_repairs_pauses_restarts_paused_and_resumes_to_one_integration_branch(self):
        head = self.git('rev-parse', 'HEAD')
        terminal = self.terminal(resume=False)
        terminal.send('/go Make answer return 42 and add app\r')
        self.screen_has(terminal, 'Autopilot')
        self.wait_until('dependent worker requested after repair',
                        lambda: self.fixture.markers().count('APP') == 1, 60)
        self.assertEqual(self.fixture.markers(), ['PLAN', 'CALC', 'REPAIR', 'APP'])
        self.assertEqual(self.statuses(), [(1, 'failed', None), (2, 'running', None), (3, 'accepted', 1)])

        terminal.send(b'\x1b[15~')  # F5 pauses; the running worker still finishes.
        self.screen_has(terminal, 'paused')
        self.fixture.release_app.set()
        self.wait_until('paused worker finished', lambda: (2, 'review-ready', None) in self.statuses())
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            terminal.pump()
            time.sleep(0.05)
        self.assertIn((2, 'review-ready', None), self.statuses(), 'paused autopilot must not review')
        self.quit(terminal)

        terminal = self.terminal(resume=True)
        self.screen_has(terminal, 'Autopilot ‖ paused')
        before = self.fixture.markers()
        deadline = time.monotonic() + 1.5
        while time.monotonic() < deadline:
            terminal.pump()
            time.sleep(0.05)
        self.assertEqual(self.fixture.markers(), before, 'restart never replays inference')
        self.assertIn((2, 'review-ready', None), self.statuses())

        terminal.send('/resume\r')
        self.screen_has(terminal, 'git switch alfredo/go-', 60)
        self.assertEqual(self.statuses(), [(1, 'failed', None), (2, 'accepted', None), (3, 'accepted', 1)])
        self.assertEqual(self.fixture.markers(), before)
        branches = self.git('for-each-ref', '--format=%(refname:short)', 'refs/heads/alfredo/').split()
        self.assertEqual(len(branches), 1, branches)
        branch = branches[0]
        self.assertTrue(branch.startswith('alfredo/go-'), branch)
        self.assertIn('Autopilot ✓ done   2/2   1 repair', terminal.screen())
        # The footer's stale notice is replaced by the one-line result.
        self.screen_has(terminal, 'Autopilot done   2/2 accepted   1 repair   git switch')
        self.assertNotIn('Autopilot resumed', terminal.screen())
        self.assertEqual(self.git('show', f'{branch}:calc.py'), 'def answer():\n    return 42')
        self.assertIn("answer={answer()}", self.git('show', f'{branch}:app.py'))
        self.assertEqual(self.git('rev-parse', 'HEAD'), head)
        self.assertEqual(self.git('symbolic-ref', 'HEAD'), 'refs/heads/main')
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual((self.workspace / 'calc.py').read_text(), 'def answer():\n    return 0\n')
        self.quit(terminal)

    def test_go_in_repository_without_commits_fails_once_with_the_fix(self):
        empty = self.workspace.parent / 'empty'
        empty.mkdir()
        subprocess.run(['git', '-C', str(empty), 'init', '-q', '--initial-branch=main'], env=self.git_env,
                       check=True, timeout=10)
        self.workspace = empty
        terminal = self.terminal(resume=False)
        self.screen_has(terminal, 'no commits yet')
        terminal.send('/go hi\r')
        self.screen_has(terminal, 'Autopilot')
        self.screen_has(terminal, 'git commit --allow-empty')
        self.screen_has(terminal, 'Autopilot ✗ failed')
        time.sleep(0.5)
        terminal.pump()
        screen = terminal.screen()
        self.assertEqual(screen.count('Autopilot ✗ failed'), 1, screen)
        self.assertEqual(screen.count('Planning failed'), 0, screen)
        self.assertNotIn('3 times', screen)
        self.assertNotIn('ambiguous argument', screen)
        self.assertEqual(self.fixture.markers(), [])
        self.quit(terminal)


if __name__ == '__main__':
    unittest.main()
