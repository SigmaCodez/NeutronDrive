"""Portable shell entrypoints must preserve arguments without host utilities.

Cache integrity, concurrency and corruption handling are tested in Rust's
appimage module; real re-exec is covered by check-appimage-bootstrap.py.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

class PortableLauncherTests(unittest.TestCase):
    def test_launchers_use_native_bootstrap_and_preserve_literal_arguments(self):
        with tempfile.TemporaryDirectory(prefix='nd image ') as temporary:
            root = Path(temporary)
            (root / 'usr/bin').mkdir(parents=True)
            (root / 'usr/lib/neutrondrive/bin').mkdir(parents=True)
            scripts = Path(__file__).resolve().parents[1] / 'packaging/appimage'
            for name, target in [('AppRun', root / 'AppRun'), ('neutrondrive', root / 'usr/bin/neutrondrive')]:
                shutil.copyfile(scripts / name, target)
                target.chmod(0o755)
            native = root / 'usr/lib/neutrondrive/bin/neutrondrive-qt'
            native.write_text('#!/bin/sh\nprintf "%s\\n" "$1" "$2" "$3" "$4" "$APPDIR" "$QT_PLUGIN_PATH"\n')
            native.chmod(0o755)
            for launcher in (root / 'AppRun', root / 'usr/bin/neutrondrive'):
                with self.subTest(launcher=launcher.name):
                    result = subprocess.run([str(launcher), 'a b', '$(never-run)'], env=dict(os.environ, PATH='/no-host-tools'), capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.splitlines(), ['--appimage-bootstrap', str(root), 'a b', '$(never-run)', str(root), str(root / 'usr/lib/neutrondrive/qt/plugins')])
