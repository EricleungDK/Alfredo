"""Release workflow guard: crates are fetched before offline packaging."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
RELEASE = ROOT / '.github/workflows/release.yml'
FETCH = re.compile(r'cargo fetch\b(?=.*--locked)(?=.*--manifest-path[ =]+\S*alfredo-tui/Cargo\.toml)')


def steps(path):
    """Split workflow text into (name, body) per step; stdlib only, no YAML parser."""
    text = path.read_text(encoding='utf-8')
    parts = re.split(r'^\s*- (?=name:|uses:)', text, flags=re.M)[1:]
    out = []
    for part in parts:
        match = re.match(r'name:\s*(.+)', part)
        out.append((match.group(1).strip() if match else '', part))
    return out


class ReleaseWorkflowTests(unittest.TestCase):
    def test_fetch_precedes_offline_packaging(self):
        listed = steps(RELEASE)
        names = [name for name, _ in listed]
        self.assertIn('Package release archive', names)
        package = names.index('Package release archive')
        fetched = [i for i, (_, body) in enumerate(listed) if FETCH.search(body)]
        self.assertTrue(fetched, 'release.yml has no `cargo fetch --locked --manifest-path alfredo-tui/Cargo.toml`')
        self.assertLess(fetched[0], package)


if __name__ == '__main__':
    unittest.main()
