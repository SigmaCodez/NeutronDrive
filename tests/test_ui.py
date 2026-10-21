"""Qt interaction tests using bounded workers and isolated synthetic files."""
from dataclasses import replace
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import traceback
from types import SimpleNamespace
import unittest
from unittest import mock

ENABLED = os.environ.get("NEUTRONDRIVE_UI_TESTS") == "1"

@unittest.skipUnless(ENABLED,"Set NEUTRONDRIVE_UI_TESTS=1 and provide a Qt display")
class QtApplicationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from PySide6.QtCore import Qt
        from PySide6.QtCore import QObject
        from PySide6.QtQuick import QQuickWindow
        from neutrondrive.app import Application
        from neutrondrive.demo import DemoBackend
        cls.Qt,cls.QObject,cls.QQuickWindow = Qt,QObject,QQuickWindow
        cls.callback_errors=[]
        cls.exception_hook=mock.patch.object(sys,'excepthook',lambda kind,value,tb:cls.callback_errors.append(''.join(traceback.format_exception(kind,value,tb))))
        cls.exception_hook.start();cls.addClassCleanup(cls.exception_hook.stop)
        cls.app=Application(DemoBackend(),demo=True)
        cls.app.activate();cls.addClassCleanup(cls._close_application)
        cls._pump_until(lambda:not cls.app.busy and len(cls.app.nodes)==6,timeout=20)

    @classmethod
    def _check_callback_errors(cls):
        if cls.callback_errors:raise AssertionError('Qt callback errors: '+ '\n'.join(cls.callback_errors))

    @classmethod
    def _pump_until(cls,predicate,timeout=4):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            cls.app.qt.processEvents();cls._check_callback_errors()
            if predicate():return
            time.sleep(0.005)
        raise AssertionError('Qt did not reach expected state')

    @classmethod
    def _close_application(cls):
        cls.app.closing=True
        cls.app.executor.shutdown(wait=True,cancel_futures=True)
        cls.app.status_executor.shutdown(wait=True,cancel_futures=True)
        cls.app.qt.processEvents();cls.app.window.close();cls.app.dispose_view();cls._check_callback_errors()

    def setUp(self):
        self._pump_until(lambda:not self.app.busy)
        self.app.invalidate_plan();self.app.navigate_root('Meine Dateien','/my-files')
        self._pump_until(lambda:not self.app.busy and len(self.app.nodes)==6)
        self.app.search.clear();self._pump_until(lambda:len(self.rows())==6)

    def tearDown(self):
        from PySide6.QtCore import QMetaObject
        dialog=self.app.window.findChild(self.QObject,"confirmationDialog")
        QMetaObject.invokeMethod(dialog,"close")
        self._pump_until(lambda:not dialog.property("visible"))
        self._check_callback_errors()

    def rows(self):
        return list(range(len(self.app.visible_nodes)))

    def select(self,item):self.app.select_row(item,False)

    def test_window_pages_and_initial_demo_rows_are_complete(self):
        self.assertIsInstance(self.app.window,self.QQuickWindow)
        self.assertTrue(self.app.window.isVisible())
        self.assertIn("DEMO", self.app.window.title())
        self.assertEqual(len(self.rows()), 6)
        self.assertEqual(self.app.path, "/my-files")
        for page in ("files", "sync", "background", "ondemand", "local-trash", "activity", "settings"):
            with self.subTest(page=page):
                self.assertIsNotNone(self.app.pages.get(page))
                self.app.show_page(page)
                self.app.qt.processEvents()
                self.assertEqual(self.app.current_page, page)
        self.assertFalse(self.app.execute_button.isEnabled())

    def test_folder_navigation_empty_state_and_breadcrumb_return(self):
        folder = next(row for row in self.rows() if self.app.visible_nodes[row].type == "folder")
        expected_path = self.app.visible_nodes[folder].path
        self.app.row_activated(folder)
        self._pump_until(lambda: not self.app.busy and self.app.path == expected_path)
        self.assertEqual(self.rows(), [])
        self.assertEqual(len(self.app.crumbs), 2)
        self.assertIn("leer", self.app.empty_label.text())
        self.app.go_crumb(0)
        self._pump_until(lambda: not self.app.busy and len(self.rows()) == 6)
        self.assertEqual(self.app.path, "/my-files")
        self.assertEqual(len(self.app.crumbs), 1)
        self.app.navigate_root("Computer", "/devices")
        self._pump_until(lambda: not self.app.busy)
        self.assertEqual(self.app.path, "/devices")
        self.assertEqual(self.rows(), [])
        self.assertFalse(self.app.upload_button.isEnabled())

    def test_search_filters_visible_rows_without_changing_directory(self):
        self.app.search.setText("BERGSEE")
        self._pump_until(lambda: len(self.rows()) == 1)
        self.assertEqual(self.app.visible_nodes[0].name, "Bergsee.jpg")
        self.assertEqual(len(self.app.nodes), 6)
        self.assertEqual(self.app.path, "/my-files")
        self.app.search.setText("nothing-matches-this-test")
        self._pump_until(lambda: len(self.rows()) == 0)
        self.assertIn("Keine passenden", self.app.empty_label.text())
        self.app.search.setText("")
        self._pump_until(lambda: len(self.rows()) == 6)

    def test_selection_respects_unsafe_metadata_and_trash_actions(self):
        file_row = next(row for row in self.rows() if self.app.visible_nodes[row].type == "file")
        node = self.app.visible_nodes[file_row]
        self.select(file_row)
        self.assertTrue(self.app.action_buttons["Download"].isEnabled())
        self.app.loaded([replace(node, safe=False)])
        self.select(self.rows()[0])
        self.assertTrue(all(not item.isEnabled() for item in self.app.action_buttons.values()))

        self.app.navigate_root("Papierkorb", "/trash")
        self._pump_until(lambda: not self.app.busy)
        self.app.loaded([replace(node, path="/trash/" + node.name)])
        self.select(self.rows()[0])
        restore = self.app.action_buttons["Wiederherstellen"]
        self.assertTrue(restore.isVisible())
        self.assertTrue(restore.isEnabled())
        for name in ("Download", "Freigabe"):
            with self.subTest(action=name):
                self.assertFalse(self.app.action_buttons[name].isEnabled())
                self.assertFalse(self.app.action_buttons[name].isVisible())
        self.assertFalse(self.app.upload_button.isEnabled())
        self.assertFalse(self.app.mkdir_button.isEnabled())

    def test_demo_never_starts_background_jobs_or_mounts(self):
        self.assertFalse(self.app.background_add_button.isEnabled())
        self.assertFalse(self.app.mount_button.isEnabled())
        self.app.background_action("start")
        self.assertIn("Demo", self.app.notice_label.text())
        self.app.mount_action("mount", mountpoint="/tmp/should-not-exist")
        self.assertIn("Demo", self.app.notice_label.text())
        self.assertFalse(self.app.busy)

    def test_background_and_online_pages_allow_vertical_scrolling(self):
        for name in ("background", "ondemand"):
            with self.subTest(page=name):
                self.app.show_page(name)
                self.app.qt.processEvents()
                page=self.app.window.findChild(self.QObject,"page-"+name)
                self.assertIsNotNone(page)
                self.assertGreater(page.property("contentHeight"),0)

    def test_demo_local_trash_never_reads_or_restores_personal_files(self):
        with mock.patch("neutrondrive.local_trash.LocalTrash") as trash:
            self.app.show_page("local-trash")
            self.app.refresh_local_trash()
            self.assertIn("Demo ohne Dateizugriff", self.app.local_trash_status.text())
            self.assertEqual(self.app.state["trash"],[])
            self.app.restore_local_trash("/tmp/not-a-real-registry", "not-a-real-entry")
            self.assertIn("Demo", self.app.notice_label.text())
            trash.assert_not_called()

    def test_local_trash_recovery_can_be_checked_but_missing_entries_are_disabled(self):
        entries = [SimpleNamespace(id=str(i), original_path="/tmp/example.txt", trash_path="/tmp/trash/example.txt", status=status)
                   for i, status in enumerate(("trashed", "recovery", "missing", "restored"))]
        with mock.patch.object(self.app, "demo", False), mock.patch.object(self.app, "restore_local_trash") as restore:
            self.app.render_local_trash([("/tmp/registry", entry) for entry in entries])
            for i in range(4):
                entry = self.app.trash_entries[i]
                self.assertEqual(entry["enabled"], i in {0,1})
                if i == 0: self.app.restore_entry(i)
            restore.assert_called_once_with("/tmp/registry", "0")
        self.app.refresh_local_trash()

    def test_local_restore_reports_collision_and_retries_without_overwriting(self):
        from neutrondrive.local_trash import DIR, LocalTrash, _snapshot
        with tempfile.TemporaryDirectory(prefix="neutrondrive-ui-trash-") as temporary:
            base = Path(temporary)
            root = base / "sync"
            root.mkdir()
            target = root / "synthetic.txt"
            target.write_bytes(b"preserved original")
            service = LocalTrash(base / "state", data_home=base / "data")
            fd = os.open(root, DIR)
            try:
                receipt = service.trash(fd, target.name, _snapshot(fd, target.name)[""], root_path=root)
            finally:
                os.close(fd)
            target.write_bytes(b"concurrent new file")
            with mock.patch.object(self.app, "demo", False), \
                    mock.patch.object(self.app.backend, "cancel_event", threading.Event(), create=True), \
                    mock.patch.object(self.app, "refresh_local_trash") as refresh:
                self.app.restore_local_trash(str(service.state_dir), receipt.id)
                self._pump_until(lambda: not self.app.busy)
                self.assertIn("Fehlgeschlagen", self.app.records[0][2])
                self.assertEqual(target.read_bytes(), b"concurrent new file")
                self.assertEqual(Path(receipt.trash_path).read_bytes(), b"preserved original")
                refresh.assert_not_called()
                target.unlink()  # Resolve only this isolated synthetic collision.
                self.app.restore_local_trash(str(service.state_dir), receipt.id)
                self._pump_until(lambda: not self.app.busy)
                self.assertEqual(target.read_bytes(), b"preserved original")
                self.assertIn("wiederhergestellt", self.app.notice_label.text())
                refresh.assert_called_once()

    def test_snap_explains_and_blocks_mount_without_starting_a_helper(self):
        # Construct the real page without activating another window or starting
        # status polling. DemoBackend still prevents accidental account access.
        app = type(self.app)(self.app.backend, demo=False)
        self.addCleanup(app.executor.shutdown, wait=True, cancel_futures=True)
        self.addCleanup(app.status_executor.shutdown, wait=True, cancel_futures=True)
        with mock.patch.dict(os.environ, {"SNAP": "/snap/neutrondrive/test"}):
            app._build_state()
            self.assertFalse(app.mount_button.isEnabled())
            with mock.patch.object(app, "message") as message, mock.patch.object(app, "submit") as submit:
                app.mount_action("mount", mountpoint="/tmp/not-mounted-by-ui-test")
                message.assert_called_once()
                self.assertIn("im Snap nicht verfügbar", message.call_args.args[0])
                submit.assert_not_called()
        # The restriction belongs to the mount operation, not the account UI.
        self.assertFalse(app.busy)

    def test_background_and_mount_status_show_failures_and_recovery(self):
        self.app.render_background_status({"running":True,"jobs":[{
            "id":"test", "local_root":"/tmp/local", "remote_root":"/my-files/test",
            "state":"conflict", "conflicts":["changed.txt"], "paused":True,
        }]})
        self.assertIn("läuft", self.app.background_status.text())
        self.assertEqual(self.app.jobs[0]["id"],"test")
        self.app.render_mount_status({"mounted":True,"mountpoint":"/tmp/virtual","recovery":["pending.txt"]})
        self.assertIn("Verbunden", self.app.mount_status.text())
        self.assertIn("pending.txt", self.app.mount_status.text())

    def test_busy_state_disables_existing_sync_plan_until_new_preview(self):
        plan = SimpleNamespace(actions=[], conflicts=[], skipped=[], summary=lambda: "Local UI test plan")
        self.app.sync_preview(plan)
        self.assertTrue(self.app.execute_button.isEnabled())
        release = threading.Event()
        self.app.submit("UI-Test", lambda: release.wait(2))
        try:
            self.assertTrue(self.app.busy)
            self.assertFalse(self.app.execute_button.isEnabled())
            self.assertFalse(self.app.plan_button.isEnabled())
            self.assertFalse(self.app.remote_entry.isEnabled())
            self.app.navigate_root("Papierkorb", "/trash")
            self.assertEqual(self.app.path, "/my-files")
        finally:
            release.set()
            self._pump_until(lambda: not self.app.busy)
        self.assertFalse(self.app.execute_button.isEnabled())
        self.assertTrue(self.app.plan_button.isEnabled())
        self.assertTrue(self.app.remote_entry.isEnabled())

    def test_worker_completion_runs_on_qt_thread_and_preserves_errors(self):
        main = threading.get_ident()
        observed = []
        self.app.submit("Thread test", threading.get_ident, lambda ident:observed.append((ident,threading.get_ident())))
        self._pump_until(lambda:not self.app.busy)
        self.assertNotEqual(observed[0][0],main)
        self.assertEqual(observed[0][1],main)
        from neutrondrive.backend import BackendError
        def fail():raise BackendError("Synthetic transfer failure")
        self.app.submit("Failure test",fail)
        self._pump_until(lambda:not self.app.busy)
        self.assertIn("Synthetic transfer failure",self.app.notice_label.text())
        self.assertIn("Fehlgeschlagen",self.app.records[0][2])

    def test_confirmation_cancellation_and_acceptance_are_distinct(self):
        from PySide6.QtTest import QTest
        calls=[]
        self.app.confirm("Synthetic","No real files",lambda:calls.append(True),destructive=True)
        self.app.qt.processEvents()
        # Escape must dismiss the actual QML popup without invoking the action.
        QTest.keyClick(self.app.window,self.Qt.Key.Key_Escape)
        self.app.qt.processEvents();self.assertEqual(calls,[])
        self.app.confirm("Synthetic","No real files",lambda:calls.append(True))
        self.app.qt.processEvents()
        self.click_qml("dialogAccept")
        self.app.qt.processEvents();self.assertEqual(calls,[True])

    def click_qml(self,name):
        from PySide6.QtCore import QMetaObject
        item=self.app.window.findChild(self.QObject,name)
        self.assertIsNotNone(item)
        self.assertTrue(QMetaObject.invokeMethod(item,"clicked"))

    def test_prompt_and_share_clipboard_keep_plain_text(self):
        calls=[]
        self.app.prompt("Rename","Synthetic","old",calls.append)
        self.app.qt.processEvents()
        entry=self.app.window.findChild(self.QObject,"promptInput")
        entry.setProperty("text","new-name.txt")
        self.click_qml("dialogAccept")
        self.app.qt.processEvents();self.assertEqual(calls,["new-name.txt"])
        node=SimpleNamespace(name="<b>literal</b>",path="/my-files/example")
        self.app.share_dialog(node,{"urlAccess":{"url":"https://example.invalid/synthetic"}})
        token=self.app.dialog_serial
        self.app.answer_dialog(token,True,"copy")
        self.assertEqual(self.app.qt.clipboard().text(),"https://example.invalid/synthetic")
        self.app.answer_dialog(self.app.dialog_serial,False,"")
        dialog=self.app.window.findChild(self.QObject,"confirmationDialog")
        from PySide6.QtCore import QMetaObject
        QMetaObject.invokeMethod(dialog,"close")

    def test_close_during_work_waits_for_cancellation(self):
        release=threading.Event()
        with mock.patch.object(self.app.backend,"cancel",side_effect=release.set) as cancel:
            self.app.submit("Synthetic close",lambda:release.wait(2))
            try:
                self.assertTrue(self.app._close())
                self.assertFalse(self.app.closing)
                # Declining close must not cancel or discard the worker.
                self.app.answer_dialog(self.app.dialog_serial,False,"");self.app.qt.processEvents();cancel.assert_not_called()
            finally:release.set();self._pump_until(lambda:not self.app.busy)

    def test_file_picker_local_urls_and_stale_dialog_tokens(self):
        calls=[]
        self.app.choose("Synthetic picker",multiple=True,callback=calls.append)
        self.app.qt.processEvents()
        token=self.app.dialog_serial
        self.app.answer_dialog(token,True,["file:///tmp/example%20name.txt","https://example.invalid/ignored"])
        self.assertEqual(calls,[[Path("/tmp/example name.txt")]])
        self.app.answer_dialog(token,True,["file:///tmp/duplicate"])
        self.assertEqual(len(calls),1)
        from PySide6.QtCore import QMetaObject
        picker=self.app.window.findChild(self.QObject,"filePicker")
        self.assertIsNotNone(picker)
        QMetaObject.invokeMethod(picker,"close")

    def test_qml_file_list_keyboard_selection_survives_model_updates(self):
        from PySide6.QtQuick import QQuickItem
        from PySide6.QtTest import QTest
        self.app.qt.processEvents()
        self.app.window.requestActivate()
        QTest.qWait(50)
        # Loader keeps the previous page alive until deferred deletion. Query
        # its current item instead of a same-named object awaiting destruction.
        page=self.app.window.findChild(self.QObject,"pageLoader").property("item")
        listing=page.findChild(QQuickItem,"fileList")
        self.assertIsNotNone(listing)
        listing.forceActiveFocus()
        self.assertTrue(listing.hasActiveFocus())
        QTest.keyClick(self.app.window,self.Qt.Key.Key_Down)
        self.app.qt.processEvents()
        self.assertEqual(self.app.selection,{1})
        self.assertTrue(listing.hasActiveFocus())
        QTest.keyClick(self.app.window,self.Qt.Key.Key_A,self.Qt.KeyboardModifier.ControlModifier)
        self.app.qt.processEvents()
        self.assertEqual(len(self.app.selected()),6)
        self.assertTrue(self.app.action_buttons["Download"].isEnabled())
        self.assertFalse(self.app.action_buttons["Umbenennen"].isEnabled())

    def test_file_delegates_survive_selection_and_double_click_opens_folder(self):
        from PySide6.QtQuick import QQuickItem
        from PySide6.QtCore import QPointF
        from PySide6.QtTest import QTest
        QTest.qWait(50)
        page=self.app.window.findChild(self.QObject,"pageLoader").property("item")
        index=next(i for i,n in enumerate(self.app.visible_nodes) if n.type=="folder")
        path=self.app.visible_nodes[index].path
        def find_row(item):
            if item.objectName()=="fileRow-"+str(index): return item
            for child in item.childItems():
                found=find_row(child)
                if found is not None:return found
            return None
        row=find_row(page)
        self.assertIsNotNone(row)
        point=row.mapToScene(QPointF(30,25)).toPoint()
        QTest.mouseClick(self.app.window,self.Qt.MouseButton.LeftButton,pos=point)
        self.app.qt.processEvents()
        self.assertEqual(self.app.selection,{index})
        self.assertIs(find_row(page),row)
        QTest.mouseDClick(self.app.window,self.Qt.MouseButton.LeftButton,pos=point)
        self._pump_until(lambda:self.app.path==path and not self.app.busy)

    def test_single_instance_activates_existing_window(self):
        from neutrondrive.qt_instance import WindowInstance
        with tempfile.TemporaryDirectory() as temporary:
            activated=[]
            first=WindowInstance(Path(temporary)/"private",lambda:activated.append(True))
            second=WindowInstance(Path(temporary)/"private",lambda:None)
            try:
                self.assertTrue(first.claim());self.assertFalse(second.claim())
                self._pump_until(lambda:bool(activated))
            finally:second.close();first.close()



if __name__ == "__main__": unittest.main()
