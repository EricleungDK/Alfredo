"""Real Linux PTY acceptance; run after cargo build (no third-party modules)."""
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import re
import select
import struct
import subprocess
import termios
import tempfile
import threading
import time
import unittest
import unicodedata


def visible_screen(output, height=24, width=100):
    """Reconstruct the CSI cursor/erase/style subset emitted by this PTY fixture.

    Buffer-backend Rust tests cover Unicode layout; this ASCII workflow fixture
    needs cell-diff reconstruction, not substring matches against terminal bytes.
    Unsupported complete controls fail explicitly rather than faking a screen.
    """
    cells = [[' '] * width for _ in range(height)]
    primary = None
    row = column = 0
    text = output.decode(errors='replace')
    index = 0
    while index < len(text):
        if text[index] == '\x1b':
            match = re.match(r'\x1b\[([0-?]*)([ -/]*)([@-~])', text[index:])
            if match is None:
                break  # The next read may complete an escape sequence.
            args, _intermediate, command = match.groups()
            if command in ('H', 'f'):
                numbers = [int(value or 1) for value in args.split(';')]
                row = numbers[0] - 1
                column = (numbers[1] if len(numbers) > 1 else 1) - 1
            elif command == 'h' and args == '?1049':
                primary = ([line[:] for line in cells], row, column)
                cells = [[' '] * width for _ in range(height)]
                row = column = 0
            elif command == 'l' and args == '?1049':
                if primary is not None:
                    cells, row, column = primary
                    primary = None
                else:
                    cells = [[' '] * width for _ in range(height)]
                    row = column = 0
            elif command == 'J' and args == '2':
                cells = [[' '] * width for _ in range(height)]
            elif command not in ('m', 'h', 'l', 'n'):
                raise AssertionError(f'Unsupported terminal control: {match.group()!r}')
            index += len(match.group())
            continue
        char = text[index]
        if char == '\r':
            column = 0
        elif char == '\n':
            row += 1
        elif ord(char) >= 32 and not unicodedata.combining(char):
            if 0 <= row < height and 0 <= column < width:
                cells[row][column] = char
            column += 2 if unicodedata.east_asian_width(char) in ('W', 'F') else 1
        index += 1
    return '\n'.join(''.join(line) for line in cells)


class Pty:
    """One real PTY-hosted alfredo-tui process with a reconstructed screen."""

    def __init__(self, test, args, cwd, env):
        self.test = test
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        self.output = bytearray()
        self.process = subprocess.Popen(args, stdin=self.slave, stdout=self.slave, stderr=self.slave, env=env, cwd=cwd)

    def screen(self):
        return visible_screen(self.output)

    def wait_for(self, text, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in self.screen():
                return
            if select.select([self.master], [], [], 0.05)[0]:
                chunk = os.read(self.master, 65536)
                self.output.extend(chunk)
                if b'\x1b[6n' in chunk:
                    os.write(self.master, b'\x1b[1;1R')
            self.test.assertIsNone(self.process.poll(), self.output.decode(errors='replace'))
        self.test.fail(f'Missing {text!r}:\n{self.screen()}')

    def write(self, data):
        os.write(self.master, data)

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=3)
        os.close(self.master)
        os.close(self.slave)


class ZeroCeremonyStart(unittest.TestCase):
    def test_repository_subdirectory_opens_default_mission_and_locked_start_falls_back(self):
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        with tempfile.TemporaryDirectory(prefix='alfredo-zero-state-') as state, tempfile.TemporaryDirectory(prefix='alfredo-zero-repo-') as repo:
            root = os.path.realpath(repo)
            subprocess.run(['git', '-C', root, 'init', '-q', '--template='], check=True)
            nested = Path(root, 'src', 'deep')
            nested.mkdir(parents=True)
            env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=state)
            args = [str(binary), '--model', 'fixture', '--endpoint', 'http://127.0.0.1:9']
            first = Pty(self, args, nested, env)
            second = None
            try:
                # A1: no flags, no typed input, from a subdirectory.
                first.wait_for('ALFREDO  default · ')  # header: mission · repository directory
                first.wait_for('◈ ○ chat 1')  # side pane chat row
                self.assertNotIn('Open your work', first.screen())
                first.wait_for(root)  # arrival line names the repository root
                manifests = [json.loads(path.read_text()) for path in Path(state).rglob('mission.json')]
                self.assertEqual([manifest['mission'] for manifest in manifests], ['default'])
                # The conversation is owned by the first terminal: fall back with the reason shown.
                second = Pty(self, args, nested, env)
                second.wait_for('Workspace selection required')
                second.wait_for('Automatic open failed')
                second.wait_for('in use')
                # Enter validates the placeholder (the subdirectory); the refusal sits on the line above the input.
                second.write(b'\r')
                second.wait_for('exact repository root')
                lines = second.screen().split('\n')
                notice = next(index for index, line in enumerate(lines) if 'exact repository root' in line)
                self.assertTrue(lines[notice + 1].startswith('┌'), second.screen())
                second.write(root.encode() + b'\r')
                second.wait_for('Mission selection required')
                second.wait_for('Enter open or create')
                # Typing replaces the default placeholder; Enter creates the missing mission.
                second.write(b'demo1\r')
                second.wait_for('ALFREDO  demo1 · ')
                self.assertNotIn('defaultdemo1', second.screen())
                names = sorted(json.loads(path.read_text())['mission'] for path in Path(state).rglob('mission.json'))
                self.assertEqual(names, ['default', 'demo1'])
                for terminal in (first, second):
                    terminal.write(b'\x11')
                    self.assertEqual(terminal.process.wait(timeout=5), 0)
            finally:
                first.close()
                if second is not None:
                    second.close()


class ScreenReconstruction(unittest.TestCase):
    def test_alternate_screen_does_not_match_stale_primary_text(self):
        entered = b'old text\x1b[?1049hnew text'
        self.assertNotIn('old text', visible_screen(entered))
        self.assertIn('new text', visible_screen(entered))
        left = entered + b'\x1b[?1049l'
        self.assertIn('old text', visible_screen(left))
        self.assertNotIn('new text', visible_screen(left))


class ChatHarnessContext(unittest.TestCase):
    def test_plain_chat_turn_carries_alfredo_harness_context(self):
        # Regression: plain chat sent only the user text, so the model denied any harness work.
        chats = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                body = json.dumps({'models': [{'name': 'fixture'}]}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                if self.path == '/api/chat':
                    chats.append(request)
                body = (b'{"done":true,"done_reason":"load"}\n' if self.path == '/api/generate'
                        else b'{"message":{"content":"CONTEXT_REPLY"},"done":true}\n')
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        with tempfile.TemporaryDirectory(prefix='alfredo-chat-state-') as state, tempfile.TemporaryDirectory(prefix='alfredo-chat-repo-') as repo:
            git = ['git', '-C', repo, '-c', 'user.name=t', '-c', 'user.email=t@t']
            subprocess.run(git + ['init', '-q', '--template='], check=True)
            subprocess.run(git + ['commit', '-q', '--allow-empty', '-m', 'init'], check=True)
            env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=state)
            terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'], repo, env)
            try:
                terminal.wait_for('◈ ○ chat 1')
                terminal.write(b'what did you remove?\r')
                terminal.wait_for('CONTEXT_REPLY')
                request = next(chat for chat in chats if chat['messages'][-1]['content'] == 'what did you remove?')
                self.assertEqual(request['messages'][0]['role'], 'system')
                self.assertIn('Alfredo', request['messages'][0]['content'])
                self.assertIn('No tasks in this mission yet.', request['messages'][0]['content'])
                terminal.write(b'\x11')
                self.assertEqual(terminal.process.wait(timeout=5), 0)
            finally:
                terminal.close()
                server.shutdown()


