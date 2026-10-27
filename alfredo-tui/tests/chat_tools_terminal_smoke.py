#!/usr/bin/env python3
"""Issue #233 journey: "summarise README" without @path. The fake Ollama asks for
read_file, Alfredo runs it on the Coding Workspace, shows the call and a cut result,
sends the result back, and the model's answer appears.
Issue #235 journeys: "redraft README" asks for edit_file; the approval card shows the
diff; y edits the file in place, n leaves it untouched and the model gets a refusal.
Issue #264: text typed into the prompt while the card waits never decides it.
Issue #302: Esc in the middle of an approved run_command (sandboxed or HOST) and Ctrl+Q
both kill the command, and so do SIGHUP and SIGTERM (issue #342); its later `touch marker`
never happens and the approval log says cancelled. Issue #303: a 900-character run_command at 24x80 is deny-only: y is refused with a note and
the command never runs, n denies. Issue #239 journey: "refactor everything" asks for propose_plan; the turn ends and the
prompt holds the editable /plan command; nothing else is requested or started."""
import http.server
import json
import fcntl
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
import unittest

from terminal_smoke import Pty

LONG = 'echo ' + 'a ' * 440 + '; touch pwned'
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
                elif 'find handle_request' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'grep', 'arguments': {
                            'pattern': 'def handle_request'}}}]}, 'done': False}]
                elif 'find handle_request' in prompt:
                    seen = 'src/app.py:1:def handle_request' in tool_results[-1]['content']
                    chunks = [{'message': {'content': 'GREP_FOUND_APP' if seen else 'GREP_VENV_NOISE'},
                               'done': False}]
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
                elif 'long command' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'run_command', 'arguments': {'command': LONG}}}]},
                        'done': False}]
                elif 'long command' in prompt:
                    denied = 'denied' in tool_results[-1]['content']
                    chunks = [{'message': {'content': 'LONG_DENIED' if denied else 'LONG_RAN'},
                               'done': False}]
                elif 'loop forever' in prompt:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'list_files', 'arguments': {}}}]}, 'done': False}]
                elif 'read big' in prompt and not tool_results:
                    chunks = [{'message': {'role': 'assistant', 'content': '', 'tool_calls': [
                        {'function': {'name': 'read_file', 'arguments': {'path': 'big.txt'}}}]},
                        'done': False}]
                elif 'read big' in prompt:
                    chunks = [{'message': {'content': 'BIG_DONE'}, 'done': False}]
                elif 'just do it' in prompt:
                    chunks = [{'message': {'content': 'WAYFINDER_CHAT_REPLY'}, 'done': False}]
                elif 'slow one' in prompt:
                    time.sleep(2)
                    chunks = [{'message': {'content': 'SLOW_REPLY_MARKER'}, 'done': False}]
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
                                   self.fixture.endpoint, *self.extra_args()], root, env)
        self.terminal.wait_for('ALFREDO')

    def extra_args(self):
        return ['--max-turns', '3'] if 'max_turns' in self._testMethodName else []

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


    def test_grep_skips_a_gitignored_virtualenv_so_project_files_are_found(self):
        root = Path(os.path.realpath(self.repo.name))
        root.joinpath('.gitignore').write_text('.venv/\n__pycache__/\nbuild/\n*.log\n')
        for n in range(300):
            mod = root / '.venv/lib/python3.12/site-packages' / f'pkg{n}' / 'mod.py'
            mod.parent.mkdir(parents=True)
            mod.write_text('import os\ndef handle_request_x():\n')
        root.joinpath('src').mkdir()
        root.joinpath('src/app.py').write_text('def handle_request():\n    pass\n')
        self.terminal.write(b'find handle_request\r')
        self.terminal.wait_for('GREP_FOUND_APP')
        tool = self.fixture.chats()[1]['messages'][-1]['content']
        self.assertNotIn('.venv', tool)

    def test_max_turns_caps_the_tool_rounds_and_the_notice_names_the_cap(self):
        # Issue #321: --max-turns 3 stops a looping model after 3 rounds.
        self.terminal.write(b'loop forever\r')
        self.terminal.wait_for('tool step limit reached after 3 rounds')
        self.assertEqual(len(self.fixture.chats()), 4)

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

    def group_members(self, pgid):
        """Live (not zombie) processes in process group `pgid`."""
        members = []
        for entry in Path('/proc').iterdir():
            if not entry.name.isdigit():
                continue
            try:
                fields = entry.joinpath('stat').read_text().rsplit(') ', 1)[1].split()
            except OSError:
                continue
            if int(fields[2]) == pgid and fields[0] != 'Z':
                members.append(int(entry.name))
        return members

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
        # Issue #333: the killed command still gets its row, and the next request says
        # it may have partly run.
        self.terminal.wait_for('⚙ run_command')
        self.terminal.wait_for('cancelled after')
        self.terminal.wait_for('it may have partly run')
        self.assertTrue(root.joinpath('started').exists())
        self.terminal.write(b'and now something else\r')
        self.terminal.wait_for('applied changes sent as context')
        deadline = time.time() + 10
        while time.time() < deadline:
            sent = [m['content'] for body in self.fixture.chats() for m in body['messages']
                    if m['role'] == 'assistant']
            if any('cancelled mid-run, effects may be partial' in text for text in sent):
                break
            time.sleep(0.1)
        self.assertTrue(any('[This turn was interrupted. Already applied before it stopped:\n- run_command '
                            in text and ': cancelled mid-run, effects may be partial' in text
                            for text in sent), sent)

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

    def check_signal_kills_the_host_command(self, signum, times=1):
        root = self.start_slow_command('slow command on host')
        pid = int(root.joinpath('pid').read_text())
        self.assertTrue(self.alive(pid))
        for _ in range(times):
            os.kill(self.terminal.process.pid, signum)
        deadline = time.time() + 10
        while self.terminal.process.poll() is None and time.time() < deadline:
            if select.select([self.terminal.master], [], [], 0.05)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))
        self.assertIsNotNone(self.terminal.process.poll(), 'Alfredo did not exit')
        self.assertEqual(self.terminal.process.returncode, 0, 'a signal quits like Ctrl+Q')
        self.assertFalse(self.alive(pid), 'the host command outlived Alfredo')
        # The whole process group is gone, not only the shell.
        self.assertEqual(self.group_members(pid), [], 'part of the command group survived')
        time.sleep(6)
        self.assertFalse(root.joinpath('marker').exists(), 'the command ran on after the signal')
        self.assertEqual(self.approval_outcomes(), ['cancelled'])
        return root

    def test_sighup_kills_a_running_host_command(self):
        self.check_signal_kills_the_host_command(signal.SIGHUP)

    def test_two_sighups_back_to_back_end_like_one(self):
        # Closing a terminal tab commonly delivers SIGHUP twice within milliseconds.
        self.check_signal_kills_the_host_command(signal.SIGHUP, times=2)

    def test_sigterm_kills_a_running_host_command_and_restores_the_terminal(self):
        self.check_signal_kills_the_host_command(signal.SIGTERM)
        self.assertIn(b'\x1b[?1049l', bytes(self.terminal.output), 'alternate screen left')
        self.assertTrue(termios.tcgetattr(self.terminal.slave)[3] & termios.ICANON, 'raw mode off')

    def test_a_denied_edit_never_touches_the_file(self):
        self.terminal.write(b'redraft README\r')
        self.terminal.wait_for('Approve edit_file')
        time.sleep(0.7)
        self.terminal.write(b'n')
        self.terminal.wait_for('MODEL_SAW_DENIAL')
        self.assertEqual(self.readme(), README)

    def test_a_900_character_command_at_24x80_is_deny_only(self):
        # Issue #303: the command cannot be shown whole, so y and a never arm (the old
        # card scrolled its header off and refused y forever); n still denies.
        fcntl.ioctl(self.terminal.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        self.terminal.process.send_signal(signal.SIGWINCH)
        # Keys typed while the resize is still being handled are lost (CI hit this: the
        # prompt stayed empty and the card never came). Wait for the 80-column frame to be
        # drawn, then let the input reader settle before typing.
        self.terminal.wait_for_width(80)
        time.sleep(0.5)
        self.assertGreater(len(LONG), 880)
        self.terminal.write(b'long command\r')
        self.terminal.wait_for('Approve run_command')
        self.terminal.wait_for('enlarge the terminal or press n')
        time.sleep(0.8)
        self.terminal.write(b'y')
        self.terminal.wait_for('enlarge the terminal or press n')
        time.sleep(0.8)
        self.terminal.write(b'a')
        time.sleep(0.8)
        self.assertIn('Approve run_command', self.terminal.screen())
        root = Path(os.path.realpath(self.repo.name))
        self.assertFalse(root.joinpath('pwned').exists(), 'y must not run a hidden command')
        self.terminal.write(b'n')
        self.terminal.wait_for('LONG_DENIED')
        self.assertFalse(root.joinpath('pwned').exists())
        self.assertEqual(self.approval_outcomes(), ['denied'])

    def tool_names(self, request):
        return [tool['function']['name'] for tool in request.get('tools') or []]

    def test_an_ordinary_migration_prompt_reaches_the_model_with_tools(self):
        # Issue #334: the bare word "migration" is ordinary chat, not Wayfinder.
        self.terminal.write(b'Run the migration script in migrations/ and fix any errors\r')
        self.terminal.wait_for('SUMMARY_FROM_TOOL')
        chats = self.fixture.chats()
        self.assertGreaterEqual(len(chats), 1)
        self.assertIn('read_file', self.tool_names(chats[0]))
        self.assertIn('run_command', self.tool_names(chats[0]))
        self.assertNotIn('Wayfinder', self.terminal.screen())

    def test_wayfinder_chat_says_tools_are_off_and_scope_clear_gives_them_back(self):
        # Issue #334: a project-start prompt enters Wayfinder (no request); the mission
        # chat is tool-free and says so; /scope-clear restores the tools.
        self.terminal.write(b'plan a migration of the whole system\r')
        self.terminal.wait_for('Scope is not agreed yet')
        self.terminal.wait_for('tools off')
        self.assertIn('/scope-clear', self.terminal.screen())
        self.assertEqual(len(self.fixture.chats()), 0)
        self.terminal.write(b'please just do it\r')
        self.terminal.wait_for('WAYFINDER_CHAT_REPLY')
        chats = self.fixture.chats()
        self.assertEqual(len(chats), 1)
        self.assertEqual(self.tool_names(chats[0]), [], 'a Wayfinder chat sends no tools')
        self.terminal.wait_for('tools off')
        # A refused @path must not hide the tools-off footer (and vice versa).
        self.terminal.write(b'@missing.txt please just do it\r')
        self.terminal.wait_for('not attached · Wayfinder chat: tools off')
        self.terminal.write(b'/scope-clear\r')
        self.terminal.wait_for('chat tools are back')
        self.terminal.write(b'summarise README\r')
        self.terminal.wait_for('SUMMARY_FROM_TOOL')
        chats = self.fixture.chats()
        self.assertIn('read_file', self.tool_names(chats[-1]))
        self.assertIn('propose_plan', self.tool_names(chats[-1]))

    def test_a_proposal_prefills_the_prompt_and_starts_nothing(self):
        self.terminal.write(b'refactor everything\r')
        self.terminal.wait_for('press Enter to start')
        self.terminal.wait_for('/plan Split PROPOSED_GOAL_MARKER into modules\u258f')
        time.sleep(0.5)
        self.assertEqual(len(self.fixture.chats()), 1, 'the turn ended; nothing started')
        self.assertNotIn('SHOULD_NOT_CONTINUE', self.terminal.screen())

    def test_reading_scrolled_up_over_expanded_results_never_stops_saving(self):
        # Issue #375: Ctrl+O rows and a PgUp used to push the saved anchor past a line
        # estimate, so the save failed and the next turn was lost on quit.
        Path(os.path.realpath(self.repo.name), 'big.txt').write_text(
            ''.join(f'BIG_ROW_{n}\n' for n in range(60)))
        self.terminal.write(b'read big\r')
        self.terminal.wait_for('BIG_DONE')
        self.terminal.write(b'\x0f')  # Ctrl+O: expand tool results
        self.terminal.wait_for('BIG_ROW_59')
        self.terminal.write(b'slow one\r')
        time.sleep(0.5)
        self.terminal.write(b'\x1b[5~\x1b[5~')  # PgUp twice while the reply is pending
        deadline = time.time() + 4
        while time.time() < deadline:  # the reply lands while scrolled up
            if select.select([self.terminal.master], [], [], 0.05)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))
        self.terminal.write(b'x')  # draft autosave while scrolled up
        deadline = time.time() + 2
        while time.time() < deadline:
            if select.select([self.terminal.master], [], [], 0.05)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))
        self.assertNotIn('save failed', self.terminal.screen())
        self.terminal.write(b'\x11')
        deadline = time.time() + 10
        while self.terminal.process.poll() is None and time.time() < deadline:
            if select.select([self.terminal.master], [], [], 0.05)[0]:
                self.terminal.output.extend(os.read(self.terminal.master, 65536))
        self.assertEqual(self.terminal.process.returncode, 0, 'the final save succeeded')
        self.terminal.close()
        binary = Path(os.environ.get('ALFREDO_TUI_BINARY',
            str(Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui')))
        env = dict(os.environ, TERM='xterm-256color', ALFREDO_STATE_DIR=self.state.name)
        self.terminal = Pty(self, [str(binary), '--model', 'fixture', '--endpoint',
                                   self.fixture.endpoint], os.path.realpath(self.repo.name), env)
        self.terminal.wait_for('SLOW_REPLY_MARKER')


if __name__ == '__main__':
    unittest.main()
