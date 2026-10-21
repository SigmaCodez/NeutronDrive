"""Persistent AppImage cache must not follow links or reuse corrupted code."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
spec = importlib.util.spec_from_file_location('portable_bootstrap', Path(__file__).resolve().parents[1] / 'packaging/appimage/bootstrap.py')
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)

class PortableCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.source = self.base / 'source'
        self.source.mkdir()
        (self.source / 'AppRun').write_text('test launcher')
        self.manifest = {'AppRun': bootstrap.digest(self.source / 'AppRun')}
        (self.source / 'runtime-manifest.json').write_text(json.dumps(self.manifest))
        self.cache = self.base / 'cache'

    def test_runtime_survives_original_mount_and_is_reused(self):
        target = bootstrap.prepare(self.source, self.cache)
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)
        self.assertEqual(target, bootstrap.prepare(target, self.cache))
        (self.source / 'AppRun').unlink()
        self.assertTrue(bootstrap.valid(target, self.manifest))

    def test_corruption_is_not_executed_or_silently_overwritten(self):
        target = bootstrap.prepare(self.source, self.cache)
        (target / 'AppRun').write_text('changed')
        with self.assertRaisesRegex(RuntimeError, 'damaged'):
            bootstrap.prepare(self.source, self.cache)
        self.assertEqual((target / 'AppRun').read_text(), 'changed')

    def test_shared_cache_and_symlink_lock_are_rejected(self):
        self.cache.mkdir(mode=0o755)
        with self.assertRaisesRegex(RuntimeError, 'private directory'):
            bootstrap.prepare(self.source, self.cache)
        self.cache.chmod(0o700)
        (self.cache / '.lock').symlink_to(self.source / 'AppRun')
        with self.assertRaises(OSError):
            bootstrap.prepare(self.source, self.cache)
        self.assertEqual((self.source / 'AppRun').read_text(), 'test launcher')

    def test_manifest_cannot_escape_runtime(self):
        with self.assertRaisesRegex(ValueError, 'Unsafe'):
            bootstrap.valid(self.source, {'../escape': 'abc'})