class ModelPicker(unittest.TestCase):
    def test_arrow_keys_choose_a_model_from_the_list(self):
        # Regression: /models listed models, but arrows fell through to prompt history.
        chats = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                body = json.dumps({'models': [{'name': 'fixture'}, {'name': 'second-model'}]}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                if self.path == '/api/chat':
                    chats.append(request)
                body = (b'{"done":true,"done_reason":"load"}\n' if self.path == '/api/generate'
                        else b'{"message":{"content":"PICKED_REPLY"},"done":true}\n')
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        with tempfile.TemporaryDirectory(prefix='alfredo-models-state-') as state, tempfile.TemporaryDirectory(prefix='alfredo-models-repo-') as repo:
            git = ['git', '-C', repo, '-c', 'user.name=t', '-c', 'user.email=t@t']
            subprocess.run(git + ['init', '-q', '--template='], check=True)
            subprocess.run(git + ['commit', '-q', '--allow-empty', '-m', 'init'], check=True)
            env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=state)
            terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'], repo, env)
            try:
                terminal.wait_for('◈ ○ chat 1')
                terminal.write(b'/models\r')
                terminal.wait_for('▸ › fixture')
                terminal.write(b'\x1b[B')
                terminal.wait_for('▸   second-model')
                terminal.write(b'\r')
                terminal.wait_for('Conversation model: second-model')
                terminal.write(b'hello\r')
                terminal.wait_for('PICKED_REPLY')
                self.assertEqual(chats[-1]['model'], 'second-model')
                terminal.write(b'\x11')
                self.assertEqual(terminal.process.wait(timeout=5), 0)
            finally:
                terminal.close()
                server.shutdown()


