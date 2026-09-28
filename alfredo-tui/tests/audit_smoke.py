#!/usr/bin/env python3
"""Prove the installed auditor rejects a known vulnerable, non-product fixture.

Run after the real audit has fetched its database. No dependency is compiled or
added to the application, and the fixture never touches the product lockfile.
"""
import json
from pathlib import Path
import subprocess
import tempfile

CRATE = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix='alfredo audit fixture ') as directory:
        lockfile = Path(directory) / 'Cargo.lock'
        lockfile.write_text('''version = 4

[[package]]
name = "lz4-sys"
version = "1.9.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
''')
        result = subprocess.run([
            'cargo', 'audit', '--file', str(lockfile), '--no-fetch', '--no-yanked',
            '--deny', 'warnings', '--json',
        ], cwd=CRATE, capture_output=True, text=True, timeout=60)
        report = json.loads(result.stdout)
        ids = {item['advisory']['id'] for item in report['vulnerabilities']['list']}
        if result.returncode != 1 or 'RUSTSEC-2022-0051' not in ids:
            raise RuntimeError(f'Auditor failed to reject known vulnerability: {result.returncode}, {ids}')
        if report['settings']['ignore']:
            raise RuntimeError('Audit unexpectedly suppresses advisory findings')
        print('Audit failure gate verified against RUSTSEC-2022-0051; application lockfile untouched')


if __name__ == '__main__':
    main()
