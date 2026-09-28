"""Collect notice files from the exact registry archives pinned by Cargo.lock.

This is a conservative resolved-package inventory, including build/dev dependencies;
it neither identifies linked machine code nor decides license compatibility.
"""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tomllib

MAX_ARCHIVE = 64 * 1024 * 1024
MAX_DOCUMENT = 2 * 1024 * 1024
MAX_NOTICES = 16 * 1024 * 1024
REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def archive_path(package):
    root = Path(package['manifest_path']).parent
    return root.parents[2] / 'cache' / root.parent.name / f"{package['name']}-{package['version']}.crate"


def documents(package, checksum, archive):
    if package['source'] != REGISTRY:
        raise ValueError(f"Review unsupported dependency source: {package['name']}")
    if archive.stat().st_size > MAX_ARCHIVE:
        raise ValueError(f"Dependency archive exceeds bound: {package['name']}")
    data = archive.read_bytes()
    if digest(data) != checksum:
        raise ValueError(f"Locked dependency archive checksum mismatch: {package['name']}")
    prefix = f"{package['name']}-{package['version']}"
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tar:
        members = []
        expanded = 0
        for member in tar:
            expanded += member.size
            if len(members) >= 100_000 or expanded > 256 * 1024 * 1024:
                raise ValueError('Expanded dependency archive exceeds bounds')
            members.append(member)
        names = [member.name for member in members]
        if len(names) != len(set(names)):
            raise ValueError('Duplicate dependency archive members')
        manifest = tar.getmember(f'{prefix}/Cargo.toml')
        if not manifest.isfile() or manifest.size > MAX_DOCUMENT:
            raise ValueError('Invalid dependency manifest')
        declared = tomllib.loads(tar.extractfile(manifest).read().decode('utf-8'))['package']
        if declared.get('license') != package['license']:
            raise ValueError(f"Dependency license metadata differs from locked source: {package['name']}")
        declared_file = declared.get('license-file')
        if declared_file:
            declared_path = PurePosixPath(declared_file)
            if declared_path.is_absolute() or '..' in declared_path.parts:
                raise ValueError('Unsafe declared license-file path')
            declared_file = str(declared_path)
        result = []
        for member in sorted(members, key=lambda member: member.name):
            path = PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or path.parts[0] != prefix:
                raise ValueError('Unsafe dependency archive path')
            name = path.name.upper()
            relative = str(path.relative_to(prefix))
            if relative != declared_file and not any(word in name for word in ('LICENSE', 'LICENCE', 'NOTICE', 'COPYING', 'COPYRIGHT', 'AUTHORS', 'CONTRIBUTORS')):
                continue
            if member.isdir():
                continue
            if not member.isfile() or member.size > MAX_DOCUMENT:
                raise ValueError(f'Invalid dependency notice: {member.name}')
            text = tar.extractfile(member).read()
            text.decode('utf-8')  # Refuse unreadable notices instead of replacing bytes.
            if not text.strip():
                continue
            result.append((str(path.relative_to(prefix)), text))
        if declared_file and declared_file not in {path for path, _ in result}:
            raise ValueError('Declared license-file missing or empty')
        if not result:
            raise ValueError(f"No notice documents found: {package['name']}")
        if not package['license'] and not declared.get('license-file'):
            raise ValueError(f"Dependency license undeclared: {package['name']}")
        return result


def bundle(metadata, lock_bytes):
    lock = tomllib.loads(lock_bytes.decode('utf-8'))
    checksums = {(p['name'], p['version'], p.get('source')): p.get('checksum') for p in lock['package']}
    nodes = {node['id'] for node in metadata['resolve']['nodes']}
    ids = [package['id'] for package in metadata['packages']]
    root = metadata['resolve']['root']
    if root not in nodes or len(ids) != len(set(ids)) or not nodes.issubset(ids):
        raise ValueError('Incomplete or ambiguous resolved dependency graph')
    packages = [package for package in metadata['packages'] if package['id'] in nodes and package['id'] != root]
    notices = bytearray(b'Alfredo third-party source notices\n\n'
                        b'Includes the target-filtered resolved dependency graph, including build/dev tools.\n'
                        b'This inventory does not determine linked code or license compatibility.\n'
                        b'License expressions are retained as declared; all matching notice files follow.\n'
                        b'These third-party terms do not select a license for Alfredo itself.\n\n')
    records = []
    for package in sorted(packages, key=lambda p: (p['name'], p['version'], p['source'] or '')):
        key = (package['name'], package['version'], package['source'])
        checksum = checksums.get(key)
        if not isinstance(checksum, str) or len(checksum) != 64:
            raise ValueError(f"Missing locked dependency checksum: {package['name']}")
        record = {'name': package['name'], 'version': package['version'], 'source': package['source'],
                  'archive_sha256': checksum, 'license_expression': package['license'], 'documents': []}
        for path, data in documents(package, checksum, archive_path(package)):
            notices.extend(f"=== {package['name']} {package['version']} / {path} ===\n".encode())
            record['documents'].append({'path': path, 'sha256': digest(data), 'offset': len(notices), 'length': len(data)})
            notices.extend(data)
            notices.extend(b'\n\n')
            if len(notices) > MAX_NOTICES:
                raise ValueError('Combined dependency notices exceed bound')
        records.append(record)
    inventory = {'schema_version': 1, 'scope': 'target-filtered resolved graph including build/dev dependencies',
                 'cargo_lock_sha256': digest(lock_bytes), 'packages': records}
    return bytes(notices), (json.dumps(inventory, indent=2, sort_keys=True) + '\n').encode()


def collect(crate, target):
    metadata = json.loads(subprocess.check_output([
        'cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
        '--filter-platform', target, '--manifest-path', str(crate / 'Cargo.toml'),
    ], text=True, timeout=60))
    notices, inventory = bundle(metadata, (crate / 'Cargo.lock').read_bytes())
    record = json.loads(inventory)
    record['target'] = target
    return {'THIRD_PARTY_NOTICES.txt': notices,
            'DEPENDENCIES.json': (json.dumps(record, indent=2, sort_keys=True) + '\n').encode()}
