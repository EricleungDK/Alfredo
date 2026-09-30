#!/usr/bin/env python3
"""Build a locked native Linux candidate; never publish or overwrite a candidate."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tomllib

import third_party

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / 'alfredo-tui'


def run(*argv):
    return subprocess.check_output(argv, cwd=ROOT, text=True).strip()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def sources():
    paths = [ROOT / 'LICENSE', CRATE / 'Cargo.toml', CRATE / 'Cargo.lock', CRATE / 'INSTALL.md']
    paths += sorted((CRATE / 'src').rglob('*.rs'))
    paths += sorted((CRATE / 'scripts').glob('*.py'))
    return {str(path.relative_to(ROOT)): sha(path.read_bytes()) for path in paths}


def package(destination):
    if destination.exists():
        raise ValueError('Output directory already exists; choose a new candidate directory')
    compiler = run('rustc', '-vV')
    fields = dict(line.split(': ', 1) for line in compiler.splitlines() if ': ' in line)
    if fields.get('release') != '1.96.0':
        raise ValueError('Use Rust 1.96.0 to build this candidate')
    target = fields['host']
    if target != 'x86_64-unknown-linux-gnu':
        raise ValueError('Release packaging currently qualifies only native x86_64 Linux GNU')
    before = sources()
    notice_files = third_party.collect(CRATE, target)
    build = run('cargo', 'build', '--locked', '--release', '--target', target, '--manifest-path',
                str(CRATE / 'Cargo.toml'), '--message-format=json')
    artifacts = [json.loads(line) for line in build.splitlines()]
    binaries = [item['executable'] for item in artifacts
                if item.get('reason') == 'compiler-artifact'
                and item.get('target', {}).get('name') == 'alfredo-tui'
                and item.get('executable')]
    if len(binaries) != 1:
        raise ValueError('Cargo did not identify exactly one terminal executable')
    binary = Path(binaries[0]).read_bytes()
    if before != sources() or notice_files != third_party.collect(CRATE, target):
        raise ValueError('Build inputs changed during packaging; candidate not produced')
    version = tomllib.loads((CRATE / 'Cargo.toml').read_text())['package']['version']
    name = f'alfredo-tui-{version}-{target}'
    files = {'alfredo-tui': binary, 'INSTALL.md': (CRATE / 'INSTALL.md').read_bytes(),
             'Cargo.lock': (CRATE / 'Cargo.lock').read_bytes(),
             'LICENSE': (ROOT / 'LICENSE').read_bytes(), **notice_files}
    provenance = {
        'schema_version': 1, 'name': 'alfredo-tui', 'version': version, 'target': target,
        'rustc': compiler, 'source_commit': run('git', 'rev-parse', 'HEAD'),
        'source_dirty': bool(run('git', 'status', '--porcelain', '--untracked-files=normal')),
        'source_files_sha256': before,
        'files_sha256': {path: sha(data) for path, data in files.items()},
        'build_host_libc': run('ldd', '--version').splitlines()[0],
        'qualification': 'Development candidate; full product and release acceptance incomplete',
    }
    files['BUILD.json'] = (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode()
    # Fixed ordering, ownership and times make identical inputs produce identical archive bytes.
    archive = io.BytesIO()
    with gzip.GzipFile(filename='', mode='wb', fileobj=archive, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w', format=tarfile.USTAR_FORMAT) as tar:
            for path, data in sorted(files.items()):
                member = tarfile.TarInfo(f'{name}/{path}')
                member.size = len(data)
                member.mode = 0o755 if path == 'alfredo-tui' else 0o644
                tar.addfile(member, io.BytesIO(data))
    data = archive.getvalue()
    destination.mkdir(parents=True, exist_ok=False)
    filename = f'{name}.tar.gz'
    (destination / filename).write_bytes(data)
    (destination / f'{filename}.sha256').write_text(f'{sha(data)}  {filename}\n')
    print(destination / filename)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    package(args.output.resolve())
