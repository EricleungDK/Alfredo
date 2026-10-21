#!/usr/bin/env python3
"""--num-ctx reaches the wire and the chat shows context fill (stdlib Linux PTY fixture)."""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

from terminal_smoke import Pty


class Fixture:
    def __init__(self, prompt_tokens):
        self.requests = []
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
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                fixture.requests.append((self.path, body))
                if self.path == '/api/generate':
                    payload = b'{"done":true,"done_reason":"load"}\n'
                    self.send_response(200)
                    self.send_header('Content-Length', str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                    return
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.end_headers()
                for chunk in (
                    {'message': {'content': 'NUM_CTX_REPLY'}, 'done': False},
                    {'message': {'content': ''}, 'done': True, 'prompt_eval_count': prompt_tokens,
                     'eval_count': 3, 'eval_duration': 1_000_000_000},
                ):
                    self.wfile.write((json.dumps(chunk) + '\n').encode())
                    self.wfile.flush()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class NumCtx(unittest.TestCase):
    def chat(self, flags, prompt_tokens, env_extra=None):
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        fixture = Fixture(prompt_tokens)
        with tempfile.TemporaryDirectory(prefix='alfredo-ctx-state-') as state, \
                tempfile.TemporaryDirectory(prefix='alfredo-ctx-repo-') as repo:
            root = os.path.realpath(repo)
            subprocess.run(['git', '-C', root, 'init', '-q', '--template='], check=True)
            env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=state, **(env_extra or {}))
            terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint', fixture.endpoint, *flags], root, env)
            try:
                terminal.wait_for('ALFREDO')
                terminal.write(b'hello ctx\r')
                terminal.wait_for('NUM_CTX_REPLY')
                return fixture, terminal
            except BaseException:
                terminal.close()
                fixture.close()
                raise

    def test_flag_is_sent_and_fill_indicator_shown(self):
        fixture, terminal = self.chat(['--num-ctx', '8192'], 1200)
        try:
            terminal.wait_for('ctx 1200/8192')
            self.assertNotIn('truncated', terminal.screen())
            chat = [body for path, body in fixture.requests if path == '/api/chat']
            preload = [body for path, body in fixture.requests if path == '/api/generate']
            self.assertEqual(chat[0]['options']['num_ctx'], 8192)
            self.assertEqual(preload[0]['options']['num_ctx'], 8192)
        finally:
            terminal.close()
            fixture.close()

    def test_environment_and_near_full_window_warns(self):
        fixture, terminal = self.chat([], 7900, {'ALFREDO_NUM_CTX': '8192'})
        try:
            terminal.wait_for('ctx 7900/8192')
            terminal.wait_for('context may be truncated')
            chat = [body for path, body in fixture.requests if path == '/api/chat']
            self.assertEqual(chat[0]['options']['num_ctx'], 8192)
        finally:
            terminal.close()
            fixture.close()

    def test_small_windows_send_a_short_prompt_with_clamped_num_predict(self):
        for window in (2048, 4096):
            fixture, terminal = self.chat(['--num-ctx', str(window)], 10)
            try:
                chat = [body for path, body in fixture.requests if path == '/api/chat']
                options = chat[0]['options']
                self.assertEqual(options['num_ctx'], window)
                self.assertTrue(256 <= options['num_predict'] < window, options)
                self.assertGreater(options['num_predict'], window - 1000)  # tool definitions (incl. propose_plan) count against the prompt
            finally:
                terminal.close()
                fixture.close()

    def test_unset_omits_the_option_and_shows_default(self):
        fixture, terminal = self.chat([], 1200)
        try:
            terminal.wait_for('ctx 1200/default')
            for _path, body in fixture.requests:
                self.assertNotIn('num_ctx', body.get('options', {}))
        finally:
            terminal.close()
            fixture.close()


if __name__ == '__main__':
    unittest.main()
