#!/usr/bin/env python3
"""Issue #187 journey: near the size limit the oldest turns are summarised by the model
(fake Ollama), the marker shows, a failed summary keeps the prompt, /clear starts fresh."""
import http.server
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import threading
import unittest

from terminal_smoke import Pty

SUMMARY_PROMPT = 'You compress an earlier part of a chat'


class Fixture:
    def __init__(self):
        self.requests = []
        self.fail_summary = False
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
                summary = body['messages'][0]['content'].startswith(SUMMARY_PROMPT)
                if summary and fixture.fail_summary:
                    self.send_response(500)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                text = 'SUMMARY_NOTE_OF_EARLIER_TURNS' if summary else f'REPLY_{len(fixture.chats())}'
                self.send_response(200)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.end_headers()
                for chunk in (
                    {'message': {'content': text}, 'done': False},
                    {'message': {'content': ''}, 'done': True, 'prompt_eval_count': 100,
                     'eval_count': 3, 'eval_duration': 1_000_000_000},
                ):
                    self.wfile.write((json.dumps(chunk) + '\n').encode())
                    self.wfile.flush()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'

    def chats(self):
        return [body for path, body in self.requests if path == '/api/chat']

    def summaries(self):
        return [b for b in self.chats() if b['messages'][0]['content'].startswith(SUMMARY_PROMPT)]

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class Compaction(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture()
        self.state = tempfile.TemporaryDirectory(prefix='alfredo-compact-state-')
        self.repo = tempfile.TemporaryDirectory(prefix='alfredo-compact-repo-')
        root = os.path.realpath(self.repo.name)
        subprocess.run(['git', '-C', root, 'init', '-q', '--template='], check=True)
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=self.state.name)
        # An 8192-token window puts the compaction budget at 8 KiB, so three small pastes trigger it.
        self.terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint',
                                   self.fixture.endpoint, '--num-ctx', '8192'], root, env)
        self.terminal.wait_for('ALFREDO')

    def tearDown(self):
        self.terminal.close()
        self.fixture.close()
        self.state.cleanup()
        self.repo.cleanup()

    def paste(self, text):
        data = b'\x1b[200~' + text.encode() + b'\x1b[201~'
        for start in range(0, len(data), 1024):
            self.terminal.write(data[start:start + 1024])
            while select.select([self.terminal.master], [], [], 0.02)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))

    def send(self, label, size=3000):
        self.paste(f'{label} ' + 'x' * (size - len(label) - 1))
        self.terminal.write(b'\r')

    def fill(self):
        for number in (1, 2):
            self.send(f'PASTE{number}')
            self.terminal.wait_for(f'REPLY_{number}')

    def test_near_the_limit_older_turns_are_summarised_and_the_prompt_is_sent(self):
        self.fill()
        self.assertEqual(self.fixture.summaries(), [])
        self.send('PASTE3')
        self.terminal.wait_for('REPLY_4')  # the summary call was chat 3
        self.terminal.wait_for('1 earlier turn compacted into a summary')  # footer notice
        for _ in range(30):
            self.terminal.write(b'\x1b[5~')  # PgUp to the top of the transcript
        self.terminal.wait_for('SUMMARY_NOTE_OF_EARLIER_TURNS')
        self.terminal.wait_for('▸ 1 earlier turn compacted into a summary')  # transcript marker
        summaries = self.fixture.summaries()
        self.assertEqual(len(summaries), 1)
        asked = summaries[0]['messages'][1]['content']
        self.assertIn('PASTE1', asked)
        self.assertNotIn('PASTE2', asked)
        self.assertNotIn('PASTE3', asked)
        final = self.fixture.chats()[-1]['messages']
        joined = [m['content'] for m in final]
        self.assertTrue(any('SUMMARY_NOTE_OF_EARLIER_TURNS' in c for c in joined))
        self.assertFalse(any(c.startswith('PASTE1') for c in joined))
        self.assertTrue(any(c.startswith('PASTE2') for c in joined), 'newest turns stay verbatim')
        self.assertTrue(final[-1]['content'].startswith('PASTE3'), 'the new prompt is last')
        self.assertEqual(final[0]['role'], 'system')

    def test_failed_summary_keeps_the_prompt_and_names_the_ways_forward(self):
        self.fill()
        self.fixture.fail_summary = True
        self.send('PASTE3')
        self.terminal.wait_for('Summary failed', timeout=60)
        screen = self.terminal.screen()
        self.assertIn('/clear', screen)
        self.assertIn('Ctrl+N', screen)
        self.assertIn('xxxx', screen.split('Prompt · Enter send')[1], 'the prompt stays in the draft')
        self.assertEqual(len([c for c in self.fixture.chats() if c not in self.fixture.summaries()]), 2)

    def test_clear_starts_the_conversation_fresh(self):
        self.fill()
        self.terminal.write(b'/clear\r')
        self.terminal.wait_for('Conversation cleared')
        self.assertNotIn('REPLY_1', self.terminal.screen())
        self.terminal.write(b'hello again\r')
        self.terminal.wait_for('REPLY_3')
        self.assertEqual(len(self.fixture.chats()[-1]['messages']), 2)  # context + the one prompt

    def test_compact_summarises_on_demand_without_sending(self):
        self.send('PASTE1', 200)
        self.terminal.wait_for('REPLY_1')
        self.send('PASTE2', 200)
        self.terminal.wait_for('REPLY_2')
        self.terminal.write(b'/compact\r')
        self.terminal.wait_for('1 earlier turn compacted into a summary')
        self.assertEqual(len(self.fixture.summaries()), 1)
        self.assertEqual(len(self.fixture.chats()), 3)


if __name__ == '__main__':
    unittest.main()
