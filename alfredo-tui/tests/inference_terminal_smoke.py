#!/usr/bin/env python3
"""Two installed Alfredo terminals share endpoint capacity (stdlib Linux PTY fixture)."""
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import threading
import time
import unittest

from terminal_smoke import visible_screen


class FixtureServer:
    def __init__(self):
        self.lock = threading.Lock()
        self.requests = []
        self.errors = []
        self.gates = {name: threading.Event() for name in (
            'HOLDER_A', 'HOLDER_WORKER', 'HOLDER_KILLED',
        )}
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
                try:
                    request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    prompt = request['messages'][-1]['content']
                    worker = prompt.startswith('Implement this task:')
                    marker = 'WORKER' if worker else prompt.removeprefix('Explain INFERENCE_')
                    with fixture.lock:
                        fixture.requests.append((marker, request))
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/x-ndjson')
                    self.end_headers()

                    def send(content, done=False):
                        self.wfile.write((json.dumps({
                            'message': {'content': content}, 'done': done,
                        }) + '\n').encode())
                        self.wfile.flush()

                    if marker == 'HOLDER_KILLED':
                        send('OWNER_PARTIAL_BEFORE_EXIT')
                    if marker in fixture.gates and not fixture.gates[marker].wait(45):
                        raise AssertionError(f'Fixture holder timed out: {marker}')
                    content = json.dumps({'files': [{'path': 'answer.py', 'content': 'VALUE = 42\n'}]}) if worker else f'REPLY_{marker}'
                    send(content, True)
                except (BrokenPipeError, ConnectionResetError):
                    # Killing a client does not imply that its server handler stopped.
                    pass
                except Exception as error:
                    with fixture.lock:
                        fixture.errors.append(repr(error))

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def count(self, marker=None):
        with self.lock:
            return sum(marker is None or actual == marker for actual, _ in self.requests)

    def close(self):
        for gate in self.gates.values():
            gate.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


class Terminal:
    def __init__(self, binary, endpoint, workspace, state, mission, resume=False,
                 conversation='default', height=36, width=140):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.height, self.width = height, width
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ,
                    struct.pack('HHHH', self.height, self.width, 0, 0))
        self.output = bytearray()
        self.expected_exit = False
        self.closed = False
        self.process = subprocess.Popen([
            str(binary), '--model', 'fixture', '--endpoint', endpoint,
            '--parallel-models', '1', '--workspace', str(workspace),
            '--state-dir', str(state), '--mission' if resume else '--new-mission', mission,
            '--conversation', conversation,
        ], stdin=self.slave, stdout=self.slave, stderr=self.slave, cwd=workspace,
            env=dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=str(state)))

    def send(self, text):
        os.write(self.master, text.encode() if isinstance(text, str) else text)

    def pump(self):
        if self.closed:
            return
        while select.select([self.master], [], [], 0)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                break  # Linux PTYs report EIO after their child exits.
            if not chunk:
                break
            self.output.extend(chunk)
            if len(self.output) > 2 * 1024 * 1024:
                raise AssertionError('Terminal fixture output exceeded 2 MiB')
            if b'\x1b[6n' in chunk and self.process.poll() is None:
                self.send(b'\x1b[1;1R')
        if not self.expected_exit and self.process.poll() is not None:
            raise AssertionError(f'Terminal exited unexpectedly ({self.process.returncode}):\n{self.screen()}')

    def screen(self):
        return visible_screen(self.output, self.height, self.width)

    def kill(self):
        self.expected_exit = True
        self.process.kill()
        self.process.wait(timeout=3)

    def close(self):
        if self.closed:
            return
        self.expected_exit = True
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=3)
        os.close(self.master)
        os.close(self.slave)
        self.closed = True


