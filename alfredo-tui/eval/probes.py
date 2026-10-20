"""Harness probes for the eval (`run_eval.py --probes`): M8 chat latency, M9 keystroke echo, M10 render cost.

M14 (kill -9 recovery) needs the eval's scripted model server, so it lives in run_eval.py.
Every probe returns plain lists or dicts; run_eval.aggregate turns them into metrics. A probe that
cannot run returns {'error': reason}, so one broken probe leaves its metric null instead of aborting.
"""
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time

from inference_terminal_smoke import Terminal  # reused PTY driver

DEFAULT_CHATS, DEFAULT_KEYS, DEFAULT_RECOVERY_TRIALS = 20, 100, 3
GIT_ENV = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
RENDER_COMMAND = ['cargo', 'test', '--release', '--locked', '--manifest-path', None, '--test', 'rendering_cost', '--',
                  '--ignored', '--nocapture', '--test-threads=1']


def percentile(values, p):
    """Percentile by rounded index into a sorted copy (index round(p/100*(n-1))); None for no samples."""
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(p / 100 * (len(ordered) - 1))))]


def stats(values):
    return {'p50': percentile(values, 50), 'p95': percentile(values, 95), 'n': len(values)}


def parse_rendering_cost(text):
    """Pick the three JSON lines the ignored `rendering_cost` benches print out of cargo's test output."""
    found = {}
    for line in text.splitlines():
        # `--nocapture` puts the JSON after "test NAME ... " on the same line.
        start = line.find('{')
        if start < 0:
            continue
        try:
            data = json.loads(line[start:])
        except ValueError:
            continue
        if not isinstance(data, dict):
            continue
        if 'cohorts' in data:
            found['mission_work'] = {
                f'{c["panel"]}@{c["viewport"][0]}x{c["viewport"][1]}': {
                    key: c[f'{key}_ns'] / 1e6 for key in ('cold_p50', 'cold_p95', 'warm_p50', 'warm_p95')}
                for c in data['cohorts']}
        elif 'average_ms' in data:
            found['dashboard_average_ms'] = data['average_ms']
        elif 'warm_p95_ms' in data:
            found['chat'] = {'warm_p50_ms': data['warm_p50_ms'], 'warm_p95_ms': data['warm_p95_ms']}
    missing = [k for k in ('mission_work', 'dashboard_average_ms', 'chat') if k not in found]
    if missing:
        return {'error': f'rendering_cost printed no JSON for: {", ".join(missing)}'}
    return found


def run_render(tui_root, timeout=1800):
    command = [c if c is not None else str(Path(tui_root) / 'Cargo.toml') for c in RENDER_COMMAND]
    try:
        run = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'error': f'rendering_cost bench could not run: {error!r}'}
    if run.returncode != 0:
        tail = (run.stderr or run.stdout).strip().splitlines()[-3:]
        return {'error': f'rendering_cost bench failed ({run.returncode}): ' + ' | '.join(tail)}
    return parse_rendering_cost(run.stdout)


class MarkerServer:
    """Ollama-compatible loopback server whose reply is a unique marker, stamped on arrival and on send.

    The marker lets the probe find the reply in raw pty bytes, which is exact where spinner redraws
    make "the screen changed" ambiguous. The prompt's last word keys the stamps.
    """

    def __init__(self):
        self.arrive, self.sent = {}, {}
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_args):
                pass

            def reply(self, body, kind):
                self.send_response(200)
                self.send_header('Content-Type', kind)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                self.reply(b'{"models":[{"name":"fixture"}]}', 'application/json')

            def do_POST(self):
                arrived = time.monotonic()
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if self.path == '/api/generate':
                    return self.reply(b'{"done":true,"done_reason":"load"}\n', 'application/json')
                key = request['messages'][-1]['content'].split()[-1]
                server.arrive[key] = arrived
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write((json.dumps({'message': {'content': marker(key)}, 'done': True}) + '\n').encode())
                self.wfile.flush()
                server.sent[key] = time.monotonic()
                self.close_connection = True

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


def marker(key):
    return f'RPLY{key}X'


