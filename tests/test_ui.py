"""Opt-in GTK smoke and interaction tests with local read-only demo fixtures.

Run with a virtual display, for example:
    NEUTRONDRIVE_UI_TESTS=1 xvfb-run -a python3 -m unittest discover -s tests -p test_ui.py -v

Ordinary unit-test discovery neither imports PyGObject nor requires a display.
"""
from __future__ import annotations

from dataclasses import replace
import os
from pathlib import Path
import sys
import threading
import time
import traceback
from types import SimpleNamespace
import unittest
from unittest import mock


ENABLED = os.environ.get("NEUTRONDRIVE_UI_TESTS") == "1"


@unittest.skipUnless(ENABLED, "Set NEUTRONDRIVE_UI_TESTS=1 and provide a GTK display")
class GtkApplicationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
        import gi
        gi.require_version("Gtk", "4.0")
        from gi.repository import Gio, GLib, Gtk
        from neutrondrive.app import Application
        from neutrondrive.demo import DemoBackend

        if not Gtk.init_check():
            raise AssertionError("GTK display unavailable; run opt-in UI tests under xvfb-run or Broadway")
        cls.GLib = GLib
        cls.callback_errors = []
        # PyGObject can otherwise print callback exceptions while the unittest
        # appears to pass. Retain them and fail at each event-loop boundary.
        cls.exception_hook = mock.patch.object(
            sys, "excepthook",
            lambda kind, value, tb: cls.callback_errors.append("".join(traceback.format_exception(kind, value, tb))),
        )
        cls.exception_hook.start()
        cls.addClassCleanup(cls.exception_hook.stop)
        cls.app = Application(DemoBackend(), demo=True)
        cls.app.set_flags(cls.app.get_flags() | Gio.ApplicationFlags.NON_UNIQUE)
        cls.addClassCleanup(cls._close_application)
        if not cls.app.register(None):
            raise AssertionError("The isolated GTK test application failed to register")
        cls.app.activate()
        cls._check_callback_errors()
        if cls.app.window is None or not hasattr(cls.app, "stack"):
            raise AssertionError("do_activate did not construct the application window and page stack")
        cls._pump_until(lambda: not cls.app.busy)
        if len(cls.app.nodes) != 6:
            raise AssertionError("The initial read-only demo directory did not load completely")

    @classmethod
    def _check_callback_errors(cls):
        if cls.callback_errors:
            raise AssertionError("GTK callback raised an exception:\n" + "\n".join(cls.callback_errors))

    @classmethod
    def _pump_until(cls, predicate, timeout=4):
        deadline = time.monotonic() + timeout
        context = cls.GLib.MainContext.default()
        while time.monotonic() < deadline:
            # A continuously pending GTK source must not defeat the deadline.
            for _ in range(32):
                if not context.pending():
                    break
                context.iteration(False)
            cls._check_callback_errors()
            if predicate():
                return
            time.sleep(0.005)
        raise AssertionError("GTK did not reach the expected state within the bounded event-loop wait")

    @classmethod
    def _close_application(cls):
        app = getattr(cls, "app", None)
        if app is None:
            return
        # Test work is always bounded and uses DemoBackend, never real transfers.
        app.executor.shutdown(wait=True, cancel_futures=True)
        context = cls.GLib.MainContext.default()
        for _ in range(64):
            if not context.pending():
                break
            context.iteration(False)
        if app.window is not None:
            app.window.destroy()
        app.quit()
        cls._check_callback_errors()

    def setUp(self):
        self._pump_until(lambda: not self.app.busy)
        self.app.invalidate_plan()
        self.app.navigate_root("Meine Dateien", "/my-files")
        self._pump_until(lambda: not self.app.busy and len(self.app.nodes) == 6)
        self.app.search.set_text("")
        self._pump_until(lambda: len(self.rows()) == 6)

    def tearDown(self):
        self._check_callback_errors()

    def rows(self):
        rows = []
        index = 0
        while (row := self.app.file_list.get_row_at_index(index)) is not None:
            rows.append(row)
            index += 1
        return rows

    def test_window_pages_and_initial_demo_rows_are_complete(self):
        self.assertTrue(self.app.window.get_visible())
        self.assertIn("DEMO", self.app.window.get_title())
        self.assertEqual(len(self.rows()), 6)
        self.assertEqual(self.app.path, "/my-files")
        for page in ("files", "sync", "activity", "settings"):
            with self.subTest(page=page):
                self.assertIsNotNone(self.app.stack.get_child_by_name(page))
                self.app.show_page(page)
                self.assertEqual(self.app.stack.get_visible_child_name(), page)
        self.assertFalse(self.app.execute_button.get_sensitive())

    def test_folder_navigation_empty_state_and_breadcrumb_return(self):
        folder = next(row for row in self.rows() if row.node.type == "folder")
        expected_path = folder.node.path
        self.app.file_list.emit("row-activated", folder)
        self._pump_until(lambda: not self.app.busy and self.app.path == expected_path)
        self.assertEqual(self.rows(), [])
        self.assertEqual(len(self.app.crumbs), 2)
        self.assertIn("leer", self.app.empty_label.get_text())
        self.app.go_crumb(0)
        self._pump_until(lambda: not self.app.busy and len(self.rows()) == 6)
        self.assertEqual(self.app.path, "/my-files")
        self.assertEqual(len(self.app.crumbs), 1)
        self.app.navigate_root("Computer", "/devices")
        self._pump_until(lambda: not self.app.busy)
        self.assertEqual(self.app.path, "/devices")
        self.assertEqual(self.rows(), [])
        self.assertFalse(self.app.upload_button.get_sensitive())

    def test_search_filters_visible_rows_without_changing_directory(self):
        self.app.search.set_text("BERGSEE")
        self._pump_until(lambda: len(self.rows()) == 1)
        self.assertEqual(self.rows()[0].node.name, "Bergsee.jpg")
        self.assertEqual(len(self.app.nodes), 6)
        self.assertEqual(self.app.path, "/my-files")
        self.app.search.set_text("nothing-matches-this-test")
        self._pump_until(lambda: len(self.rows()) == 0)
        self.assertIn("Keine passenden", self.app.empty_label.get_text())
        self.app.search.set_text("")
        self._pump_until(lambda: len(self.rows()) == 6)

    def test_selection_respects_unsafe_metadata_and_trash_actions(self):
        file_row = next(row for row in self.rows() if row.node.type == "file")
        node = file_row.node
        self.app.file_list.select_row(file_row)
        self.assertTrue(self.app.action_buttons["Download"].get_sensitive())
        self.app.loaded([replace(node, safe=False)])
        self.app.file_list.select_row(self.rows()[0])
        self.assertTrue(all(not item.get_sensitive() for item in self.app.action_buttons.values()))

        self.app.navigate_root("Papierkorb", "/trash")
        self._pump_until(lambda: not self.app.busy)
        self.app.loaded([replace(node, path="/trash/" + node.name)])
        self.app.file_list.select_row(self.rows()[0])
        restore = self.app.action_buttons["Wiederherstellen"]
        self.assertTrue(restore.get_visible())
        self.assertTrue(restore.get_sensitive())
        for name in ("Download", "Freigabe"):
            with self.subTest(action=name):
                self.assertFalse(self.app.action_buttons[name].get_sensitive())
                self.assertFalse(self.app.action_buttons[name].get_visible())
        self.assertFalse(self.app.upload_button.get_sensitive())
        self.assertFalse(self.app.mkdir_button.get_sensitive())

    def test_busy_state_disables_existing_sync_plan_until_new_preview(self):
        plan = SimpleNamespace(actions=[], conflicts=[], skipped=[], summary=lambda: "Local UI test plan")
        self.app.sync_preview(plan)
        self.assertTrue(self.app.execute_button.get_sensitive())
        release = threading.Event()
        self.app.submit("UI-Test", lambda: release.wait(2))
        try:
            self.assertTrue(self.app.busy)
            self.assertFalse(self.app.execute_button.get_sensitive())
            self.assertFalse(self.app.plan_button.get_sensitive())
            self.assertFalse(self.app.remote_entry.get_sensitive())
            self.app.navigate_root("Papierkorb", "/trash")
            self.assertEqual(self.app.path, "/my-files")
        finally:
            release.set()
            self._pump_until(lambda: not self.app.busy)
        self.assertFalse(self.app.execute_button.get_sensitive())
        self.assertTrue(self.app.plan_button.get_sensitive())
        self.assertTrue(self.app.remote_entry.get_sensitive())


if __name__ == "__main__":
    unittest.main()
