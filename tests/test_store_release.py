import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('store_release', Path(__file__).resolve().parents[1] / 'scripts/validate-store-release.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class StoreReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.package = Path(self.tmp.name) / 'neutrondrive_0.1.0-beta1_amd64.snap'
        self.package.write_bytes(b'tested release artifact')
        self.tag = 'v0.1.0-beta1'
        self.release = dict(tag_name=self.tag, draft=False, published_at='2026-10-26',
                            assets=[dict(name=self.package.name), dict(name='SHA256SUMS')])
        self.metadata = dict(name='neutrondrive', version='0.1.0-beta.1', confinement='strict',
                             grade='devel', base='core24', architectures=['amd64'],
                             summary='Unofficial Proton Drive client for Linux')
        self.sums = hashlib.sha256(self.package.read_bytes()).hexdigest() + '  ' + self.package.name + '\n'

    def validate(self):
        module.validate(self.release, self.tag, self.package, self.sums, self.metadata)

    def test_valid_release(self):
        self.validate()

    def test_corrupt_package(self):
        self.package.write_bytes(b'changed')
        with self.assertRaises(ValueError): self.validate()

    def test_wrong_version(self):
        self.metadata['version'] = '0.1.0-beta2'
        with self.assertRaises(ValueError): self.validate()

    def test_draft(self):
        self.release['draft'] = True
        with self.assertRaises(ValueError): self.validate()

    def stable(self):
        self.tag = self.release['tag_name'] = 'v1.0.0'
        self.metadata.update(version='1.0.0', grade='stable')

    def test_stable_release(self):
        self.stable()
        self.validate()
        self.assertEqual(module.channel_for(self.tag), 'latest/stable')
        self.assertEqual(module.channel_for('v1.0.1-beta1'), 'latest/beta')

    def test_devel_grade_cannot_publish_stable(self):
        self.stable()
        self.metadata['grade'] = 'devel'
        with self.assertRaises(ValueError): self.validate()

    def test_prerelease_cannot_publish_stable(self):
        self.stable()
        self.release['prerelease'] = True
        with self.assertRaises(ValueError): self.validate()

    def test_unknown_tag_rejected(self):
        self.tag = self.release['tag_name'] = 'v1.0.0-rc1'
        with self.assertRaises(ValueError): self.validate()

    def test_missing_or_duplicate_checksum(self):
        original = self.sums
        for sums in ('', original + original):
            self.sums = sums
            with self.assertRaises(ValueError): self.validate()

    def test_foreign_package(self):
        self.release['assets'] = [dict(name='SHA256SUMS')]
        with self.assertRaises(ValueError): self.validate()

    def test_unexpected_metadata(self):
        for field, value in [('name', 'another-app'), ('confinement', 'classic'), ('architectures', ['arm64'])]:
            with self.subTest(field=field):
                original = self.metadata[field]
                self.metadata[field] = value
                with self.assertRaises(ValueError): self.validate()
                self.metadata[field] = original


if __name__ == '__main__':
    unittest.main()
