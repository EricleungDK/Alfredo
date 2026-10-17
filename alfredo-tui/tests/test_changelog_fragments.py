"""Changelog fragments under changelog.d/ are well formed and fold into CHANGELOG.md."""
from pathlib import Path
import importlib.util
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('collect_changelog', ROOT / 'alfredo-tui/scripts/collect_changelog.py')
collect = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collect)


class ChangelogFragmentTests(unittest.TestCase):
    def test_every_fragment_in_the_repo_is_valid(self):
        for path in collect.fragment_paths(ROOT):
            with self.subTest(fragment=path.name):
                collect.parse_fragment(path)

    def test_fold_appends_to_existing_and_new_sections_and_deletes_fragments(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'changelog.d').mkdir()
            (root / 'changelog.d/README.md').write_text('# Fragments\n', encoding='utf-8')
            (root / 'CHANGELOG.md').write_text(
                '# Changelog\n\n## [Unreleased]\n\n### Fixed\n\n- old fix.\n\n## [0.1.0]\n\n- first.\n',
                encoding='utf-8')
            (root / 'changelog.d/2-b.md').write_text('### Fixed\n\n- new fix.\n', encoding='utf-8')
            (root / 'changelog.d/1-a.md').write_text('### Added\n\n- new feature\n  continued.\n', encoding='utf-8')
            self.assertEqual(0, collect.main([], root=root))
            text = (root / 'CHANGELOG.md').read_text(encoding='utf-8')
            self.assertEqual(
                '# Changelog\n\n## [Unreleased]\n\n### Added\n\n- new feature\n  continued.\n\n'
                '### Fixed\n\n- old fix.\n\n- new fix.\n\n## [0.1.0]\n\n- first.\n', text)
            self.assertEqual(['README.md'], sorted(p.name for p in (root / 'changelog.d').iterdir()))

    def test_malformed_fragment_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / 'bad.md'
            bad.write_text('- entry without a section\n', encoding='utf-8')
            with self.assertRaises(ValueError):
                collect.parse_fragment(bad)
            bad.write_text('### Fixes\n\n- wrong heading\n', encoding='utf-8')
            with self.assertRaises(ValueError):
                collect.parse_fragment(bad)


if __name__ == '__main__':
    unittest.main()
