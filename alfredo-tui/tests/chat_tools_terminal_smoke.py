#!/usr/bin/env python3
"""Issue #233 journey: "summarise README" without @path. The fake Ollama asks for
read_file, Alfredo runs it on the Coding Workspace, shows the call and a cut result,
sends the result back, and the model's answer appears.
Issue #235 journeys: "redraft README" asks for edit_file; the approval card shows the
diff; y edits the file in place, n leaves it untouched and the model gets a refusal."""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest

from terminal_smoke import Pty

README = '# Demo project\nREADME_BODY_MARKER: a tiny repository used by the smoke.\n'


class Fixture:
    def __init__(self):
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
                tool_results = [m for m in body['messages'] if m['role'] == 'tool']
                prompt = [m for m in body['messages'] if m['role'] == 'user'][-1]['content']
                if 'redraft' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'edit_file', 'arguments': {
                            'path': 'README.md', 'old_text': 'README_BODY_MARKER: a tiny',
                            'new_text': 'REDRAFTED_MARKER: a small'}}}]}, 'done': False}]
                elif 'redraft' in prompt:
                    denied = 'denied' in tool_results[-1]['content']
                    chunks = [{'message': {'content': 'MODEL_SAW_DENIAL' if denied else 'MODEL_SAW_EDIT'},
                               'done': False}]
                elif not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'read_file', 'arguments': {'path': 'README.md'}}}]},
                        'done': False}]
                else:
                    seen = 'README_BODY_MARKER' in tool_results[-1]['content']
                    chunks = [{'message': {'content': 'SUMMARY_FROM_TOOL' if seen else 'NO_RESULT'},
                               'done': False}]
                chunks.append({'message': {'content': ''}, 'done': True, 'prompt_eval_count': 50,
                               'eval_count': 3, 'eval_duration': 1_000_000_000})
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.end_headers()
                for chunk in chunks:
                    self.wfile.write((json.dumps(chunk) + '\n').encode())
                    self.wfile.flush()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def chats(self):
        return [body for path, body in self.requests if path == '/api/chat']

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class ChatTools(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture()
        self.state = tempfile.TemporaryDirectory(prefix='alfredo-tools-state-')
        self.repo = tempfile.TemporaryDirectory(prefix='alfredo-tools-repo-')
        root = os.path.realpath(self.repo.name)
        subprocess.run(['git', '-C', root, 'init', '-q', '--template='], check=True)
        Path(root, 'README.md').write_text(README)
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=self.state.name)
        self.terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint',
                                   self.fixture.endpoint], root, env)
        self.terminal.wait_for('ALFREDO')

    def tearDown(self):
        self.terminal.close()
        self.fixture.close()
        self.state.cleanup()
        self.repo.cleanup()

    def test_summarise_readme_runs_one_read_file_round_trip(self):
        self.terminal.write(b'summarise README\r')
        self.terminal.wait_for('SUMMARY_FROM_TOOL')
        self.terminal.wait_for('read_file README.md')
        self.terminal.wait_for('# Demo project')
        chats = self.fixture.chats()
        self.assertEqual(len(chats), 2)
        names = [tool['function']['name'] for tool in chats[0]['tools']]
        self.assertEqual(names, ['read_file', 'list_files', 'grep', 'git_status', 'git_branch',
                                 'git_log', 'git_diff', 'write_file', 'edit_file',
                                 'git_commit', 'git_switch', 'git_merge', 'git_push'])
        second = chats[1]['messages']
        self.assertEqual(second[-2]['role'], 'assistant')
        self.assertEqual(second[-2]['tool_calls'][0]['function']['name'], 'read_file')
        self.assertEqual(second[-1]['role'], 'tool')
        self.assertIn('README_BODY_MARKER', second[-1]['content'])
        self.assertIn('read_file', second[0]['content'], 'the system prompt names the tools')


    def readme(self):
        return Path(os.path.realpath(self.repo.name), 'README.md').read_text()

    def test_redraft_readme_edits_in_place_after_approval(self):
        self.terminal.write(b'redraft README\r')
        self.terminal.wait_for('Approve edit_file')
        self.terminal.wait_for('+REDRAFTED_MARKER')
        self.assertEqual(self.readme(), README, 'nothing written before approval')
        # Typeahead right after the card appears is ignored (arm delay).
        self.terminal.write(b'y')
        self.terminal.wait_for('press y or n again')
        self.assertEqual(self.readme(), README)
        time.sleep(0.7)
        self.terminal.write(b'y')
        self.terminal.wait_for('MODEL_SAW_EDIT')
        self.assertIn('REDRAFTED_MARKER: a small', self.readme())
        self.terminal.wait_for('Approved edit_file')

    def test_a_denied_edit_never_touches_the_file(self):
        self.terminal.write(b'redraft README\r')
        self.terminal.wait_for('Approve edit_file')
        time.sleep(0.7)
        self.terminal.write(b'n')
        self.terminal.wait_for('MODEL_SAW_DENIAL')
        self.assertEqual(self.readme(), README)


if __name__ == '__main__':
    unittest.main()
