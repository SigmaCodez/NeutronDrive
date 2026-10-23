"""Real QML view tests against Rust controllers, not the retired PySide adapter.

Model/worker contracts are tested in desktop/src/{files,preferences,updates,
update_installer,instance}.rs and the specialized native QML checks. See
NATIVE-UI-PARITY.md for the mapping from the former Python controller tests.
"""
import os
import unittest
from native_ui_support import check


@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_UI_TESTS') == '1', 'Enable Qt UI tests')
class NativeUiTests(unittest.TestCase):
    def test_actual_window_pages_themes_selection_and_double_click(self):
        output = check('--check-window')
        self.assertIn('NATIVE_KEYBOARD_AND_MOUSE_OK', output)
        self.assertIn('NATIVE_MAIN_WINDOW_OK', output)

    def test_literal_prompt_and_clipboard_through_native_sdk_worker(self):
        output = check('--check-window-fixture', fixture=True)
        self.assertIn('NATIVE_PROMPT_AND_CLIPBOARD_OK', output)
        self.assertIn('NATIVE_WINDOW_LITERAL_SDK_ARGUMENT_OK', output)
