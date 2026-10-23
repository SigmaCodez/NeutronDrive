"""Native Qt appearance bindings; actual portal messages are covered in Rust.

core/tests/desktop_bus.rs checks initial replies, notifications and the stale-read
race on a private bus. scripts/check-native-qt runs those contracts together with
this Qt boundary. No Python application or PySide binding remains in this test.
"""
import os
import unittest
from native_ui_support import check


@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_UI_TESTS') == '1', 'Enable Qt UI tests')
class NativeAppearanceTests(unittest.TestCase):
    def test_reactive_manual_system_fallback_and_watcher_shutdown(self):
        self.assertIn('NATIVE_APPEARANCE_OK', check('--check-appearance'))
