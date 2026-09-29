#!/usr/bin/env python3
"""Verify a locally built archive and exercise its installed binary outside the checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib

# PTY journeys run against the installed binary; autopilot covers /go end to end,
# the agent journey steering and instructing workers from the agent view.
SMOKES = ['terminal_smoke.py', 'inference_terminal_smoke.py', 'qualification_cli_smoke.py',
          'recovery_terminal_smoke.py', 'autopilot_terminal_smoke.py', 'agent_terminal_smoke.py']


def verify(archive):
    expected, filename = Path(str(archive) + '.sha256').read_text().strip().split('  ')
    if filename != archive.name or hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise ValueError('Archive checksum mismatch')
    with tarfile.open(archive, 'r:gz') as tar:
        members = tar.getmembers()
        prefix = archive.name.removesuffix('.tar.gz')
        expected_names = {f'{prefix}/{name}' for name in ['alfredo-tui', 'INSTALL.md', 'BUILD.json', 'Cargo.lock', 'DEPENDENCIES.json', 'THIRD_PARTY_NOTICES.txt', 'LICENSE']}
        if len(members) != 7 or {m.name for m in members} != expected_names:
            raise ValueError('Unexpected archive members')
        if any(not m.isfile() or m.size > 64 * 1024 * 1024 for m in members):
            raise ValueError('Unsafe archive member')
        payload = {Path(m.name).name: tar.extractfile(m).read() for m in members}
    manifest = json.loads(payload['BUILD.json'])
    expected_payload = set(payload) - {'BUILD.json'}
    if set(manifest['files_sha256']) != expected_payload:
        raise ValueError('Incomplete payload checksums')
    for name in expected_payload:
        if hashlib.sha256(payload[name]).hexdigest() != manifest['files_sha256'][name]:
            raise ValueError(f'Payload checksum mismatch: {name}')
    inventory = json.loads(payload['DEPENDENCIES.json'])
    if inventory['schema_version'] != 1 or inventory['target'] != manifest['target']:
        raise ValueError('Dependency inventory identity mismatch')
    if inventory['cargo_lock_sha256'] != hashlib.sha256(payload['Cargo.lock']).hexdigest():
        raise ValueError('Dependency inventory lock mismatch')
    locked = {(p['name'], p['version'], p.get('source')): p.get('checksum') for p in tomllib.loads(payload['Cargo.lock'].decode())['package']}
    seen = set()
    for package in inventory['packages']:
        key = (package['name'], package['version'], package['source'])
        if key in seen or locked.get(key) != package['archive_sha256'] or not package['documents']:
            raise ValueError('Invalid dependency package record')
        seen.add(key)
        for document in package['documents']:
            start, length = document['offset'], document['length']
            notices = payload['THIRD_PARTY_NOTICES.txt']
            if not isinstance(start, int) or not isinstance(length, int) or start < 0 or length <= 0 or start + length > len(notices):
                raise ValueError('Invalid notice range')
            if hashlib.sha256(notices[start:start + length]).hexdigest() != document['sha256']:
                raise ValueError('Dependency notice digest mismatch')
    if not seen:
        raise ValueError('Empty dependency inventory')
    return payload, manifest


def smoke(archive):
    payload, manifest = verify(archive)
    with tempfile.TemporaryDirectory(prefix='alfredo installed candidate ') as directory:
        root = Path(directory)
        corrupted = root / archive.name
        data = bytearray(archive.read_bytes())
        data[len(data) // 2] ^= 1
        corrupted.write_bytes(data)
        Path(str(corrupted) + '.sha256').write_bytes(Path(str(archive) + '.sha256').read_bytes())
        try:
            verify(corrupted)
        except ValueError:
            pass
        else:
            raise ValueError('Corrupted archive passed verification')
        prefix = root / 'local bin'
        prefix.mkdir()
        binary = prefix / 'alfredo-tui'
        binary.write_bytes(payload['alfredo-tui'])
        binary.chmod(0o755)
        env = dict(os.environ, PATH=f'{prefix}:/usr/bin:/bin', ALFREDO_TUI_BINARY=str(binary))
        version = subprocess.check_output(['alfredo-tui', '--version'], cwd=root, env=env, text=True).strip()
        if version != f"alfredo-tui {manifest['version']}":
            raise ValueError('Installed version does not match build provenance')
        subprocess.run(['alfredo-tui', '--help'], cwd=root, env=env, check=True, stdout=subprocess.DEVNULL)
        for smoke_script in SMOKES:
            subprocess.run([sys.executable, str(Path(__file__).with_name(smoke_script).resolve())],
                           cwd=root, env=env, check=True)
        print(f'Installed archive acceptance passed: {version} ({manifest["target"]})')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    smoke(parser.parse_args().archive.resolve())