def _repo(root, name):
    workspace = root / name
    workspace.mkdir()
    (workspace / 'a.py').write_text('x = 1\n')
    for args in (['init', '-q', '--initial-branch=main'], ['config', 'user.name', 'f'],
                 ['config', 'user.email', 'f@x.invalid'], ['add', 'a.py'],
                 ['-c', 'commit.gpgSign=false', 'commit', '-qm', 'f']):
        subprocess.run(['git', '-C', str(workspace), *args], env=GIT_ENV, check=True, stdout=subprocess.DEVNULL)
    return workspace


def _spawn(binary, server, root, name, model):
    terminal = Terminal(binary, server.endpoint, _repo(root, name), root / f'{name}-state', 'probe', model=model)
    _drain(terminal, 1.5)  # let preload and the first frames settle
    return terminal


def _drain(terminal, seconds, nap=0.01):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        terminal.pump()
        time.sleep(nap)


def _wait(terminal, predicate, timeout=10, nap=0.0005):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        terminal.pump()
        if predicate():
            return True
        time.sleep(nap)
    return False


def run_chat(binary, chats=DEFAULT_CHATS, model='fixture', tmp_root=None):
    """Enter to request at the server and reply to screen, for `chats` plain chat turns (ms)."""
    server, terminal = MarkerServer(), None
    root = Path(tempfile.mkdtemp(prefix='alfredo-probe-', dir=tmp_root))
    try:
        terminal = _spawn(binary, server, root, 'chat', model)
        enter_to_request, reply_to_screen = [], []
        for index in range(chats):
            key = f'{index:03d}'
            terminal.send(f'chat {key}')
            if not _wait(terminal, lambda: f'chat {key}' in terminal.screen()):
                return {'error': f'chat {key} was never typed into the input'}
            start_bytes, pressed = len(terminal.output), time.monotonic()
            terminal.send('\r')  # Enter is the last byte sent
            needle = marker(key).encode()
            seen = []
            if not _wait(terminal, lambda: seen.append(time.monotonic()) or needle in bytes(terminal.output[start_bytes:]),
                         nap=0.0003):
                return {'error': f'reply marker for chat {key} never reached the screen'}
            seen_at = seen[-1]
            if key not in server.arrive or key not in server.sent:
                return {'error': f'chat {key} never reached the server'}
            enter_to_request.append((server.arrive[key] - pressed) * 1000)
            reply_to_screen.append((seen_at - server.sent[key]) * 1000)
            _drain(terminal, 0.35)
        return {'enter_to_request_ms': enter_to_request, 'reply_to_screen_ms': reply_to_screen}
    except AssertionError as error:  # Terminal.pump: the TUI exited by itself
        return {'error': f'Alfredo exited during the chat probe: {str(error)[:200]}'}
    finally:
        try:
            if terminal:
                terminal.close()
        finally:
            try:
                server.close()
            finally:
                shutil.rmtree(root, ignore_errors=True)


def run_echo(binary, keys=DEFAULT_KEYS, model='fixture', tmp_root=None):
    """Keystroke to first pty output byte, for `keys` keys typed into an idle input (ms)."""
    server, terminal = MarkerServer(), None
    root = Path(tempfile.mkdtemp(prefix='alfredo-probe-', dir=tmp_root))
    try:
        terminal = _spawn(binary, server, root, 'echo', model)
        echo = []
        for index in range(keys):
            before, pressed = len(terminal.output), time.monotonic()
            terminal.send('abcdefghij'[index % 10])
            if not _wait(terminal, lambda: len(terminal.output) > before, timeout=2, nap=0.0002):
                return {'error': f'key {index} produced no output within 2 s'}
            echo.append((time.monotonic() - pressed) * 1000)
            _drain(terminal, 0.03 + (index % 7) * 0.003, nap=0.002)  # vary the gap so ticks do not align
        return {'echo_ms': echo}
    except AssertionError as error:
        return {'error': f'Alfredo exited during the echo probe: {str(error)[:200]}'}
    finally:
        try:
            if terminal:
                terminal.close()
        finally:
            try:
                server.close()
            finally:
                shutil.rmtree(root, ignore_errors=True)
