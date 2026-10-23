"""All shipped locales in the real Rust-backed Qt Quick application."""
import os
from pathlib import Path
import subprocess
import sys
import unittest


@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_UI_TESTS') == '1', 'Enable Qt UI tests')
class LocalizedUiTests(unittest.TestCase):
    def test_all_locales_in_native_window(self):
        root = Path(__file__).resolve().parents[1]
        target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'build/native-qt-target'))
        binary = Path(os.environ.get('NEUTRONDRIVE_NATIVE_EXECUTABLE', target / 'debug/neutrondrive-qt'))
        self.assertTrue(binary.is_file(), 'Build the Rust Qt frontend before enabling UI tests')
        result = subprocess.run([
            'dbus-run-session', '--config-file=' + str(root / 'desktop/tests/session.conf'), '--',
            sys.executable, str(root / 'desktop/tests/check-window-localization.py'),
            str(binary), str(root / 'src/neutrondrive/locales'),
        ], capture_output=True, text=True, timeout=700)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('NATIVE_MAIN_WINDOW_ALL_LANGUAGES_AND_ENGLISH_FALLBACK_OK', result.stdout)