class SharedInferenceTerminalSmoke(unittest.TestCase):
    def test_shared_capacity_cancellation_owner_exit_and_restart_without_replay(self):
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
                      str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'))).resolve()
        fixture = FixtureServer()
        terminals = []
        with tempfile.TemporaryDirectory(prefix='alfredo shared inference installed ') as directory:
            root = Path(directory)
            workspaces = [root / 'workspace-a', root / 'workspace-b']
            states = [root / 'state-a', root / 'state-b']
            git_env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
            for workspace in workspaces:
                workspace.mkdir()
                (workspace / 'answer.py').write_text('VALUE = 0\n')
                for args in [
                    ['init', '-q', '--template=', '--initial-branch=main'],
                    ['config', 'user.name', 'Fixture'],
                    ['config', 'user.email', 'fixture@example.invalid'],
                    ['add', 'answer.py'],
                    ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm', 'fixture'],
                ]:
                    subprocess.run(['git', '-C', str(workspace), *args], env=git_env,
                                   check=True, timeout=5, stdout=subprocess.DEVNULL)

            def wait_until(label, predicate, timeout=8):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    for terminal in terminals:
                        terminal.pump()
                    with fixture.lock:
                        self.assertFalse(fixture.errors, fixture.errors)
                    if predicate():
                        return
                    time.sleep(0.025)
                screens = '\n\n'.join(f'Terminal {index}:\n{terminal.screen()}'
                                        for index, terminal in enumerate(terminals))
                self.fail(f'Timed out: {label}\nRequests: {fixture.requests!r}\n{screens}')

            def screen_has(terminal, text, timeout=8):
                wait_until(f'screen contains {text!r}', lambda: text in terminal.screen(), timeout)

            def unchanged_requests(expected, duration=0.4):
                deadline = time.monotonic() + duration
                while time.monotonic() < deadline:
                    for terminal in terminals:
                        terminal.pump()
                    self.assertEqual(fixture.count(), expected, fixture.requests)
                    time.sleep(0.025)

            def task_snapshot():
                paths = list(states[1].glob('rust-tasks-v1/*/tasks.json'))
                return json.loads(paths[0].read_text()) if paths else {'revision': 0, 'tasks': []}

            def saved_partial():
                paths = list(states[0].glob('rust-tasks-v1/*/conversations-*.json'))
                return bool(paths) and any(
                    message['content'] == 'OWNER_PARTIAL_BEFORE_EXIT'
                    for session in json.loads(paths[0].read_text())['sessions']
                    for message in session['messages']
                )

            try:
                first = Terminal(binary, fixture.endpoint, workspaces[0], states[0], 'first')
                terminals.append(first)
                second = Terminal(binary, fixture.endpoint, workspaces[1], states[1], 'second')
                terminals.append(second)
                screen_has(first, 'Sessions')
                screen_has(second, 'Sessions')
                self.assertNotEqual(states[0], states[1])
                self.assertEqual(fixture.count(), 0)

                first.send('Explain INFERENCE_HOLDER_A\r')
                wait_until('first terminal owns the only HTTP slot', lambda: fixture.count('HOLDER_A') == 1)
                screen_has(first, 'Waiting for model server')
                second.send('Explain INFERENCE_SECOND\r')
                screen_has(second, 'Queued for Alfredo')
                screen_has(second, 'position 1/1')
                self.assertEqual(fixture.count('SECOND'), 0)
                second.send(b'\x1b')
                screen_has(second, 'Cancelled')
                unchanged_requests(1)
                second.send(b'\x12')  # Explicit retry creates a new live queue ticket.
                screen_has(second, 'Queued for Alfredo')
                screen_has(second, 'position 1/1')
                self.assertEqual(fixture.count('SECOND'), 0)
                fixture.gates['HOLDER_A'].set()
                screen_has(first, 'REPLY_HOLDER_A')
                screen_has(second, 'REPLY_SECOND')
                self.assertEqual(fixture.count('SECOND'), 1)
                unchanged_requests(2)

                # A real approved worker uses Background and can be cancelled before HTTP.
                first.send('Explain INFERENCE_HOLDER_WORKER\r')
                wait_until('worker fixture capacity holder', lambda: fixture.count('HOLDER_WORKER') == 1)
                second.send('/task INFERENCE_QUEUED_WORKER\r')
                wait_until('worker proposal saved', lambda: task_snapshot()['revision'] == 1)
                screen_has(second, 'revision 1')
                policy = {'files': ['answer.py'], 'check': ['/usr/bin/python3', '-B', '-c', 'assert True']}
                second.send('/permit 1 ' + json.dumps(policy) + '\r')
                wait_until('worker permission saved', lambda: task_snapshot()['revision'] == 2)
                screen_has(second, 'revision 2')
                second.send('/approve 1\r')
                wait_until('worker approved', lambda: task_snapshot()['tasks'][0]['status'] == 'approved')
                screen_has(second, 'revision 3')
                second.send('/run 1\r')
                screen_has(second, 'Shared Alfredo capacity', timeout=15)
                screen_has(second, 'background')
                screen_has(second, 'position 1/1')
                self.assertEqual(fixture.count('WORKER'), 0)
                second.send('/cancel-task 1\r')
                wait_until('queued worker cancellation saved',
                           lambda: task_snapshot()['tasks'][0]['status'] == 'cancelled', timeout=15)
                self.assertEqual(fixture.count('WORKER'), 0)
                fixture.gates['HOLDER_WORKER'].set()
                screen_has(first, 'REPLY_HOLDER_WORKER')
                second.send('/chat\r')
                screen_has(second, 'Session 1')
                second.send('Explain INFERENCE_AFTER_WORKER_CANCEL\r')
                screen_has(second, 'REPLY_AFTER_WORKER_CANCEL')
                self.assertEqual(fixture.count('WORKER'), 0)
                self.assertEqual((workspaces[1] / 'answer.py').read_text(), 'VALUE = 0\n')

                # Kill the active ticket owner. The server handler deliberately stays open;
                # the guarantee being checked is release of Alfredo client capacity.
                first.send('Explain INFERENCE_HOLDER_KILLED\r')
                screen_has(first, 'OWNER_PARTIAL_BEFORE_EXIT')
                wait_until('partial reply durably checkpointed before process death', saved_partial)
                second.send('Explain INFERENCE_SURVIVOR\r')
                screen_has(second, 'Queued for Alfredo')
                screen_has(second, 'position 1/1')
                self.assertEqual(fixture.count('SURVIVOR'), 0)
                first.kill()
                screen_has(second, 'REPLY_SURVIVOR')
                self.assertEqual(fixture.count('SURVIVOR'), 1)
                self.assertFalse(fixture.gates['HOLDER_KILLED'].is_set())
                fixture.gates['HOLDER_KILLED'].set()
                first.close()
                before_restart = fixture.count()
                restored = Terminal(binary, fixture.endpoint, workspaces[0], states[0], 'first', resume=True)
                terminals.append(restored)
                screen_has(restored, 'Conversations restored')
                screen_has(restored, 'OWNER_PARTIAL_BEFORE_EXIT')
                screen_has(restored, 'Disconnected / failed')
                self.assertNotIn('Queued for Alfredo', restored.screen())
                restored.send('/models\r')
                screen_has(restored, 'Installed models')
                unchanged_requests(before_restart)
                self.assertEqual(fixture.count('HOLDER_KILLED'), 1)
                self.assertEqual(fixture.count('SURVIVOR'), 1)
                self.assertEqual(fixture.count('WORKER'), 0)

                for terminal in [second, restored]:
                    terminal.expected_exit = True
                    terminal.send(b'\x11')
                    wait_until('clean terminal exit', lambda: terminal.process.poll() is not None)
                    self.assertEqual(terminal.process.returncode, 0, terminal.screen())
                    self.assertEqual(termios.tcgetattr(terminal.slave), terminal.original)
                print('Installed shared inference acceptance passed: two terminals, queued chat/worker cancellation, owner exit, restart without replay')
            finally:
                for terminal in terminals:
                    terminal.close()
                fixture.close()


if __name__ == '__main__':
    unittest.main()
