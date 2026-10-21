"""Exercise every shipped locale in the actual Qt Quick view with demo data."""
import os
from pathlib import Path
import sys
import time
import unittest
from unittest import mock

@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_UI_TESTS')=='1','Enable Qt UI tests')
class LocalizedUiTests(unittest.TestCase):
    def test_all_locales_preserve_commands_names_and_dialog_values(self):
        from PySide6.QtCore import QObject, QMetaObject
        from neutrondrive.app import Application
        from neutrondrive.demo import DemoBackend
        from neutrondrive.i18n import SUPPORTED, tr
        errors=[]
        def exception(kind,value,tb):errors.append(str(value))
        with mock.patch.object(sys,'excepthook',exception):
            for code in (*SUPPORTED,'ja-JP'):
                with self.subTest(code=code), mock.patch.dict(os.environ,{'LC_ALL':code.replace('-','_')+'.UTF-8','LANGUAGE':''}):
                    app=Application(DemoBackend(),demo=True)
                    try:
                        app.activate()
                        deadline=time.monotonic()+10
                        while app.busy and time.monotonic()<deadline:
                            app.qt.processEvents();time.sleep(.005)
                        self.assertFalse(app.busy)
                        self.assertFalse(errors,errors)
                        self.assertEqual(app.language,'en' if code=='ja-JP' else code)
                        self.assertEqual(app.fields['files_title'].text(),tr('Meine Dateien'))
                        self.assertEqual([n.name for n in app.visible_nodes],[v['name'] for v in app.fileEntries])
                        for node,entry in zip(app.visible_nodes,app.fileEntries):
                            self.assertEqual(entry['isFolder'],node.type=='folder')
                        self.assertEqual(set(app.action_buttons),{'Download','Umbenennen','Verschieben','Freigabe','Papierkorb','Wiederherstellen'})
                        for page in ('background','ondemand','local-trash','activity','settings','files'):
                            app.show_page(page);app.qt.processEvents()
                            self.assertIsNotNone(app.window.findChild(QObject,'page-'+page))
                        selected=[]
                        original='Meine Dateien {p0} 中文.txt'
                        app.prompt(tr('Umbenennen'),tr('Neuer Name'),original,selected.append)
                        app.qt.processEvents()
                        self.assertEqual(app.dialog_state['initial'],original)
                        modal=app.window.findChild(QObject,'confirmationDialog')
                        self.assertTrue(modal.property('visible'))
                        self.assertEqual(app.window.findChild(QObject,'promptInput').property('text'),original)
                        token=app.dialog_state['token']
                        QMetaObject.invokeMethod(modal,'close')
                        app.answer_dialog(token,True,original)
                        self.assertEqual(selected,[original])
                        if code in {'en','de','fr','ru','zh-CN','zh-TW'} and os.environ.get('NEUTRONDRIVE_I18N_SCREENSHOTS'):
                            app.show_page('settings')
                            for _ in range(10):app.qt.processEvents();time.sleep(.01)
                            folder=Path(os.environ['NEUTRONDRIVE_I18N_SCREENSHOTS']);folder.mkdir(parents=True,exist_ok=True)
                            self.assertTrue(app.window.grabWindow().save(str(folder/(code+'.png'))))
                        self.assertFalse(errors,errors)
                    finally:
                        app.closing=True
                        app.service_timer.stop();app.portal_timer.stop()
                        app.executor.shutdown(wait=True,cancel_futures=True)
                        app.status_executor.shutdown(wait=True,cancel_futures=True)
                        app.qt.processEvents()
                        if app.window:app.window.close()
                        app.dispose_view()
