#!/usr/bin/env python3
"""Installed recovery acceptance; subprocess Rust tests provide actual crash cuts.

The recovery journey constructs a crash state from a real installed worker's
checkpoint and captured Running snapshot. It does not claim to kill that worker
at the checkpoint boundary. A separate real-keyboard test covers narrow paging.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import termios
import threading
import time
import unittest

from inference_terminal_smoke import FixtureServer, Terminal


class RecoveryTerminalSmoke(unittest.TestCase):
    def setUp(self):
        fallback = Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'
        self.binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(fallback))).resolve()
        self.assertTrue(self.binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {self.binary}')
        temporary = tempfile.TemporaryDirectory(prefix='alfredo installed recovery ')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.workspace, self.state = self.root / 'workspace', self.root / 'state'
        self.workspace.mkdir()
        (self.workspace / 'answer.py').write_text('VALUE = 0\n')
        git_env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
        for args in [
            ['init', '-q', '--template=', '--initial-branch=main'],
            ['config', 'user.name', 'Fixture'],
            ['config', 'user.email', 'fixture@example.invalid'],
            ['add', 'answer.py'],
            ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false', 'commit', '-qm', 'fixture'],
        ]:
            subprocess.run(['git', '-C', str(self.workspace), *args], env=git_env,
                           check=True, timeout=5, stdout=subprocess.DEVNULL)
        self.fixture = FixtureServer()
        self.addCleanup(self.fixture.close)
        self.terminals = []
        self.addCleanup(lambda: [terminal.close() for terminal in self.terminals])

    def terminal(self, **options):
        terminal = Terminal(self.binary, self.fixture.endpoint, self.workspace,
                            self.state, 'recovery', **options)
        self.terminals.append(terminal)
        self.screen_has(terminal, 'ALFREDO')
        return terminal

    def wait_until(self, label, predicate, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for terminal in self.terminals:
                terminal.pump()
            self.assertFalse(self.fixture.errors, self.fixture.errors)
            if predicate():
                return
            time.sleep(0.02)
        screens = '\n\n'.join(terminal.screen() for terminal in self.terminals if not terminal.closed)
        self.fail(f'Timed out: {label}\n{screens}')

    def screen_has(self, terminal, text, timeout=10):
        self.wait_until(f'screen contains {text!r}', lambda: text in terminal.screen(), timeout)

    def task_path(self):
        paths = list(self.state.glob('rust-tasks-v1/*/tasks.json'))
        return paths[0] if paths else None

    def snapshot(self):
        path = self.task_path()
        return json.loads(path.read_text()) if path else {'revision': 0, 'tasks': []}

    def command_revision(self, terminal, command, revision):
        terminal.send(command + '\r')
        self.wait_until(f'revision {revision}', lambda: self.snapshot()['revision'] == revision)
        self.screen_has(terminal, f'revision {revision}')

    def stop(self, terminal):
        terminal.expected_exit = True
        terminal.send(b'\x11')
        self.assertEqual(terminal.process.wait(timeout=5), 0)
        self.assertEqual(termios.tcgetattr(terminal.slave), terminal.original)
        terminal.close()

    def page_to(self, terminal, text, timeout=8):
        """Use actual page keys; allow a blank-to-blank page to emit no diff."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            terminal.pump()
            previous = terminal.screen()
            if text in previous:
                return
            terminal.send(b'\x1b[6~')
            changed_by = min(deadline, time.monotonic() + 0.12)
            while time.monotonic() < changed_by:
                terminal.pump()
                if terminal.screen() != previous:
                    break
                time.sleep(0.01)
        self.fail(f'Paging did not reach {text!r}:\n{terminal.screen()}')

    def test_narrow_page_keys_do_not_skip_task_inspector_rows(self):
        terminal = self.terminal(height=10, width=32)
        terminal.send('/task Inspect every detail\r')
        self.screen_has(terminal, '○ #1')
        self.screen_has(terminal, 'Needs approval')
        before = self.task_path().read_bytes()
        # Two detail rows at 32x10: page down to the last row, then back up.
        self.page_to(terminal, '/permit 1 JSON')
        terminal.send(b'\x1b[5~' * 8)
        self.screen_has(terminal, 'Needs approval')
        self.assertEqual(self.task_path().read_bytes(), before)
        self.assertEqual(self.fixture.count(), 0)
        self.stop(terminal)

    def test_genuine_checkpoint_recovers_constructed_crash_state_without_replay(self):
        self.fixture.gates['WORKER'] = threading.Event()
        terminal = self.terminal()
        self.command_revision(terminal, '/task Checkpoint worker', 1)
        policy = {
            'files': ['answer.py', 'check-count.txt'],
            'check': ['/usr/bin/python3', '-B', '-c',
                      "from pathlib import Path; from answer import VALUE; "
                      "p=Path('check-count.txt'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); "
                      "assert VALUE == 42; print('CHECKPOINT_STDOUT_SENTINEL')"],
        }
        self.command_revision(terminal, '/permit 1 ' + json.dumps(policy), 2)
        self.command_revision(terminal, '/approve 1', 3)
        terminal.send('/run 1\r')
        self.wait_until('worker reached model request', lambda: self.fixture.count('WORKER') == 1)
        running_bytes = self.task_path().read_bytes()
        running = json.loads(running_bytes)
        self.assertEqual(running['tasks'][0]['status'], 'running')
        run = running['tasks'][0]['run']['id']
        self.fixture.gates['WORKER'].set()
        self.wait_until('worker finished', lambda: self.snapshot()['tasks'][0]['status'] == 'review-ready', 20)
        self.screen_has(terminal, 'needs review')
        run_dir = self.task_path().parent / 'runs' / run
        checkpoint = run_dir / 'check-result.json'
        self.assertTrue(checkpoint.is_file(), 'Worker must checkpoint its real returned receipt')
        checkpoint_bytes = checkpoint.read_bytes()
        final = json.loads((run_dir / 'evidence.json').read_text())
        self.assertEqual(json.loads(checkpoint_bytes)['receipt'], final['check'])
        counter = run_dir / 'worktree' / 'check-count.txt'
        self.assertEqual(counter.read_text(), '1')
        self.stop(terminal)

        # Construct only this isolated fixture's missing-finalization state. A new
        # conversation avoids retaining the old complete command's presentation.
        self.task_path().write_bytes(running_bytes)
        (run_dir / 'evidence.json').unlink()
        terminal = self.terminal(resume=True, conversation='recovery-inspection')
        terminal.send('/tasks #1\r')
        self.screen_has(terminal, 'Worker stopped after check')
        self.assertEqual(self.fixture.count(), 1)
        terminal.send('/recover 1\r')
        self.wait_until('recovered failed task', lambda: self.snapshot()['tasks'][0]['status'] == 'failed')
        self.screen_has(terminal, 'Interrupted worker recorded as failed after check')
        recovered = self.task_path().read_bytes()
        evidence = json.loads((run_dir / 'evidence.json').read_text())
        self.assertEqual(evidence['status'], 'failed')
        self.assertIsNone(evidence.get('candidate_commit'))
        self.assertEqual(evidence['patch'], '')
        self.assertEqual(evidence['check'], final['check'])
        self.assertEqual(checkpoint.read_bytes(), checkpoint_bytes)
        self.assertEqual(counter.read_text(), '1')
        self.assertEqual(self.fixture.count(), 1)
        terminal.send('/recover 1\r')
        self.screen_has(terminal, 'already acknowledged')
        self.assertEqual(self.task_path().read_bytes(), recovered)
        terminal.send('/evidence 1\r')
        self.screen_has(terminal, 'CHECKPOINT_STDOUT_SENTINEL')
        self.stop(terminal)
        self.assertEqual(self.fixture.count(), 1)
        self.assertEqual(counter.read_text(), '1')

        terminal = self.terminal(resume=True, conversation='narrow-inspection', height=10, width=32)
        terminal.send('/tasks #1\r')
        self.screen_has(terminal, '✗ #1')
        # The Result value wraps under its label at 32 columns.
        self.page_to(terminal, 'interrupted')
        terminal.send('/evidence 1\r')
        self.screen_has(terminal, 'Verified run evidence')
        self.page_to(terminal, 'CHECKPOINT_STDOUT_SENTINEL')
        self.assertEqual(self.task_path().read_bytes(), recovered)
        self.assertEqual(checkpoint.read_bytes(), checkpoint_bytes)
        self.assertEqual(self.fixture.count(), 1)
        self.assertEqual(counter.read_text(), '1')
        self.stop(terminal)


if __name__ == '__main__':
    unittest.main()
