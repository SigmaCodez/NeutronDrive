"""Persistent AppImage cache must not follow links or reuse corrupted code."""
import importlib.util
import json
import os
import shutil
import subprocess
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

    def test_launchers_need_no_host_utilities_before_resetting_runtime_paths(self):
        # Host coreutils may require a newer libsystemd than the portable runtime.
        # Simulate that boundary by providing no external utilities in PATH.
        root = self.base / 'portable path with spaces'
        (root / 'usr/bin').mkdir(parents=True)
        scripts = Path(__file__).resolve().parents[1] / 'packaging/appimage'
        for name, target in [('AppRun', root / 'AppRun'), ('neutrondrive', root / 'usr/bin/neutrondrive')]:
            shutil.copyfile(scripts / name, target)
            target.chmod(0o755)
        interpreter = root / 'usr/bin/python3.10'
        interpreter.write_text('#!/bin/sh\nprintf "launcher-ok:%s\\n" "$1"\n')
        interpreter.chmod(0o755)
        for launcher in (root / 'AppRun', root / 'usr/bin/neutrondrive'):
            with self.subTest(launcher=launcher.name):
                result = subprocess.run([str(launcher), '--help'], env=dict(os.environ, PATH='/no-host-tools'), capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), 'launcher-ok:' + str(root / 'usr/lib/neutrondrive/appimage-bootstrap.py'))
