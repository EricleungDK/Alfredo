"""Eval grader sandbox (#207): check.sh runs model-written code, so it must run under bwrap.

The live attack tests need /usr/bin/bwrap (CI installs bubblewrap and runs them); they skip
elsewhere. Everything else runs without bwrap.
"""
import importlib.util
import io
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
EVAL = ROOT / 'alfredo-tui/eval'


def load(name):
    spec = importlib.util.spec_from_file_location(name, EVAL / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(EVAL))
    sys.path.insert(0, str(ROOT / 'alfredo-tui/tests'))
    spec.loader.exec_module(module)
    return module


run_eval = load('run_eval')

BWRAP = Path('/usr/bin/bwrap')
HAVE_BWRAP = BWRAP.is_file() and Path('/usr/bin/prlimit').is_file()

# A candidate that looks like a fix to T1 (clamp/double) but also writes a marker outside the
# exported tree when it is imported. This is the attack from the #207 ticket's Evidence.
CORRECT_MATHX = '''\
def clamp(x, lo, hi):
    return max(lo, min(hi, x))


def double(x):
    return x * 2
'''


def attack_mathx(marker):
    return f'''\
import pathlib
pathlib.Path({str(marker)!r}).write_text('pwned')

''' + CORRECT_MATHX


class Fixture(unittest.TestCase):
    """A temp root holding a task repo, an outside directory the candidate targets, and no bwrap."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix='eval-grader-test-')
        self.root = Path(self._tmp.name)
        self.outside = self.root / 'outside'
        self.outside.mkdir()
        self.marker = self.outside / 'EVAL_POC_MARKER'
        self.task = next(t for t in run_eval.load_tasks(EVAL) if t['id'].startswith('T1-'))
        self.repo = self.root / 'repo'
        run_eval.make_repo(self.task, self.repo)

    def tearDown(self):
        self._tmp.cleanup()

    def commit_candidate(self, source):
        (self.repo / 'mathx.py').write_text(source, encoding='utf-8')
        run_eval.git(self.repo, 'add', '-A')
        run_eval.git(self.repo, '-c', 'commit.gpgSign=false', 'commit', '-qm', 'candidate', '--allow-empty')
        return 'HEAD'

    def no_sandbox(self):
        return mock.patch.object(run_eval, 'sandbox_tools', return_value=None)


class SandboxArgvTests(unittest.TestCase):
    """The bwrap argv mirrors the worker profile (alfredo-tui/src/worker.rs check_request)."""

    def setUp(self):
        self.export = Path('/tmp/alfredo-eval-grade-example')
        self.check = Path('/repo/tasks/T1-add-function/check.sh')
        self.argv = run_eval.sandbox_argv(self.export, self.check, bwrap='/usr/bin/bwrap',
                                          prlimit='/usr/bin/prlimit')

    def test_starts_with_bwrap_and_isolates_everything(self):
        self.assertEqual('/usr/bin/bwrap', self.argv[0])
        for flag in ('--unshare-all', '--unshare-net', '--die-with-parent', '--new-session'):
            self.assertIn(flag, self.argv)

    def test_drops_every_capability_before_the_command(self):
        # Issue #517: as root, bwrap keeps CAP_SYS_ADMIN unless every capability is dropped.
        separator = self.argv.index('--')
        prefix = self.argv[:separator]
        self.assertIn(('--cap-drop', 'ALL'), list(zip(prefix, prefix[1:])), prefix)

    def test_root_is_a_tmpfs_with_fresh_dev_and_proc(self):
        i = self.argv.index('--tmpfs')
        self.assertEqual('/', self.argv[i + 1])
        self.assertIn('--dev', self.argv)
        self.assertIn('--proc', self.argv)

    def test_system_directories_are_read_only(self):
        ro = [self.argv[i + 1] for i, a in enumerate(self.argv) if a == '--ro-bind']
        for system in ('/usr', '/etc'):
            self.assertIn(system, ro)

    def test_only_the_export_is_bound_read_write(self):
        rw = [self.argv[i + 1] for i, a in enumerate(self.argv) if a == '--bind']
        self.assertEqual([str(self.export)], rw)

    def test_check_script_is_bound_read_only_and_run_from_the_export(self):
        self.assertIn(str(self.check), self.argv)
        i = self.argv.index(str(self.check))
        self.assertEqual('--ro-bind', self.argv[i - 1])
        self.assertIn('--chdir', self.argv)
        self.assertEqual(str(self.export), self.argv[self.argv.index('--chdir') + 1])

    def test_prlimit_wraps_the_check_with_the_worker_limits(self):
        sep = self.argv.index('/usr/bin/prlimit')
        limits = self.argv[sep + 1:]
        for prefix in ('--as=', '--fsize=', '--nofile=', '--nproc='):
            self.assertTrue(any(a.startswith(prefix) for a in limits), prefix)
        self.assertEqual(['sh', run_eval.GUEST_CHECK], self.argv[-2:])

    def test_does_not_share_the_host_network_or_home(self):
        self.assertNotIn('--share-net', self.argv)
        self.assertNotIn('/home', ' '.join(self.argv))


class ArchiveCheckTests(unittest.TestCase):
    """Archive members that point or write outside the export are refused before extraction."""

    def archive(self, *members):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode='w') as tar:
            for kind, name, target in members:
                info = tarfile.TarInfo(name)
                if kind == 'file':
                    body = b'x'
                    info.size = len(body)
                    tar.addfile(info, io.BytesIO(body))
                    continue
                if kind == 'symlink':
                    info.type = tarfile.SYMTYPE
                else:
                    info.type = tarfile.LNKTYPE
                info.linkname = target
                tar.addfile(info)
        return buffer.getvalue()

    def test_plain_files_and_inside_symlinks_are_allowed(self):
        run_eval.check_archive(self.archive(('file', 'mathx.py', None), ('symlink', 'alias', 'mathx.py'),
                                            ('symlink', 'pkg/link', '../mathx.py')))

    def test_absolute_symlink_is_refused(self):
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('symlink', 'escape', '/etc/passwd')))

    def test_symlink_climbing_out_of_the_export_is_refused(self):
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('symlink', 'escape', '../../outside')))
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('symlink', 'pkg/escape', '../../../x')))

    def test_hardlink_outside_the_export_is_refused(self):
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('hardlink', 'copy', '/etc/passwd')))
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('hardlink', 'copy', '../secret')))

    def test_member_names_that_leave_the_export_are_refused(self):
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('file', '/tmp/evil', None)))
        with self.assertRaises(ValueError):
            run_eval.check_archive(self.archive(('file', '../evil', None)))


class GraderWithoutBwrapTests(Fixture):
    """Without bwrap, grading refuses to run model-written code unless explicitly opted in."""

    def test_refuses_without_bwrap_and_never_runs_the_candidate(self):
        ref = self.commit_candidate(attack_mathx(self.marker))
        with self.no_sandbox():
            ok, output = run_eval.grade(self.task, self.repo, ref)
        self.assertFalse(ok)
        self.assertIn('sandbox', output)
        self.assertIn('bubblewrap', output)
        self.assertFalse(self.marker.exists(), 'the candidate must not run unsandboxed')

    def test_unsafe_opt_in_is_the_only_way_to_grade_without_bwrap(self):
        ref = self.commit_candidate(CORRECT_MATHX)
        with self.no_sandbox():
            ok, _ = run_eval.grade(self.task, self.repo, ref, unsafe_unsandboxed=True)
        self.assertTrue(ok)

    def test_validate_tasks_refuses_instead_of_passing_vacuously(self):
        with self.no_sandbox():
            problems = run_eval.validate_tasks([self.task])
        self.assertTrue(problems)
        self.assertIn('sandbox', problems[0])

    def test_escaping_symlink_in_the_candidate_is_refused_before_extraction(self):
        os.symlink('/etc', self.repo / 'escape')
        run_eval.git(self.repo, 'add', '-A')
        run_eval.git(self.repo, '-c', 'commit.gpgSign=false', 'commit', '-qm', 'link')
        ok, output = run_eval.grade(self.task, self.repo, 'HEAD', unsafe_unsandboxed=True)
        self.assertFalse(ok)
        self.assertIn('refused', output)


class NonUtf8CheckOutputTests(Fixture):
    """#390: invalid UTF-8 from check.sh must still grade, never raise (it became harness_error)."""

    def grade_with_check(self, script):
        check = self.root / 'check.sh'
        check.write_text(script, encoding='utf-8')
        task = dict(self.task, check=check)
        with self.no_sandbox():
            return run_eval.grade(task, self.repo, 'HEAD', unsafe_unsandboxed=True)

    def test_invalid_utf8_with_exit_1_fails_with_a_readable_tail(self):
        ok, output = self.grade_with_check('printf "bad \\377 byte\\n"\nexit 1\n')
        self.assertFalse(ok)
        self.assertIn('bad', output)
        self.assertIn('byte', output)

    def test_invalid_utf8_with_exit_0_passes(self):
        ok, output = self.grade_with_check('printf "\\377\\n"\nexit 0\n')
        self.assertTrue(ok, output)


@unittest.skipUnless(HAVE_BWRAP, 'needs /usr/bin/bwrap and /usr/bin/prlimit (CI installs bubblewrap)')
class LiveSandboxTests(Fixture):
    """Reproduces the #207 attack under the real bwrap profile."""

    def test_attack_cannot_write_a_marker_outside_the_export(self):
        ref = self.commit_candidate(attack_mathx(self.marker))
        ok, output = run_eval.grade(self.task, self.repo, ref)
        self.assertFalse(self.marker.exists(), output)
        self.assertFalse(ok, 'the attack candidate must not pass its check')

    def test_a_correct_candidate_still_passes_inside_the_sandbox(self):
        ref = self.commit_candidate(CORRECT_MATHX)
        ok, output = run_eval.grade(self.task, self.repo, ref)
        self.assertTrue(ok, output)

    def test_the_candidate_cannot_see_the_host_home_directory(self):
        ref = self.commit_candidate('import os\nos.listdir(%r)\n' % str(Path.home()) + CORRECT_MATHX)
        ok, output = run_eval.grade(self.task, self.repo, ref)
        self.assertFalse(ok, 'reading the host home must fail inside the sandbox: ' + output)


if __name__ == '__main__':
    unittest.main()
