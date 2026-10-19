#!/usr/bin/env python3
"""Headless `--go GOAL --headless` journeys without a TTY (stdlib only; run after cargo build).

stdin is /dev/null and stdout/stderr are pipes, so nothing here has a terminal. The
fake Ollama of the autopilot journey plans two tasks; task 1 fails its first check
and is repaired. Asserts the one-line JSON result on stdout, stderr progress lines
and the exit codes (0 all accepted, 1 partial/failed/interrupted, 2 setup/usage).
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time
import unittest

from autopilot_terminal_smoke import AutopilotServer

GOAL = 'Make answer return 42 and add app'


class HeadlessSmoke(unittest.TestCase):
    def setUp(self):
        fallback = Path(__file__).resolve().parents[1] / 'target/debug/alfredo-tui'
        self.binary = Path(os.environ.get('ALFREDO_TUI_BINARY', str(fallback))).resolve()
        self.assertTrue(self.binary.is_file(), f'Build or set ALFREDO_TUI_BINARY: {self.binary}')
        temporary = tempfile.TemporaryDirectory(prefix='alfredo headless journey ')
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
        self.fixture.release_app.set()
        self.addCleanup(self.fixture.close)

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.workspace), *args], env=self.git_env, check=True,
                              timeout=10, capture_output=True, text=True).stdout.strip()

    def start(self, *extra, goal=GOAL):
        args = [str(self.binary), '--model', 'fixture', '--endpoint', self.fixture.endpoint,
                '--workspace', str(self.workspace), '--state-dir', str(self.state)]
        if goal is not None:
            args += ['--go', goal]
        return subprocess.Popen([*args, *extra], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, cwd=self.workspace,
                                env=dict(os.environ, ALFREDO_STATE_DIR=str(self.state)))

    def run_headless(self, *extra, goal=GOAL, timeout=90):
        process = self.start(*extra, goal=goal)
        try:
            out, err = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            out, err = process.communicate()
            self.fail(f'Timed out\nstdout: {out}\nstderr: {err}\nRequests: {self.fixture.markers()}')
        self.assertFalse(self.fixture.errors, self.fixture.errors)
        return process.returncode, out, err

    def result(self, out):
        self.assertEqual(len(out.strip().splitlines()), 1, f'exactly one JSON line expected: {out!r}')
        return json.loads(out)

    def test_goal_completes_with_progress_lines_one_json_object_and_exit_0(self):
        head = self.git('rev-parse', 'HEAD')
        code, out, err = self.run_headless('--headless')
        self.assertEqual(code, 0, err)
        result = self.result(out)
        self.assertEqual(set(result), {'state', 'goal', 'branch', 'accepted', 'total', 'repairs', 'failed'})
        self.assertEqual((result['state'], result['goal'], result['accepted'], result['total'],
                          result['repairs'], result['failed']), ('done', GOAL, 2, 2, 1, []))
        self.assertRegex(result['branch'], r'^alfredo/go-')
        lines = err.splitlines()
        for expected in ['plan drafted', '#1 approved', '#1 check failed → repair #3', '#2 approved',
                         f"done 2/2 {result['branch']}"]:
            self.assertIn(expected, lines, err)
        self.assertEqual(self.fixture.markers(), ['PLAN', 'CALC', 'REPAIR', 'APP'])
        self.assertEqual(self.git('show', f"{result['branch']}:calc.py"), 'def answer():\n    return 42')
        self.assertEqual(self.git('rev-parse', 'HEAD'), head)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_failed_tasks_exit_1_and_are_listed(self):
        self.fixture.prose_worker = True
        code, out, err = self.run_headless('--headless', '--max-repairs', '0')
        self.assertEqual(code, 1, err)
        result = self.result(out)
        self.assertEqual((result['state'], result['accepted'], result['total']), ('failed', 0, 2))
        self.assertEqual([item['id'] for item in result['failed']], [1, 2])
        for item in result['failed']:
            self.assertEqual(set(item), {'id', 'title', 'reason'})
            self.assertTrue(item['title'] and item['reason'])
        self.assertIn('failed 0/2', err)

    def test_usage_errors_exit_2_with_nothing_on_stdout(self):
        for extra, goal, message in [(['--headless'], None, '--headless needs --go'),
                                     (['--headless', '--select'], GOAL, '--select')]:
            code, out, err = self.run_headless(*extra, goal=goal)
            self.assertEqual((code, out), (2, ''), err)
            self.assertIn(message, err)

    def test_interactive_refusal_without_a_terminal_names_headless(self):
        code, out, err = self.run_headless()
        self.assertEqual((code, out), (1, ''))
        self.assertIn('--headless', err)

    def test_sigterm_cancels_workers_and_leaves_the_run_paused_like_stop(self):
        self.fixture.hold_calc = release = threading.Event()
        self.addCleanup(release.set)
        process = self.start('--headless')
        deadline = time.monotonic() + 60
        while 'CALC' not in self.fixture.markers() and time.monotonic() < deadline:
            if process.poll() is not None:
                self.fail(f'Exited early: {process.communicate()}')
            time.sleep(0.05)
        self.assertIn('CALC', self.fixture.markers())
        process.send_signal(signal.SIGTERM)
        out, err = process.communicate(timeout=30)
        release.set()
        self.assertEqual(process.returncode, 1, err)
        result = self.result(out)
        self.assertEqual((result['state'], result['accepted']), ('paused', 0))
        self.assertIn('interrupted', err)
        saved = list(self.state.glob('**/autopilot*'))
        self.assertTrue(saved, 'autopilot state is kept for /resume')


if __name__ == '__main__':
    unittest.main()