class TerminalSmoke(unittest.TestCase):
    def test_stalled_model_does_not_block_other_session_or_terminal_restore(self):
        slow_started = threading.Event()
        release_slow = threading.Event()
        release_reading = threading.Event()
        finish_reading = threading.Event()
        worker_models = []
        model_requests = []
        parallel_workers = threading.Barrier(2)
        release_parallel = threading.Event()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps({'models': [{'name': 'fixture'}, {'name': 'second-model'}]}).encode())

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
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                model_requests.append(request)
                if 'format' in request:
                    assert request['think'] is False
                elif request['messages'][-1]['content'].startswith('Implement this task:'):
                    # Default FILE-block workers keep the thinking policy without a schema.
                    assert request['think'] is False
                    assert '=== END FILE ===' in request['messages'][-1]['content']
                else:
                    assert 'think' not in request
                if request['messages'][-1]['content'].startswith('Implement this task:'):
                    worker_models.append(request['model'])
                    if 'AUTO_' in request['messages'][-1]['content'].split('\n', 1)[0]:
                        parallel_workers.wait(timeout=8)
                        release_parallel.wait(10)
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.end_headers()
                if request['messages'][-1]['content'] == 'limit':
                    self.wfile.write(b'{"message":{"content":"LIMIT_PARTIAL"},"done":true,"done_reason":"length"}\n')
                    self.wfile.flush()
                    return
                if request['messages'][-1]['content'] == 'reading':
                    def send_reading(text, done=False):
                        self.wfile.write((json.dumps({'message': {'content': text}, 'done': done}) + '\n').encode())
                        self.wfile.flush()
                    send_reading(''.join(f'READ_HISTORY_{n:03}\n' for n in range(60)))
                    release_reading.wait(10)
                    send_reading(''.join(f'READ_MORE_{n:03}\n' for n in range(40)))
                    finish_reading.wait(10)
                    send_reading('READING_DONE', True)
                    return
                if request['messages'][-1]['content'] == 'slow':
                    self.wfile.write(b'{"message":{"thinking":"PRIVATE_THINKING_SENTINEL"}}\n')
                    self.wfile.flush()
                    slow_started.set()
                    release_slow.wait(10)
                    return
                if request['messages'][0]['content'].startswith('Act as Frontier Architect'):
                    context = json.loads(request['messages'][1]['content'].split('\n', 1)[1])
                    architecture = 'escalated architecture revision' in request['messages'][0]['content']
                    expected = 'VALUE = 42\n' if architecture else 'VALUE = 0\n'
                    assert any(source['path'] == 'answer.py' and source['content'] == expected for source in context['sources'])
                    policy = {'files': ['answer.py'], 'check': ['/usr/bin/python3', '-m', 'unittest']}
                    content = json.dumps({'tasks': [
                        {'title': 'Planned calculation', 'model': request['model'], 'dependencies': [], 'policy': policy, 'acceptance': ['Calculation returns 42']},
                        {'title': 'Revised integration' if 'Revision request:' in request['messages'][-1]['content'] else 'Planned integration', 'model': request['model'], 'dependencies': [1], 'policy': policy, 'acceptance': ['Calculation returns 42']},
                    ]})
                    if architecture:
                        assert any('Failure: architecture' in message['content'] for message in request['messages'])
                        content = json.dumps({'tasks': [{'title': 'Architect corrected calculation', 'model': request['model'], 'dependencies': [], 'policy': {'files': ['answer.py'], 'check': ['/usr/bin/python3', '-B', '-c', 'from answer import VALUE; assert VALUE == 42']}, 'acceptance': ['Revised calculation returns 42']}]})
                elif request['messages'][-1]['content'].startswith('Implement this task: Dependent_check'):
                    content = json.dumps({'files': [{'path': 'child.txt', 'content': 'ready'}]})
                elif request['messages'][-1]['content'].startswith('Implement this task:'):
                    if 'Implement this task: Planned calculation' in request['messages'][-1]['content']:
                        assert 'RECORDED ACCEPTANCE CRITERIA' in request['messages'][-1]['content']
                        assert 'Calculation returns 42' in request['messages'][-1]['content']
                    # FILE-block answer; other fixtures keep legacy JSON answers.
                    content = 'Updating answer.py.\n=== FILE: answer.py ===\nVALUE = 42\n=== END FILE ===\n'
                else:
                    content = 'FAST_REPLY'
                self.wfile.write((json.dumps({'message': {'content': content}, 'done': True, 'load_duration': 500000000, 'eval_duration': 2000000000, 'eval_count': 40}) + '\n').encode())
                self.wfile.flush()

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        master, slave = pty.openpty()
        original = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        state = tempfile.TemporaryDirectory(prefix='alfredo-pty-state-')
        workspace = tempfile.TemporaryDirectory(prefix='alfredo-pty-workspace-')
        for args in [['init', '-q'], ['config', 'user.name', 'Fixture'], ['config', 'user.email', 'fixture@example.invalid']]:
            subprocess.run(['git', '-C', workspace.name, *args], check=True)
        Path(workspace.name, '.gitattributes').write_text('* text eol=lf\n')
        Path(workspace.name, 'answer.py').write_text('VALUE = 0\n')
        subprocess.run(['git', '-C', workspace.name, 'add', '.'], check=True)
        subprocess.run(['git', '-C', workspace.name, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fixture'], check=True)
        env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=state.name)
        for mode in ['auto', 'on', 'off']:
            result = subprocess.run([str(binary), '--structured-thinking', mode, '--help'], capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
        for tail in [[], ['invalid']]:
            result = subprocess.run([str(binary), '--structured-thinking', *tail], capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('--structured-thinking needs auto, on or off', result.stderr)
        preflight = subprocess.run(
            [str(binary), '--doctor', '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
            capture_output=True, text=True, env=env, cwd=workspace.name, timeout=15,
        )
        self.assertEqual(preflight.returncode, 0, preflight.stdout + preflight.stderr)
        self.assertIn('PASS model catalog: fixture is installed', preflight.stdout)
        process = subprocess.Popen(  # --select: inside a repository the selector is opt-in.
            [str(binary), '--select', '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
            stdin=slave, stdout=slave, stderr=slave, env=env, cwd=workspace.name,
        )
        output = bytearray()

        def wait_for(text, timeout=30):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if text.decode() in visible_screen(output):
                    return
                if select.select([master], [], [], 0.05)[0]:
                    chunk = os.read(master, 65536)
                    output.extend(chunk)
                    if b'\x1b[6n' in chunk:
                        os.write(master, b'\x1b[1;1R')
                self.assertIsNone(process.poll(), output.decode(errors='replace'))
            stores = list(Path(state.name).glob('rust-tasks-v1/*/tasks.json'))
            tasks = [(task['id'], task['status'], (task.get('run') or {}).get('detail')) for task in json.loads(stores[0].read_text())['tasks']] if stores else []
            failed_checks = [json.loads(path.read_text()).get('check') for path in Path(state.name).rglob('evidence.json') if json.loads(path.read_text()).get('status') == 'failed']
            self.fail(f'Missing {text!r}:\n{visible_screen(output)}\nSaved task outcomes: {tasks}\nFailed checks: {failed_checks}')

        def wait_for_header_without(text, timeout=30):
            # Header row 1 names dispatch only while it is on.
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                header = visible_screen(output).split('\n')[0]
                if 'ALFREDO' in header and text.decode() not in header:
                    return
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
                self.assertIsNone(process.poll(), output.decode(errors='replace'))
            self.fail(f'Header still shows {text!r}:\n{visible_screen(output)}')

        try:
            wait_for(b'Workspace selection required')
            self.assertFalse(list(Path(state.name).rglob('conversations-*.json')))
            os.write(master, b'\r')
            wait_for(b'Mission selection required')
            self.assertFalse(list(Path(state.name).rglob('conversations-*.json')))
            self.assertFalse(slow_started.is_set())
            self.assertFalse(list(Path(state.name).rglob('mission.json')))
            os.write(master, b'\r')  # Enter on the placeholder creates the missing default mission.
            wait_for('◈ ○ chat 1'.encode())
            os.write(master, b'/scope\r')
            wait_for(b'Outside flow')
            os.write(master, b'/chat\r@unknown request\r')
            wait_for(b'Unknown capability')
            self.assertEqual(len(model_requests), 0)
            os.write(master, b'\x15@way\t')
            wait_for(b'Enter fills draft')
            wait_for(b'@wayfinder')
            os.write(master, b'\x1b')
            dismissed_by = time.monotonic() + 3
            while ' Complete ' in visible_screen(output):
                self.assertLess(time.monotonic(), dismissed_by, 'Escape did not dismiss completion')
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
                self.assertIsNone(process.poll())
            self.assertIn('@way', visible_screen(output))
            self.assertEqual(len(model_requests), 0)
            os.write(master, b'\t\r')  # Complete the name only; no turn submitted.
            wait_for(b'@wayfinder')
            self.assertFalse(list(Path(state.name).rglob('tasks.json')))
            self.assertEqual(len(model_requests), 0)
            # Scope mutations must survive the same real failed-save barrier as
            # task commands. Ctrl+R retries the captured turn and exact request.
            deadline = time.monotonic() + 5
            while not list(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json')):
                self.assertLess(time.monotonic(), deadline, 'Initial conversation was not saved')
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
            scope_intent_path = next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json'))
            scope_intent_bytes = scope_intent_path.read_bytes()
            scope_intent_path.unlink()
            scope_intent_path.mkdir()
            try:
                os.write(master, b'inspect project scope\r')
                wait_for(b'Intent save failed')
                self.assertFalse(list(Path(state.name).rglob('understanding.json')))
                self.assertEqual(len(model_requests), 0)
            finally:
                scope_intent_path.rmdir()
                scope_intent_path.write_bytes(scope_intent_bytes)
            os.write(master, b'\x12')
            wait_for(b'Wayfinder / Chart')
            wait_for(b'Provide four lines')
            self.assertFalse(list(Path(state.name).rglob('tasks.json')))
            self.assertEqual(len(model_requests), 0)
            os.write(master, b'/task blocked by scope\r')
            wait_for(b'Shared Understanding pending')
            wait_for(b'before saving plans')  # Wait for this command's refusal, not the pre-existing header.
            self.assertFalse(list(Path(state.name).rglob('tasks.json')))
            os.write(master, b'/chat\r')
            brief = '@wayfinder Destination: Build the requested change\nScope: Fixture repository only\nConstraints: Keep the original workspace unchanged\nUncertainty: Local model quality not qualified'
            os.write(master, b'\x1b[200~' + brief.encode() + b'\x1b[201~\r')
            wait_for(b'confirm shared understanding 2')
            os.write(master, b'/scope\r')
            wait_for(b'Known uncertainty')
            os.write(master, b'/chat\r@wayfinder confirm shared understanding 2\r')
            wait_for(b'turn complete')
            wait_for('Wayfinder · scope receipt 3'.encode())
            self.assertFalse(list(Path(state.name).rglob('tasks.json')))
            self.assertEqual(len(model_requests), 0)
            understanding_path = next(Path(state.name).rglob('understanding.json'))
            understanding = json.loads(understanding_path.read_text())
            self.assertEqual(understanding['schema_version'], 2)
            self.assertEqual(understanding['flow']['mode'], 'chart')
            self.assertTrue(understanding['confirmed'])
            os.write(master, b'slow\r')
            self.assertTrue(slow_started.wait(5), 'Model request did not start')
            scope_reference = model_requests[-1]['messages'][0]
            self.assertEqual(scope_reference['role'], 'system')
            captured_scope = json.loads(scope_reference['content'].split('Scope reference: ', 1)[1])
            self.assertEqual(captured_scope['revision'], 3)
            self.assertTrue(captured_scope['confirmed'])
            self.assertEqual(captured_scope['brief']['scope'], 'Fixture repository only')
            wait_for(b' thinking')
            self.assertNotIn('PRIVATE_THINKING_SENTINEL', visible_screen(output))
            wait_for('thinking · 1.'.encode())  # Clock advances while no model output arrives.
            os.write(master, b'/workspace\r')
            wait_for(b'Finish or cancel active conversations')
            self.assertNotIn('Open your work', visible_screen(output))
            os.write(master, b'\x15')
            os.write(master, b'\x0e/mod\t\r')  # Completion fills /models, without submitting.
            wait_for(b'/models')
            os.write(master, b'\r')
            wait_for(b'Installed models')
            wait_for(b'second-model')
            os.write(master, b'/model sec\t\r')
            wait_for(b'/model second-model')
            self.assertNotIn('Conversation model: second-model', visible_screen(output))
            os.write(master, b'\r')
            wait_for(b'Conversation model: second-model')
            os.write(master, b'fst\x1b[D\x1b[Da\x1b[F\r')  # New session; insert a into fst using Left, then End.
            wait_for(b'FAST_REPLY')
            wait_for(b'tok/s')
            self.assertFalse(release_slow.is_set())
            os.write(master, b'limit\r')
            wait_for(b'4096-token limit')
            wait_for(b'LIMIT_PARTIAL')
            os.write(master, b'\t\x1b')  # Return to slow session and cancel.
            wait_for(b'cancelled')
            # Intent publication failure must prevent canonical task dispatch.
            intent_path = next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json'))
            intent_bytes = intent_path.read_bytes()
            intent_path.unlink()
            intent_path.mkdir()
            try:
                os.write(master, b'/task MUST_NOT_DISPATCH\r')
                wait_for(b'Intent save failed')
                for task_path in Path(state.name).glob('rust-tasks-v1/*/tasks.json'):
                    self.assertEqual(json.loads(task_path.read_text())['tasks'], [])
            finally:
                intent_path.rmdir()
                intent_path.write_bytes(intent_bytes)
            os.write(master, b'/task Persisted_fix\r')
            wait_for(b'Persisted_fix')
            wait_for(b'Task #1 saved')
            os.write(master, b'/approve 1\r')
            wait_for('Approved · fixture'.encode())  # detail status line
            saved = next(Path(state.name).glob('rust-tasks-v1/*/tasks.json'))
            record = json.loads(saved.read_text())
            self.assertEqual(record['tasks'][0]['status'], 'approved')
            self.assertEqual(record['revision'], 2)
            os.write(master, b'/chat\r')
            wait_for(b'Task #1 approved')
            os.write(master, b'/tasks\r')
            wait_for(b'Persisted_fix')
            os.write(master, b'\x11')  # Ctrl+Q
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(termios.tcgetattr(slave), original)
            conversations = json.loads(next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json')).read_text())
            self.assertEqual(len(conversations['sessions']), 2)
            commands = [command for session in conversations['sessions'] for command in session.get('commands', [])]
            wayfinder_commands = [command for command in conversations['sessions'][0]['commands']
                                  if command['intent']['kind'] == 'wayfinder']
            self.assertEqual(len(wayfinder_commands), 3)
            self.assertEqual(wayfinder_commands[0]['attempt'], 2)
            for command, receipt in zip(wayfinder_commands, understanding['receipts']):
                self.assertEqual(command['intent']['request'], receipt['request'])
                turn = command['intent']['user_message']
                self.assertEqual(command['after_messages'], turn + 2)
                self.assertEqual(conversations['sessions'][0]['messages'][turn]['role'], 'user')
                self.assertNotEqual(command['text'], conversations['sessions'][0]['messages'][turn]['content'])
            self.assertEqual(next(command for command in commands if command['text'] == '/task MUST_NOT_DISPATCH')['state']['kind'], 'refused')
            self.assertEqual(next(command for command in commands if command['text'] == '/task Persisted_fix')['intent']['request'], record['receipts'][0]['request'])
            observed = [ref for session in conversations['sessions'] for ref in session.get('task_receipts', [])]
            self.assertEqual(observed, [])  # Command acknowledgments stay at their origin, without duplicate observations.
            self.assertEqual(conversations['sessions'][1]['model'], 'second-model')
            self.assertEqual(conversations['sessions'][1]['messages'][1]['content'], 'FAST_REPLY')
            output.clear()
            process = subprocess.Popen(
                [str(binary), '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
                stdin=slave, stdout=slave, stderr=slave, env=env, cwd=state.name,
            )
            wait_for(b'ALFREDO')
            wait_for(b'Workspace selection required')
            wait_for(b'Tab fills a saved path')
            self.assertEqual(json.loads(saved.read_text())['revision'], 2)
            os.write(master, b'\t\r')
            wait_for(b'Mission selection required')
            wait_for(b'Tab fills a saved name')
            self.assertEqual(json.loads(saved.read_text())['revision'], 2)
            os.write(master, b'\x1bOQ\r')  # Start New cannot replace the existing default mission.
            wait_for(b'Mission already exists')
            self.assertEqual(json.loads(saved.read_text())['revision'], 2)
            os.write(master, b'\x1bOQ\t\r')  # Return to Resume, fill the saved name, then open it.
            wait_for(b'Conversations restored')
            wait_for(b'Persisted_fix')
            os.write(master, b'/chat\r\t')
            wait_for(b'FAST_REPLY')
            # Replay the original confirmed scope from another conversation:
            # no new receipt, model request, reply ownership or task approval.
            prior_requests = len(model_requests)
            os.write(master, f"/retry-command 1:{wayfinder_commands[-1]['sequence']}\r".encode())
            wait_for(b'Wayfinder scope receipt verified')
            self.assertEqual(json.loads(understanding_path.read_text()), understanding)
            self.assertEqual(len(model_requests), prior_requests)
            self.assertEqual(json.loads(saved.read_text()), record)
            # Explicit restored retry from the other session retains original intent.
            proposal_command = next(command for command in commands if command['text'] == '/task Persisted_fix')
            os.write(master, f"/retry-command 1:{proposal_command['sequence']}\r".encode())
            wait_for(b'Task #1 saved')
            self.assertEqual(json.loads(saved.read_text()), record)
            os.write(master, b'/chat\r')
            wait_for(b'FAST_REPLY')
            os.write(master, b'\t/tasks\r')
            wait_for(b'Persisted_fix')
            wait_for('Approved · fixture'.encode())  # detail status line
            self.assertEqual(json.loads(saved.read_text())['revision'], 2)
            os.write(master, b'/activity #1\r')
            wait_for(b'Saved task activity')
            wait_for(b'Task approved')
            self.assertEqual(json.loads(saved.read_text())['revision'], 2)
            policy = json.dumps({'files': ['answer.py'], 'check': ['/usr/bin/python3', '-B', '-c', 'from answer import VALUE; assert VALUE == 42']})
            os.write(master, ('/permit 1 ' + policy + '\r').encode())
            wait_for(b'Task #1 saved')
            wait_for(b'revision 3')
            os.write(master, b'/approve 1\r')
            wait_for(b'revision 4')
            os.write(master, b'/run 1\r')
            wait_for(b'needs review', timeout=15)
            os.write(master, b'/chat\r')
            wait_for(b'Task #1 started')
            wait_for(b'Task #1 check passed')
            os.write(master, b'/tasks\r')
            wait_for(b'Persisted_fix')
            wait_for(b'Task queue refreshed')
            self.assertEqual(Path(workspace.name, 'answer.py').read_text(), 'VALUE = 0\n')
            os.write(master, b'\x1bOR')  # F3 opens selected task evidence
            wait_for(b'Verified run evidence')
            wait_for(b'Requested generation: thinking off')
            wait_for(b'Check result')
            before_review_models = len(worker_models)
            decision = {'outcome': 'needs-repair', 'reason': 'Clarify the implementation', 'criteria': []}
            os.write(master, ('/review 1 ' + json.dumps(decision) + '\r').encode())
            wait_for(b'repair #2 proposed')
            self.assertEqual(len(worker_models), before_review_models)
            self.assertEqual(json.loads(saved.read_text())['receipts'][-1]['request']['action']['kind'], 'review-and-repair')
            repair = json.loads(saved.read_text())['tasks'][1]
            self.assertEqual(repair['repair_of'], 1)
            self.assertEqual(repair['status'], 'proposed')
            os.write(master, b'/approve 2\r')
            wait_for(b'revision 8')
            os.write(master, b'/run 2\r')
            wait_for('◐ #2'.encode(), timeout=15)
            repair_requests = [r for r in model_requests if r['messages'][-1]['content'].startswith('Implement this task: Repair #1:')]
            self.assertEqual(len(repair_requests), 1)
            self.assertEqual([m['role'] for m in repair_requests[0]['messages']], ['user', 'assistant', 'user'])
            recorded = {json.loads(p.read_text())['run']: json.loads(p.read_text()) for p in Path(state.name).rglob('evidence.json')}
            tasks = json.loads(saved.read_text())['tasks']
            parent_agent = recorded[tasks[0]['run']['id']]['agent']
            repair_agent = recorded[tasks[1]['run']['id']]['agent']
            self.assertEqual(repair_agent['agent'], parent_agent['agent'])
            self.assertEqual(repair_agent['continued_from'], tasks[0]['run']['id'])
            os.write(master, b'/evidence 2\r')
            wait_for(b'continued conversation')
            os.write(master, b'/tasks #1\r')
            wait_for(b'Task filter applied')
            os.write(master, b'/evidence 2\r')
            wait_for(b'Verified evidence for task #2')
            os.write(master, b'/accept\r')  # Must target the evidence being inspected.
            wait_for(b'revision 11')  # Wait for the review receipt, not evidence explanatory text.
            self.assertEqual(json.loads(saved.read_text())['tasks'][1]['status'], 'accepted')
            self.assertEqual(json.loads(saved.read_text())['tasks'][0]['status'], 'rejected')
            os.write(master, b'/activity #2\r')
            wait_for(b'Review accepted')
            os.write(master, b'/after 1 Dependent_check\r')
            wait_for(b'Task #3 saved')
            policy = json.dumps({'files': ['child.txt'], 'check': ['/usr/bin/python3', '-B', '-c', "from answer import VALUE; from pathlib import Path; assert VALUE == 42; assert Path('child.txt').read_text() == 'ready'"]})
            os.write(master, ('/permit 3 ' + policy + '\r').encode())
            wait_for(b'revision 13')
            os.write(master, b'/approve 3\r')
            wait_for(b'revision 14')
            os.write(master, b'/resolve-repair 2\r')
            wait_for(b'revision 15')
            os.write(master, b'/run 3\r')
            wait_for('◐ #3'.encode(), timeout=15)
            os.write(master, b'/evidence 3\r')
            wait_for(b'Accepted dependency inputs: #1 via repair #2')
            result = json.loads(saved.read_text())
            self.assertEqual(result['tasks'][2]['run']['inputs'][0]['task'], 1)
            self.assertEqual(result['tasks'][2]['run']['inputs'][0]['source_task'], 2)
            self.assertEqual(Path(workspace.name, 'answer.py').read_text(), 'VALUE = 0\n')
            self.assertFalse(Path(workspace.name, 'child.txt').exists())
            os.write(master, b'/accept\r')
            wait_for(b'Task #3 saved')
            wait_for(b'revision 18')
            os.write(master, b'/branch\r')
            wait_for(b'Review branch ready:')
            result = json.loads(saved.read_text())
            branch = result['receipts'][-1]['request']['action']
            self.assertEqual(branch['kind'], 'branch')
            self.assertEqual(branch['task'], 3)
            self.assertEqual(subprocess.check_output(['git', '-C', workspace.name, 'show', branch['name'] + ':child.txt'], text=True), 'ready')
            self.assertEqual(Path(workspace.name, 'answer.py').read_text(), 'VALUE = 0\n')
            os.write(master, b'/activity #3\r')
            wait_for(b'Review branch recorded')
            os.write(master, b'/plan Extend calculation with integration checks\r')
            wait_for(b'Review draft paths', timeout=10)
            self.assertEqual(len(json.loads(saved.read_text())['tasks']), 3)
            os.write(master, b'/plan-revise Add boundary coverage\r')
            wait_for(b'Review revised draft paths', timeout=10)
            self.assertEqual(len(json.loads(saved.read_text())['tasks']), 3)
            revision_request = model_requests[-1]
            self.assertTrue(any(m['content'].startswith('Previous unsaved task draft') for m in revision_request['messages']))
            request_count = len(model_requests)
            os.write(master, b'\x11')
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(termios.tcgetattr(slave), original)
            draft_snapshot = json.loads(next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json')).read_text())
            self.assertEqual(draft_snapshot['plan_draft']['plan']['tasks'][1]['title'], 'Revised integration')
            planner_commands = [command for session in draft_snapshot['sessions'] for command in session.get('commands', []) if command['intent']['kind'] == 'planner']
            self.assertEqual(len(planner_commands), 2)
            self.assertTrue(all(command['state']['kind'] == 'planner' for command in planner_commands))
            self.assertTrue(all(command['state']['outcome']['kind'] == 'generated' for command in planner_commands))
            self.assertEqual(draft_snapshot['plan_draft']['origin'], planner_commands[-1]['intent']['request'])

            output.clear()
            process = subprocess.Popen(
                [str(binary), '--workspace', workspace.name, '--mission', 'default', '--state-dir', state.name,
                 '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
                stdin=slave, stdout=slave, stderr=slave, env=env, cwd=state.name,
            )
            wait_for(b'Restored plan draft')
            self.assertEqual(len(model_requests), request_count)
            self.assertEqual(len(json.loads(saved.read_text())['tasks']), 3)
            os.write(master, b'/plan-save\r')
            wait_for(b'2 proposed tasks')
            proposed = json.loads(saved.read_text())
            self.assertEqual(len(proposed['tasks']), 5)
            self.assertEqual(proposed['tasks'][4]['title'], 'Revised integration')
            self.assertEqual(proposed['tasks'][4]['dependencies'], [4])
            self.assertEqual(proposed['receipts'][-1]['request']['action']['plan']['tasks'][0]['acceptance'], ['Calculation returns 42'])
            self.assertEqual(proposed['tasks'][3]['status'], 'proposed')
            self.assertIsNone(proposed['tasks'][3].get('run'))
            self.assertEqual(proposed['receipts'][-1]['request']['action']['kind'], 'plan')
            context = proposed['receipts'][-1]['request']['action']['plan']['context']
            self.assertEqual(context['baseline'], subprocess.check_output(['git', '-C', workspace.name, 'rev-parse', 'HEAD'], text=True).strip())
            self.assertTrue(any(source['path'] == 'answer.py' for source in context['sources']))
            replacement = 'fixture' if proposed['tasks'][3]['model'] != 'fixture' else 'second-model'
            os.write(master, b'/models\r')
            wait_for(b'Installed models')
            wait_for(b'second-model')
            os.write(master, b'/approve 4\r')
            wait_for(b'revision 21')
            os.write(master, ('/assign 4 ' + replacement[:2] + '\t\r').encode())
            wait_for(('/assign 4 ' + replacement).encode())
            self.assertEqual(json.loads(saved.read_text())['revision'], 21)
            os.write(master, b'\r')
            wait_for(b'Assignment recorded: task #4')
            reassigned = json.loads(saved.read_text())
            self.assertEqual(reassigned['tasks'][3]['model'], replacement)
            self.assertEqual(reassigned['tasks'][3]['status'], 'proposed')
            self.assertEqual(reassigned['receipts'][-1]['request']['action']['kind'], 'assign')
            policy = json.dumps({'files': ['answer.py'], 'check': ['/usr/bin/python3', '-B', '-c', 'from answer import VALUE; assert VALUE == 42']})
            os.write(master, ('/permit 4 ' + policy + '\r').encode())
            wait_for(b'revision 23')
            os.write(master, b'/approve 4\r')
            wait_for(b'revision 24')
            os.write(master, b'/run 4\r')
            wait_for('◐ #4'.encode(), timeout=15)
            self.assertEqual(worker_models[-1], replacement)
            self.assertEqual(json.loads(saved.read_text())['tasks'][3]['model'], replacement)
            self.assertEqual(Path(workspace.name, 'answer.py').read_text(), 'VALUE = 0\n')
            # Two ready workers must reach inference concurrently; the barrier rejects serial dispatch.
            revision = json.loads(saved.read_text())['revision']
            for task_id, title in [(6, 'AUTO_ONE'), (7, 'AUTO_TWO')]:
                os.write(master, ('/task ' + title + '\r').encode())
                revision += 1
                wait_for(('revision ' + str(revision)).encode())
                os.write(master, ('/permit ' + str(task_id) + ' ' + policy + '\r').encode())
                revision += 1
                wait_for(('revision ' + str(revision)).encode())
                os.write(master, ('/approve ' + str(task_id) + '\r').encode())
                revision += 1
                wait_for(('revision ' + str(revision)).encode())
            os.write(master, ('/permit 5 ' + policy + '\r').encode())
            revision += 1
            wait_for(('revision ' + str(revision)).encode())
            os.write(master, b'/approve 5\r')  # Its parent #4 is only ReviewReady.
            revision += 1
            wait_for(('revision ' + str(revision)).encode())
            os.write(master, b'/dispatch on\r')
            wait_for(b'   dispatch on')  # header attention while on
            os.write(master, b'/chat\r')
            wait_for(b'   2 running', timeout=15)
            release_parallel.set()
            wait_for(b'3 review', timeout=15)
            os.write(master, b'\x1bOQ')
            # Hierarchy rows can extend beyond the viewport; inspect both exact
            # results without changing the concurrent-dispatch authority checks.
            os.write(master, b'/tasks #6\r')
            wait_for('◐ #6'.encode(), timeout=15)
            os.write(master, b'/tasks #7\r')
            wait_for('◐ #7'.encode(), timeout=15)
            self.assertIsNone(json.loads(saved.read_text())['tasks'][4].get('run'))
            os.write(master, b'/evidence 4\r')
            wait_for(b'Acceptance criteria')
            os.write(master, b'/accept 4\r')
            wait_for(b'This task has acceptance criteria')
            self.assertEqual(json.loads(saved.read_text())['tasks'][3]['status'], 'review-ready')
            os.write(master, b'\x15')
            hold = {'outcome': 'rejected', 'risk': 'security', 'reason': 'Human inspection required',
                    'criteria': [{'criterion': 1, 'met': False, 'note': 'Awaiting human inspection'}]}
            os.write(master, ('/review 4 ' + json.dumps(hold) + '\r').encode())
            wait_for(b'escalated to human review')
            held = json.loads(saved.read_text())
            self.assertEqual(held['tasks'][3]['status'], 'needs-human-review')
            self.assertIsNone(held['tasks'][4].get('run'))
            decision = {'outcome': 'approved-with-limitations', 'reason': 'Reviewed calculation contract',
                        'criteria': [{'criterion': 1, 'met': True, 'note': 'Passing test asserts VALUE equals 42'}],
                        'limitations': ['Performance beyond fixture inputs is unmeasured']}
            os.write(master, ('/review 4 ' + json.dumps(decision) + '\r').encode())
            wait_for(b'Approved with limitations')
            wait_for(b'Limitation:')
            self.assertTrue(any(r['request']['action'].get('decision') == decision for r in json.loads(saved.read_text())['receipts']))
            wait_for('◐ #5'.encode(), timeout=15)
            os.write(master, b'/dispatch off\r')
            wait_for('✓ Dispatch off'.encode())
            wait_for_header_without(b'dispatch on')
            dispatched = json.loads(saved.read_text())
            self.assertEqual(dispatched['tasks'][4]['run']['inputs'][0]['task'], 4)
            for task_id in [5, 6, 7]:
                self.assertEqual(sum(receipt['request']['action']['kind'] == 'start' and receipt['task'] == task_id for receipt in dispatched['receipts']), 1)
            # Repeated architecture failures automatically invoke the original Architect.
            architecture_review = {'outcome': 'needs-repair', 'failure': 'architecture',
                                   'reason': 'Revise the calculation boundary',
                                   'criteria': [{'criterion': 1, 'met': False, 'note': 'Implementation boundary needs revision'}]}
            revision = json.loads(saved.read_text())['revision']
            os.write(master, ('/review 5 ' + json.dumps(architecture_review) + '\r').encode())
            revision += 1
            wait_for(('revision ' + str(revision)).encode())
            self.assertEqual(json.loads(saved.read_text())['tasks'][7]['repair_of'], 5)
            os.write(master, b'/approve 8\r')
            revision += 1
            wait_for(('revision ' + str(revision)).encode())
            os.write(master, b'/tasks #8\r')
            wait_for(b'Task filter applied')
            os.write(master, b'/run 8\r')
            wait_for('◐ #8'.encode(), timeout=15)
            requests_before_architect = len(model_requests)
            os.write(master, ('/review 8 ' + json.dumps(architecture_review) + '\r').encode())
            wait_for(b'Review draft paths', timeout=15)
            os.write(master, b'\x1b[6~')
            wait_for(b'Architect corrected calculation')
            self.assertEqual(len(model_requests), requests_before_architect + 1)
            self.assertEqual(len(json.loads(saved.read_text())['tasks']), 8)
            self.assertEqual(model_requests[-1]['model'], 'fixture')
            # The real terminal must persist the generated draft under the exact
            # automatic command before separate task adoption.
            conversation_path = next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json'))
            deadline = time.monotonic() + 5
            while True:
                draft_snapshot = json.loads(conversation_path.read_text())
                draft = draft_snapshot.get('plan_draft')
                if draft and draft['plan'].get('architecture', {}).get('task') == 8:
                    break
                self.assertLess(time.monotonic(), deadline, 'Architect draft was not saved')
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
            generated = next(command for session in draft_snapshot['sessions']
                             for command in session.get('commands', [])
                             if command['intent']['kind'] == 'architect-draft')
            self.assertEqual(draft['origin'], generated['intent']['request']['request'])
            os.write(master, b'/plan-save\r')
            wait_for(b'Plan saved')
            adopted = json.loads(saved.read_text())
            self.assertEqual(adopted['tasks'][8]['repair_of'], 8)
            self.assertEqual(adopted['tasks'][8]['status'], 'proposed')
            self.assertEqual(adopted['tasks'][8]['dependencies'], [4])
            self.assertEqual(adopted['receipts'][-1]['request']['action']['plan']['architecture']['task'], 8)
            self.assertIsNone(adopted['tasks'][8].get('run'))
            os.write(master, b'/approve 9\r')
            wait_for(('revision ' + str(adopted['revision'] + 1)).encode())
            os.write(master, b'/tasks #9\r')
            wait_for(b'Task filter applied')
            os.write(master, b'/run 9\r')
            wait_for('◐ #9'.encode(), timeout=15)
            revised = json.loads(saved.read_text())
            self.assertEqual(revised['tasks'][8]['run']['baseline'], revised['tasks'][7]['run']['baseline'])
            self.assertEqual(revised['tasks'][8]['run']['inputs'], revised['tasks'][7]['run']['inputs'])
            # Real tree disclosure keys must preserve canonical state and never
            # leave an invisible previous task as a shorthand action target.
            tree_bytes = saved.read_bytes()
            tree_requests = len(model_requests)
            os.write(master, b'/tasks #1\r')
            wait_for(b'Task filter applied')
            os.write(master, b'/tasks\r')
            wait_for('├ work'.encode())
            os.write(master, b'\x1b[1;3D' * 3)  # Collapse repairs, focus group, collapse group.
            wait_for('┌ Group '.encode())  # group detail in the right pane
            os.write(master, b'/evidence\r')
            wait_for(b'Select a task first')
            self.assertEqual(saved.read_bytes(), tree_bytes)
            self.assertEqual(len(model_requests), tree_requests)
            os.write(master, b'\x15')  # Refused commands retain their editable draft.
            os.write(master, b'\x1b[1;3C\x1b[B')  # Expand, then explicitly select a task.
            os.write(master, b'/tasks #9\r')
            wait_for('Task #9 · awaiting review'.encode())
            self.assertEqual(saved.read_bytes(), tree_bytes)
            self.assertEqual(len(model_requests), tree_requests)
            # Read old output while a real HTTP stream continues, then resume its tail.
            os.write(master, b'\x0ereading\r')
            wait_for(b'READ_HISTORY_059')
            os.write(master, b'\x1b[5~\x1b[5~')
            wait_for(b'READ_HISTORY_030')
            anchor = re.search(r'READ_HISTORY_\d{3}', visible_screen(output)).group()
            release_reading.set()
            deadline = time.monotonic() + 5
            while True:
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
                files = list(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json'))
                if files and 'READ_MORE_039' in files[0].read_text():
                    break
                self.assertLess(time.monotonic(), deadline, 'Stream did not reach the saved UI state')
            os.write(master, b'\x1bOS')  # F4 opens receipt-backed Activity from chat.
            wait_for(b'Saved task activity')
            os.write(master, b'\x1bOQ')
            wait_for(anchor.encode())
            self.assertEqual(re.search(r'READ_HISTORY_\d{3}', visible_screen(output)).group(), anchor)
            os.write(master, b'\x1b[6~' * 6)
            wait_for(b'READ_MORE_039')
            finish_reading.set()
            wait_for(b'READING_DONE')
            os.write(master, b'\x1b[5~')
            wait_for(b'READ_MORE_020')
            restart_anchor = re.search(r'READ_MORE_\d{3}', visible_screen(output)).group()
            os.write(master, b'\t/tasks #7\r')  # Restore the original selected conversation.
            wait_for('Task #7 · awaiting review'.encode())
            os.write(master, b'\x11')
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(termios.tcgetattr(slave), original)
            conversation_path = next(Path(state.name).glob('rust-tasks-v1/*/conversations-*.json'))
            preferences = json.loads(conversation_path.read_text())
            self.assertIn('Model output hit the 4096-token limit', preferences['sessions'][1]['status']['Failed'])
            self.assertEqual(preferences['sessions'][1]['messages'][-1]['content'], 'LIMIT_PARTIAL')
            sources = preferences['sessions'][0]['sources']
            self.assertEqual(sources['1']['kind'], 'wayfinder')
            self.assertEqual(sources['1']['receipt']['revision'], 1)
            self.assertEqual(sources['5']['receipt']['revision'], 3)
            self.assertEqual(sources['7'], {'kind': 'model', 'model': 'fixture'})
            self.assertEqual(preferences['schema_version'], 17)
            controls = [command for session in preferences['sessions'] for command in session.get('commands', []) if command['intent']['kind'] == 'control']
            self.assertEqual([command['intent']['request']['operation']['enabled'] for command in controls], [True, False])
            self.assertTrue(all(command['state']['kind'] == 'control' for command in controls))
            self.assertEqual([command['state']['outcome']['enabled'] for command in controls], [True, False])
            self.assertEqual(controls[0]['intent']['request']['controller'], controls[1]['intent']['request']['controller'])
            automatic = [(session, command) for session in preferences['sessions'] for command in session.get('commands', []) if command['intent']['kind'] == 'dispatch-run']
            self.assertEqual(sorted(command['intent']['request']['task'] for _, command in automatic), [5, 6, 7])
            for session, command in automatic:
                launch = command['intent']['request']
                self.assertEqual(launch['source'], controls[0]['intent']['request'])
                parent = next(item for item in session['commands'] if item['id'] == controls[0]['id'])
                self.assertLess(parent['sequence'], command['sequence'])
                claim = revised['receipts'][launch['expected_revision']]
                self.assertEqual(claim['request']['correlation'], launch['correlation'])
                self.assertEqual(claim['request']['action']['kind'], 'start')
                approval = revised['receipts'][launch['approval_revision'] - 1]
                self.assertEqual(approval['request']['action'], {'kind': 'approve', 'task': launch['task']})
                self.assertFalse(any(message['content'] == command['text'] for message in session['messages']))
            architects = [(session, command) for session in preferences['sessions']
                          for command in session.get('commands', [])
                          if command['intent']['kind'] == 'architect-draft']
            self.assertEqual(len(architects), 1)
            session, command = architects[0]
            architect = command['intent']['request']
            parent = next(item for item in session['commands']
                          if item['intent'] == {'kind': 'task', 'request': architect['source']})
            self.assertLess(parent['sequence'], command['sequence'])
            review = revised['receipts'][architect['source']['expected_revision']]
            self.assertEqual(review['request'], architect['source'])
            operation = architect['request']['operation']
            self.assertEqual(operation['kind'], 'architect')
            self.assertEqual(operation['origin']['task'], 8)
            self.assertEqual(operation['origin']['review_revision'], review['revision'])
            self.assertEqual(command['state']['kind'], 'planner')
            self.assertEqual(command['state']['outcome']['kind'], 'generated')
            self.assertEqual(command['state']['outcome']['tasks'], 1)
            self.assertFalse(any(message['content'] == command['text'] for message in session['messages']))
            explicit_finish = next(receipt for receipt in revised['receipts'] if receipt['request']['action']['kind'] == 'finish' and receipt['task'] == 1)
            self.assertFalse(any(reference['revision'] == explicit_finish['revision'] for session in preferences['sessions'] for reference in session.get('task_receipts', [])))
            reading_session = next(session for session in preferences['sessions'] if any(message['content'] == 'reading' for message in session['messages']))
            self.assertEqual(reading_session['reading']['block_anchor']['key'], {'kind': 'message', 'id': 1})
            self.assertEqual(preferences['task_view'], {'visible': True, 'selected': 7, 'query': '#7'})
            output.clear()
            process = subprocess.Popen(
                [str(binary), '--workspace', workspace.name, '--mission', 'default', '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
                stdin=slave, stdout=slave, stderr=slave, env=env, cwd=workspace.name,
            )
            wait_for(b'ALFREDO')
            wait_for(b'AUTO_TWO')
            wait_for(b'#7')
            wait_for_header_without(b'dispatch on')
            self.assertEqual(json.loads(saved.read_text()), revised)
            os.write(master, b'\x1bOQ\x1b[Z')  # Chat, then previous (reading) session.
            wait_for(restart_anchor.encode())
            self.assertEqual(re.search(r'READ_MORE_\d{3}', visible_screen(output)).group(), restart_anchor)
            os.write(master, b'\t/tasks #7\r')
            wait_for(b'AUTO_TWO')
            # Switching is an in-process handoff, with cancellation and distinct stores.
            original_pid = process.pid
            selection_journal = Path(state.name, 'rust-selection-v1', 'selections.json')
            before_cancel = selection_journal.read_bytes()
            os.write(master, b'/workspace\r')
            wait_for(b'Workspace selection required')
            os.write(master, b'\x1b')
            wait_for(b'Workspace selection cancelled')
            wait_for(b'AUTO_TWO')
            self.assertEqual(selection_journal.read_bytes(), before_cancel)
            with tempfile.TemporaryDirectory(prefix='alfredo-switch-workspace-') as other:
                subprocess.run(['git', '-C', other, 'init', '-q', '--template='], check=True)
                os.write(master, b'\x15/workspace\r')
                wait_for(b'Workspace selection required')
                os.write(master, b'\x15' + other.encode() + b'\r')
                wait_for(b'Mission selection required')
                os.write(master, b'\x1bOQ\x15side-mission\r')
                wait_for('ALFREDO  side-mission · '.encode())
                wait_for('◈ ○ chat 1'.encode())
                self.assertNotIn('AUTO_TWO', visible_screen(output))
                self.assertNotIn('FAST_REPLY', visible_screen(output))
                os.write(master, b'\x0eSIDE_DRAFT\t/workspace\r')
                wait_for(b'Workspace selection required')
                os.write(master, b'\x15' + workspace.name.encode() + b'\r')
                wait_for(b'Mission selection required')
                os.write(master, b'\r')  # Resume default in the original repository.
                wait_for('ALFREDO  default · '.encode())
                wait_for(b'AUTO_TWO')
                wait_for(b'#7')
                self.assertEqual(process.pid, original_pid)
                self.assertNotIn('SIDE_DRAFT', visible_screen(output))
                self.assertEqual(json.loads(saved.read_text()), revised)
                snapshots = [json.loads(path.read_text()) for path in Path(state.name).glob('rust-tasks-v1/*/conversations-*.json')]
                self.assertEqual(sum('SIDE_DRAFT' in json.dumps(snapshot) for snapshot in snapshots), 1)
                # A target loading failure retains the current workstation and owner.
                side_manifest = next(path for path in Path(state.name).glob('rust-tasks-v1/*/mission.json')
                                     if json.loads(path.read_text())['mission'] == 'side-mission')
                side_history = next(side_manifest.parent.glob('conversations-*.json'))
                retained_side_history = side_history.read_bytes()
                side_history.write_bytes(b'{invalid target history')
                os.write(master, b'/workspace\r')
                wait_for(b'Workspace selection required')
                os.write(master, b'\x15' + other.encode() + b'\r')
                wait_for(b'Mission selection required')
                os.write(master, b'\x15side-mission\r')
                wait_for(b'Could not switch work')
                wait_for('ALFREDO  default · '.encode())
                wait_for(b'AUTO_TWO')
                self.assertEqual(process.pid, original_pid)
                self.assertEqual(side_history.read_bytes(), b'{invalid target history')
                failed_handoff = json.loads(selection_journal.read_text())['records'][-1]
                self.assertEqual(failed_handoff['outcome']['phase'], 'mission-ready')
                self.assertIn('Malformed conversation state', failed_handoff['outcome']['failure'])
                self.assertEqual(failed_handoff['request']['origin']['workspace'], workspace.name)
                self.assertEqual(failed_handoff['request']['choice']['workspace']['path'], other)
                self.assertTrue(failed_handoff['dispatched'])
                side_history.write_bytes(retained_side_history)
                self.assertEqual(json.loads(saved.read_text()), revised)
            os.write(master, b'\x11')
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(termios.tcgetattr(slave), original)
            # Exercise actual new-repository creation through the selector as well.
            with tempfile.TemporaryDirectory(prefix='alfredo-new-repository-') as parent, tempfile.TemporaryDirectory(prefix='alfredo-new-state-') as new_state:
                created = Path(parent, 'new project')
                output.clear()
                new_env = dict(env, ALFREDO_STATE_DIR=new_state)
                process = subprocess.Popen(
                    [str(binary), '--new-mission', 'fresh', '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
                    stdin=slave, stdout=slave, stderr=slave, env=new_env, cwd=workspace.name,
                )
                wait_for(b'Workspace selection required')
                os.write(master, b'\x1bOQ\r')  # F2 switches to Create; existing target must refuse.
                wait_for(b'already exists')
                wait_for(b'empty initial commit')
                self.assertFalse(list(Path(new_state).rglob('conversations-*.json')))
                os.write(master, b'\x15' + str(created).encode() + b'\r')
                wait_for(b'Mission selection required')
                wait_for(b'Nothing created yet')
                self.assertFalse(created.exists())
                self.assertFalse(list(Path(new_state).rglob('conversations-*.json')))
                self.assertEqual(list(Path(new_state).iterdir()), [])
                # Cancelling after path validation must leave no creation or selection history.
                os.write(master, b'\x1b')
                self.assertEqual(process.wait(timeout=3), 0)
                self.assertFalse(created.exists())
                self.assertEqual(list(Path(new_state).iterdir()), [])
                output.clear()
                process = subprocess.Popen(
                    [str(binary), '--new-mission', 'fresh', '--model', 'fixture', '--endpoint', f'http://127.0.0.1:{server.server_port}'],
                    stdin=slave, stdout=slave, stderr=slave, env=new_env, cwd=workspace.name,
                )
                wait_for(b'Workspace selection required')
                os.write(master, b'\x1bOQ\x15' + str(created).encode() + b'\r')
                wait_for(b'Mission selection required')
                self.assertFalse(created.exists())
                os.write(master, b'\r')  # Admit exact repository + mission request before creation.
                wait_for('◈ ○ chat 1'.encode())
                self.assertTrue(created.joinpath('.git').is_dir())
                self.assertEqual(subprocess.run(['git', '-C', str(created), 'rev-parse', '--verify', 'HEAD'], capture_output=True).returncode, 0)
                self.assertEqual(subprocess.check_output(['git', '-C', str(created), 'ls-tree', '-r', '--name-only', 'HEAD']), b'')
                self.assertEqual([path.name for path in created.iterdir()], ['.git'])
                os.write(master, b'\x11')
                try:
                    self.assertEqual(process.wait(timeout=3), 0)
                except subprocess.TimeoutExpired:
                    while select.select([master], [], [], 0)[0]:
                        output.extend(os.read(master, 65536))
                    self.fail('New mission did not exit:\n' + visible_screen(output))
                self.assertEqual(termios.tcgetattr(slave), original)
                self.assertEqual(len(list(Path(new_state).rglob('conversations-*.json'))), 1)
                self.assertEqual(json.loads(next(Path(new_state).rglob('mission.json')).read_text())['mission'], 'fresh')
                journal = json.loads(Path(new_state, 'rust-selection-v1', 'selections.json').read_text())
                self.assertEqual(journal['schema_version'], 1)
                self.assertEqual(len(journal['records']), 1)
                selection = journal['records'][0]
                self.assertEqual(selection['request']['origin'], {'kind': 'startup'})
                self.assertEqual(selection['request']['choice']['workspace'], {'kind': 'create', 'parent': parent, 'name': 'new project'})
                self.assertEqual(selection['request']['choice']['mission'], {'kind': 'start-new', 'name': 'fresh'})
                self.assertEqual(selection['outcome'], {'phase': 'selected', 'failure': None})
                arrival_history = json.loads(next(Path(new_state).rglob('conversations-*.json')).read_text())
                arrival = arrival_history['sessions'][0]['commands'][0]
                self.assertEqual(arrival['intent'], {'kind': 'selection-arrival', 'request': selection['request']})
                self.assertEqual(arrival['state'], {'kind': 'selection', 'outcome': selection['outcome']})
                self.assertEqual(arrival_history['sessions'][0]['messages'], [])
                self.assertEqual(json.loads(saved.read_text()), revised)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)
            release_slow.set()
            release_parallel.set()
            release_reading.set()
            finish_reading.set()
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)
            os.close(master)
            os.close(slave)
            state.cleanup()
            workspace.cleanup()


if __name__ == '__main__':
    unittest.main()
