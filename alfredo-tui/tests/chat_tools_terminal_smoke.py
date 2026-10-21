#!/usr/bin/env python3
"""Issue #233 journey: "summarise README" without @path. The fake Ollama asks for
read_file, Alfredo runs it on the Coding Workspace, shows the call and a cut result,
sends the result back, and the model's answer appears.
Issue #235 journeys: "redraft README" asks for edit_file; the approval card shows the
diff; y edits the file in place, n leaves it untouched and the model gets a refusal.
Issue #264: text typed into the prompt while the card waits never decides it.
Issue #302: Esc in the middle of an approved run_command (sandboxed or HOST) and Ctrl+Q
both kill the command; its later `touch marker` never happens and the approval log says
cancelled. Issue #239 journey: "refactor everything" asks for propose_plan; the turn ends and the
prompt holds the editable /plan command; nothing else is requested or started."""
import http.server
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import threading
import time
import unittest

from terminal_smoke import Pty

SLOW = 'echo $$ > pid; touch started; sleep 4; touch marker'
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
                # Results of this turn only (after the latest user message).
                last_user = max(i for i, m in enumerate(body['messages']) if m['role'] == 'user')
                tool_results = [m for m in body['messages'][last_user:] if m['role'] == 'tool']
                prompt = [m for m in body['messages'] if m['role'] == 'user'][-1]['content']
                if 'refactor everything' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'propose_plan', 'arguments': {
                            'goal': 'Split PROPOSED_GOAL_MARKER into modules', 'mode': 'plan',
                            'reason': 'broad refactor'}}}]}, 'done': False}]
                elif 'refactor everything' in prompt:
                    chunks = [{'message': {'content': 'SHOULD_NOT_CONTINUE'}, 'done': False}]
                elif 'redraft' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'edit_file', 'arguments': {
                            'path': 'README.md', 'old_text': 'README_BODY_MARKER: a tiny',
                            'new_text': 'REDRAFTED_MARKER: a small'}}}]}, 'done': False}]
                elif 'redraft' in prompt:
                    denied = 'denied' in tool_results[-1]['content']
                    chunks = [{'message': {'content': 'MODEL_SAW_DENIAL' if denied else 'MODEL_SAW_EDIT'},
                               'done': False}]
                elif 'slow command' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'run_command', 'arguments': {
                            'command': SLOW, 'escalate': 'on host' in prompt}}}]}, 'done': False}]
                elif 'slow command' in prompt:
                    chunks = [{'message': {'content': 'SLOW_DONE'}, 'done': False}]
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
                                 'git_commit', 'git_switch', 'git_merge', 'git_push',
                                 'run_command', 'propose_plan'])
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
        # Issue #264: a message typed while the card waits goes to the draft and never
        # decides it, even a leading or later y/n.
        time.sleep(1.0)
        self.terminal.write(b'hey, you')
        self.terminal.wait_for('clear the draft to answer')
        self.assertEqual(self.readme(), README)
        self.terminal.write(b'\x7f' * len('hey, you'))
        time.sleep(0.7)
        self.terminal.write(b'y')
        self.terminal.wait_for('MODEL_SAW_EDIT')
        self.assertIn('REDRAFTED_MARKER: a small', self.readme())
        self.terminal.wait_for('Approved edit_file')

    def test_always_allow_then_an_identical_edit_runs_without_a_card(self):
        # Issue #238: a on the card adds a rule; the next identical edit is auto-approved.
        self.terminal.write(b'redraft README\r')
        self.terminal.wait_for('a always allow edit_file')
        time.sleep(0.7)
        self.terminal.write(b'a')
        self.terminal.wait_for('MODEL_SAW_EDIT')
        self.terminal.wait_for('Always allowed')
        Path(os.path.realpath(self.repo.name), 'README.md').write_text(README)
        self.terminal.write(b'redraft README again\r')
        deadline = time.time() + 15
        while 'REDRAFTED_MARKER' not in self.readme() and time.time() < deadline:
            time.sleep(0.1)
        self.assertIn('REDRAFTED_MARKER: a small', self.readme(), 'auto-approved, no keypress')
        logs = list(Path(self.state.name).rglob('chat-approvals.jsonl'))
        deadline = time.time() + 5
        while time.time() < deadline:
            records = [json.loads(line) for line in logs[0].read_text().splitlines()] if logs else []
            if len(records) == 2:
                break
            time.sleep(0.1)
            logs = list(Path(self.state.name).rglob('chat-approvals.jsonl'))
        self.assertEqual([r.get('rule') for r in records], [1, 1], records)
        self.terminal.write(b'/allow\r')
        self.terminal.wait_for('#1 edit_file')

    def start_slow_command(self, prompt):
        root = Path(os.path.realpath(self.repo.name))
        self.terminal.write(prompt.encode() + b'\r')
        self.terminal.wait_for('Approve run_command')
        time.sleep(0.7)
        self.terminal.write(b'y')
        deadline = time.time() + 15
        while not root.joinpath('started').exists() and time.time() < deadline:
            time.sleep(0.05)
        self.assertTrue(root.joinpath('started').exists(), 'the command never started')
        return root

    def approval_outcomes(self):
        logs = list(Path(self.state.name).rglob('chat-approvals.jsonl'))
        if not logs:
            return []
        return [json.loads(line)['outcome'] for line in logs[0].read_text().splitlines()]

    def alive(self, pid):
        try:
            state = Path(f'/proc/{pid}/stat').read_text().rsplit(') ', 1)[1][0]
        except OSError:
            return False
        return state != 'Z'

    def check_esc_stops_the_command(self, prompt):
        root = self.start_slow_command(prompt)
        # Issue #302: the status says what runs and that Esc stops it.
        self.terminal.wait_for('running run_command')
        self.terminal.wait_for('Esc stops')
        self.assertNotIn('waiting for model', self.terminal.screen())
        self.terminal.write(b'\x1b')
        self.terminal.wait_for('cancelled')
        deadline = time.time() + 3
        while 'cancelled' not in self.approval_outcomes() and time.time() < deadline:
            time.sleep(0.1)
        self.assertEqual(self.approval_outcomes(), ['cancelled'])
        time.sleep(5)
        self.assertFalse(root.joinpath('marker').exists(), 'the command ran on after Esc')

    def test_esc_stops_a_running_sandboxed_command(self):
        self.check_esc_stops_the_command('slow command')

    def test_esc_stops_a_running_host_command(self):
        self.check_esc_stops_the_command('slow command on host')
        pid = int(Path(os.path.realpath(self.repo.name), 'pid').read_text())
        self.assertFalse(self.alive(pid))

    def test_ctrl_q_kills_a_running_host_command(self):
        root = self.start_slow_command('slow command on host')
        pid = int(root.joinpath('pid').read_text())
        self.assertTrue(self.alive(pid))
        self.terminal.write(b'\x11')
        deadline = time.time() + 10
        while self.terminal.process.poll() is None and time.time() < deadline:
            if select.select([self.terminal.master], [], [], 0.05)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))
        self.assertIsNotNone(self.terminal.process.poll(), 'Alfredo did not exit')
        self.assertFalse(self.alive(pid), 'the host command outlived Alfredo')
        time.sleep(5)
        self.assertFalse(root.joinpath('marker').exists(), 'the command ran on after quit')
        self.assertEqual(self.approval_outcomes(), ['cancelled'])

    def test_a_denied_edit_never_touches_the_file(self):
        self.terminal.write(b'redraft README\r')
        self.terminal.wait_for('Approve edit_file')
        time.sleep(0.7)
        self.terminal.write(b'n')
        self.terminal.wait_for('MODEL_SAW_DENIAL')
        self.assertEqual(self.readme(), README)

    def test_a_proposal_prefills_the_prompt_and_starts_nothing(self):
        self.terminal.write(b'refactor everything\r')
        self.terminal.wait_for('press Enter to start')
        self.terminal.wait_for('/plan Split PROPOSED_GOAL_MARKER into modules\u258f')
        time.sleep(0.5)
        self.assertEqual(len(self.fixture.chats()), 1, 'the turn ended; nothing started')
        self.assertNotIn('SHOULD_NOT_CONTINUE', self.terminal.screen())


if __name__ == '__main__':
    unittest.main()
