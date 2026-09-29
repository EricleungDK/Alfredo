"""Doc drift guards: entry-point docs link to real files and document every CLI flag."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
ENTRY_DOCS = ['README.md', 'CONTRIBUTING.md', 'docs/README.md', 'alfredo-tui/README.md',
              'alfredo-tui/INSTALL.md', '.agent/README.md', '.agent/Tasks/STATUS.md']
LINK = re.compile(r'\[[^\]]*\]\(([^)\s]+)\)')
IMAGE = re.compile(r'<img[^>]+src="([^"]+)"')


class LinkTests(unittest.TestCase):
    def test_relative_links_resolve(self):
        broken = []
        for name in ENTRY_DOCS:
            doc = ROOT / name
            self.assertTrue(doc.is_file(), f'{name} is missing')
            text = doc.read_text(encoding='utf-8')
            for target in LINK.findall(text) + IMAGE.findall(text):
                if re.match(r'[a-z]+:', target) or target.startswith('#'):
                    continue
                path = target.split('#')[0]
                if not (doc.parent / path).exists():
                    broken.append(f'{name} -> {target}')
        self.assertEqual([], broken)


class FlagTests(unittest.TestCase):
    def test_readme_documents_every_flag(self):
        source = (ROOT / 'alfredo-tui/src/main.rs').read_text(encoding='utf-8')
        flags = set(re.findall(r'^\s*"(--[a-z-]+)"(?: \| "--[a-z-]+")* =>', source, re.M))
        flags |= set(re.findall(r'\| "(--[a-z-]+)"', source))
        self.assertGreater(len(flags), 15)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        missing = sorted(f for f in flags if f'`{f}' not in readme)
        self.assertEqual([], missing)

    def test_readme_names_the_current_version_and_toolchain(self):
        cargo = (ROOT / 'alfredo-tui/Cargo.toml').read_text(encoding='utf-8')
        version = re.search(r'^version = "([^"]+)"', cargo, re.M).group(1)
        toolchain = re.search(r'^rust-version = "([^"]+)"', cargo, re.M).group(1)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        self.assertIn(version, readme)
        self.assertIn(f'Rust {toolchain}', readme)


class StatusTests(unittest.TestCase):
    def test_status_is_not_stale_about_branches(self):
        status = (ROOT / '.agent/Tasks/STATUS.md').read_text(encoding='utf-8')
        self.assertNotIn('`main` does not yet', status)


if __name__ == '__main__':
    unittest.main()
