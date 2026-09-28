"""Offline fixtures exercise locked notices and installed archive integrity."""
import copy
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import third_party as tp
import release_smoke


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.package = dict(id='fixture', name='fixture', version='1.0.0', source=tp.REGISTRY,
                            license='MIT', manifest_path=str(self.root / 'registry/src/index/fixture-1.0.0/Cargo.toml'))
        self.metadata = dict(packages=[dict(id='root'), self.package],
                             resolve=dict(root='root', nodes=[dict(id='root'), dict(id='fixture')]))
        self.make_source({'LICENSE': b'Original copyright\r\nMIT terms\n',
                          'vendor/NOTICE': 'Nested attribution ©\n'.encode()})

    def make_source(self, files, license='MIT', link=False):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode='w:gz') as tar:
            entries = {'Cargo.toml': f'[package]\nname="fixture"\nversion="1.0.0"\nlicense="{license}"\n'.encode(), **files}
            for name, content in entries.items():
                member = tarfile.TarInfo('fixture-1.0.0/' + name)
                member.size = len(content)
                if link and name == 'LICENSE':
                    member.type = tarfile.SYMTYPE
                    member.linkname = '/etc/passwd'
                    member.size = 0
                tar.addfile(member, io.BytesIO(content))
        archive = tp.archive_path(self.package)
        archive.parent.mkdir(parents=True, exist_ok=True)
        archive.write_bytes(data.getvalue())
        self.lock = (f'[[package]]\nname="fixture"\nversion="1.0.0"\nsource="{tp.REGISTRY}"\n'
                     f'checksum="{tp.digest(data.getvalue())}"\n').encode()

    def test_preserves_nested_exact_bytes_and_is_deterministic(self):
        notices, inventory = tp.bundle(self.metadata, self.lock)
        records = json.loads(inventory)['packages']
        self.assertEqual(len(records), 1)
        docs = records[0]['documents']
        self.assertEqual([d['path'] for d in docs], ['LICENSE', 'vendor/NOTICE'])
        self.assertEqual(notices[docs[0]['offset']:docs[0]['offset'] + docs[0]['length']], b'Original copyright\r\nMIT terms\n')
        reordered = copy.deepcopy(self.metadata)
        reordered['packages'].reverse()
        self.assertEqual(tp.bundle(reordered, self.lock), (notices, inventory))

    def test_modified_cached_archive_refused(self):
        with tp.archive_path(self.package).open('ab') as stream:
            stream.write(b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            tp.bundle(self.metadata, self.lock)

    def test_missing_notices_refused(self):
        self.make_source({'README': b'No license here'})
        with self.assertRaisesRegex(ValueError, 'No notice'):
            tp.bundle(self.metadata, self.lock)

    def test_metadata_license_mismatch_refused(self):
        self.package['license'] = 'Apache-2.0'
        with self.assertRaisesRegex(ValueError, 'metadata differs'):
            tp.bundle(self.metadata, self.lock)

    def test_linked_notice_refused(self):
        self.make_source({'LICENSE': b''}, link=True)
        with self.assertRaisesRegex(ValueError, 'Invalid dependency notice'):
            tp.bundle(self.metadata, self.lock)

    def test_traversal_refused_without_extraction(self):
        self.make_source({'../LICENSE': b'unsafe'})
        with self.assertRaisesRegex(ValueError, 'Unsafe'):
            tp.bundle(self.metadata, self.lock)
        self.assertFalse((self.root / 'LICENSE').exists())

    def installed_archive(self, damage=None):
        notices, raw = tp.bundle(self.metadata, self.lock)
        inventory = json.loads(raw)
        inventory['target'] = 'fixture-target'
        files = {'LICENSE': b'fixture project license', 'alfredo-tui': b'fixture binary', 'INSTALL.md': b'fixture install',
                 'Cargo.lock': self.lock, 'THIRD_PARTY_NOTICES.txt': notices,
                 'DEPENDENCIES.json': json.dumps(inventory).encode()}
        if damage:
            damage(files, inventory)
        manifest = {'target': 'fixture-target', 'files_sha256': {k: tp.digest(v) for k, v in files.items()}}
        files['BUILD.json'] = json.dumps(manifest).encode()
        archive = self.root / 'candidate.tar.gz'
        with tarfile.open(archive, 'w:gz') as tar:
            for name, data in files.items():
                member = tarfile.TarInfo('candidate/' + name)
                member.size = len(data)
                tar.addfile(member, io.BytesIO(data))
        Path(str(archive) + '.sha256').write_text(f'{tp.digest(archive.read_bytes())}  {archive.name}\n')
        return archive

    def test_installed_inventory_verified(self):
        payload, _ = release_smoke.verify(self.installed_archive())
        self.assertIn('DEPENDENCIES.json', payload)

    def test_inner_notice_damage_rejected_despite_rehashed_payload(self):
        def damage(files, inventory):
            data = bytearray(files['THIRD_PARTY_NOTICES.txt'])
            data[inventory['packages'][0]['documents'][0]['offset']] ^= 1
            files['THIRD_PARTY_NOTICES.txt'] = bytes(data)
        with self.assertRaisesRegex(ValueError, 'notice digest mismatch'):
            release_smoke.verify(self.installed_archive(damage))

    def test_missing_inventory_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Unexpected archive members'):
            release_smoke.verify(self.installed_archive(lambda files, _: files.pop('DEPENDENCIES.json')))


if __name__ == '__main__':
    unittest.main()
