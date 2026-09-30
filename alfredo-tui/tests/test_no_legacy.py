"""Guard: legacy desktop app and Python orchestrator stay removed from tracked files."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
LEGACY = ('mission-control', 'albert_mvp')
EXEMPT_PREFIXES = ('.agent/',)
EXEMPT_FILES = {'CHANGELOG.md', 'alfredo-tui/tests/test_no_legacy.py'}


def tracked():
    out = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode('utf-8')
    return [p for p in out.split('\0') if p and p not in EXEMPT_FILES and not p.startswith(EXEMPT_PREFIXES)]


class NoLegacyTests(unittest.TestCase):
    def test_no_tracked_path_under_legacy_dirs(self):
        found = [p for p in tracked() if p.split('/')[0] in LEGACY]
        self.assertEqual([], found)

    def test_no_tracked_file_references_legacy_names(self):
        found = []
        for path in tracked():
            file = ROOT / path
            if not file.is_file():
                continue
            text = file.read_bytes().decode('utf-8', errors='ignore')
            if any(name in text for name in LEGACY):
                found.append(path)
        self.assertEqual([], found)


if __name__ == '__main__':
    unittest.main()
