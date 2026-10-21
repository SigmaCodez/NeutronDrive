"""Qt Quick interface and asynchronous controller for the guarded backend adapters."""
from __future__ import annotations
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
import os
from pathlib import Path
import sys
from PySide6.QtCore import QObject, Qt, QTimer, Signal, Slot, QUrl, Property, QCoreApplication, QEvent, QLocale, QDate
from PySide6.QtGui import QGuiApplication, QDesktopServices
from PySide6.QtQml import QQmlApplicationEngine
from PySide6.QtQuick import QQuickWindow
from . import APP_ID, __version__
from .backend import BackendError, Cancelled, data_dir
from .sync import SyncError
from .i18n import tr, configure, language, diagnostic


class Field:
    """Controller form state, independent of any graphical widget."""
    def __init__(self, owner, value="", *, enabled=True, visible=True):
        self.owner, self.value, self.enabled, self.visible = owner, value, enabled, visible
        self.on_change = None
    def text(self): return str(self.value)
    def setText(self, value):
        if self.value == value: return
        self.value = value
        if self.on_change: self.on_change()
        self.owner.changed.emit()
    def clear(self): self.setText("")
    def setPlainText(self, value): self.setText(value)
    def toPlainText(self): return self.text()
    def setEnabled(self, value): self.enabled = bool(value); self.owner.changed.emit()
    def isEnabled(self): return self.enabled
    def setVisible(self, value): self.visible = bool(value); self.owner.changed.emit()
    def isVisible(self): return self.visible
    def isChecked(self): return bool(self.value)
    def setChecked(self, value): self.setText(bool(value))
    def show(self): self.setVisible(True)
    def hide(self): self.setVisible(False)
    def snapshot(self): return dict(value=self.value, enabled=self.enabled, visible=self.visible)

def size_text(size):
    if size is None: return "—"
    value = float(size)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if value < 1024 or unit == "TB":
            return QLocale(language()).toString(value, "f", 0 if unit == "B" else 1) + " " + unit
        value /= 1024


def date_text(value):
    try:
        date = datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone().date()
        return QLocale(language()).toString(QDate(date.year, date.month, date.day), QLocale.FormatType.ShortFormat)
    except (ValueError, AttributeError): return "—"


class Dispatcher(QObject):
    ready = Signal(object, object)

    def __init__(self):
        super().__init__()
        self.ready.connect(self.invoke, Qt.ConnectionType.QueuedConnection)

    @Slot(object, object)
    def invoke(self, callback, arguments):
        callback(*arguments)


class Application(QObject):
    changed = Signal()
    filesChanged = Signal()
    dialogRequested = Signal("QVariantMap")

    def __init__(self, backend, *, demo=False):
        self.qt = QGuiApplication.instance() or QGuiApplication([sys.argv[0]])
        super().__init__()
        from PySide6.QtCore import QTranslator, QLibraryInfo
        configure(QLocale.system().uiLanguages())
        self._qt_translators = []
        for domain in ('qtbase', 'qtdeclarative'):
            translator = QTranslator(self.qt)
            if translator.load(QLocale(language()), domain, '_', QLibraryInfo.path(QLibraryInfo.LibraryPath.TranslationsPath)):
                self.qt.installTranslator(translator)
                self._qt_translators.append(translator)
        from .appearance import SystemAppearance
        self._appearance = SystemAppearance(self.qt, self)
        self.qt.setApplicationName("NeutronDrive")
        self.qt.setDesktopFileName(APP_ID)
        if demo:
            from .demo import DemoBackend
            backend = DemoBackend()
        self.backend, self.demo = backend, demo
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="neutrondrive")
        self.status_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="neutrondrive-status")
        self.dispatcher = Dispatcher()
        self.status_polling = self.busy = self.closing = False
        self.window = None
        self.nodes, self.records = [], []
        self.path = "/my-files"
        self.crumbs = [(tr('Meine Dateien'), self.path)]
        self.current_page = "files"
        self.pages = {name: name for name in ("files", "background", "ondemand", "local-trash", "activity", "settings")}
        self.fields, self.action_buttons = {}, {}
        self.visible_nodes, self.selection = [], set()
        self.focus_index = 0
        self.jobs, self.trash_entries = [], []
        self.notice_error = False
        self.dialog_state, self.dialog_callback = {}, None
        self.dialog_serial = 0
        self.engine = None
        self._build_state()
        self.login_url = None
        self.service_timer = QTimer(self)
        self.service_timer.timeout.connect(self.poll_services)
        # Existing desktop portals use Gio asynchronously. Pump its context on
        # the Qt GUI thread; do not rely on Qt's optional GLib dispatcher build.
        self.portal_timer = QTimer(self)
        self.portal_timer.timeout.connect(self._pump_portal)

    @Property(str, constant=True)
    def language(self):
        return language()

    @Slot(str, result=str)
    def translate(self, source):
        return tr(source)

    @Property(QObject, constant=True)
    def appearance(self):
        return self._appearance

    def dispatch(self, callback, *arguments):
        self.dispatcher.ready.emit(callback, arguments)

    def _pump_portal(self):
        if 'gi.repository.GLib' not in sys.modules: return
        from gi.repository import GLib
        context = GLib.MainContext.default()
        for _ in range(16):
            if not context.pending(): break
            context.iteration(False)

    def activate(self):
        if self.window is None:
            self.engine = QQmlApplicationEngine(self)
            self.engine.rootContext().setContextProperty("controller", self)
            self.engine.load(QUrl.fromLocalFile(str(Path(__file__).with_name("qml") / "Main.qml")))
            if not self.engine.rootObjects():
                raise RuntimeError(tr('Qt Quick-Oberfläche konnte nicht geladen werden.'))
            self.window = self.engine.rootObjects()[0]
            # The list is viewport-limited; finish delegates synchronously so
            # rapid page changes cannot invalidate pending delegate contexts.
            self.engine.setIncubationController(None)
            self.service_timer.start(3000)
            self.portal_timer.start(50)
            if self.backend.available: self.refresh()
            else:
                self.show_page("settings")
                self.message(tr('Das Proton-SDK-Backend wurde nicht gefunden.'), error=True)
        self.window.show()
        self.window.raise_()
        self.window.requestActivate()

    def run(self, arguments=None):
        from .qt_instance import WindowInstance
        scope = "demo" if self.demo else "flatpak" if os.environ.get("FLATPAK_ID") else "snap" if os.environ.get("SNAP") else "native"
        instance = WindowInstance(data_dir() / ("qt-window-" + scope), self.activate)
        try:
            if not instance.claim(): return 0
            self.activate()
            return self.qt.exec()
        finally:
            instance.close()
            self.dispose_view()
            self.executor.shutdown(wait=False,cancel_futures=True)
            self.status_executor.shutdown(wait=False,cancel_futures=True)

    def dispose_view(self):
        # Destroy the QML context while its controller still exists.
        if self.engine is not None:
            self.engine.deleteLater()
            QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)
            self.engine = None
            self.window = None
            self._appearance.close()
            for translator in self._qt_translators:
                self.qt.removeTranslator(translator)
            self._qt_translators.clear()

    def quit(self):
        self.service_timer.stop()
        self.portal_timer.stop()
        self.qt.quit()

    def _build_state(self):
        defaults = dict(search="", files_title=tr('Meine Dateien'), empty_label=tr('Noch keine Dateien geladen.'), count_label="",
            notice_label="", login_button="", status_label=tr('Bereit'), spinner="", cancel_button="", file_list="",
            upload_button="", folder_upload_button="", mkdir_button="", refresh_button="",
            background_local_label=tr('Kein lokaler Ordner gewählt'), background_remote="/my-files", background_add_button="",
            background_status=tr('Noch keine Hintergrund-Aufträge geladen.'), mount_label=tr('Kein Einhängeordner gewählt'),
            mount_remote="/my-files", mount_writable=False, mount_offline=False, mount_button="",
            mount_status=tr('Noch kein Online-Laufwerk verbunden.'), cache_path="",
            local_trash_status=tr('Noch keine Einträge geladen.'), local_trash_empty=tr('Keine registrierten Papierkorbeinträge.'),
            backend_label=tr("Demo — keine Cloud-Verbindung") if self.demo else self.backend.executable or tr('Backend nicht installiert'))
        for name, value in defaults.items():
            field = Field(self, value)
            self.fields[name] = field
            setattr(self, name, field)
        self.background_local = self.mount_local = None
        for name in ("notice_label", "login_button", "spinner", "cancel_button"): self.fields[name].visible = False
        self.background_add_button.enabled = not self.demo
        self.mount_button.enabled = not self.demo and not os.environ.get("SNAP")
        for name in ("Download", "Umbenennen", "Verschieben", "Freigabe", "Papierkorb", "Wiederherstellen"):
            self.action_buttons[name] = Field(self, enabled=False)
        self.search.on_change = self.render_nodes

    @Property("QVariantMap", notify=changed)
    def state(self):
        return dict(fields={name: field.snapshot() for name, field in self.fields.items()},
            actions={name: field.snapshot() for name, field in self.action_buttons.items()},
            focusIndex=self.focus_index, page=self.current_page, path=self.path, busy=self.busy, demo=self.demo, version=__version__,
            noticeError=self.notice_error, crumbs=[name for name, _ in self.crumbs],
            selection=sorted(self.selection),
            jobs=self.jobs, trash=self.trash_entries, activity=[dict(time=t,title=n,status=s) for t,n,s in self.records[:100]])

    @Property("QVariantList", notify=filesChanged)
    def fileEntries(self):
        # Keep the ListView model stable when selection/status changes. Replacing
        # its model would destroy focused/pressed delegates between mouse clicks.
        return [dict(name=diagnostic(n.name) if not n.safe and n.name in {"[Name nicht entschlüsselbar]", "[Ungültiger Name]"} else n.name,kind=tr('Ordner') if n.type=="folder" else tr('Datei'),isFolder=n.type=="folder",safe=n.safe,
                     size=size_text(n.size) if n.type!="folder" else "—",modified=date_text(n.modified))
                for n in self.visible_nodes]

    @Slot(str, "QVariant")
    def edit(self, name, value):
        if name not in {"search", "background_remote", "mount_remote", "mount_writable", "mount_offline", "cache_path"}: return
        field = self.fields[name]
        if field.enabled: field.setText(value)

    @Slot(str)
    def show_page(self, page):
        if page not in self.pages: return
        self.current_page = page
        self.changed.emit()
        if page == "local-trash": self.refresh_local_trash()
        if page in {"background", "ondemand"}: self.poll_services()

    @Slot(str, str)
    def navigate(self, title, path): self.navigate_root(title, path)

    @Slot(int)
    def crumb(self, index):
        if 0 <= index < len(self.crumbs): self.go_crumb(index)

    def update_breadcrumb(self):
        self.files_title.setText(self.crumbs[-1][0]); self.changed.emit()

    def selected(self): return [n for i,n in enumerate(self.visible_nodes) if i in self.selection]

    @Slot(int, bool)
    def select_row(self, index, extend=False):
        if self.busy or not 0 <= index < len(self.visible_nodes): return
        self.focus_index = index
        if not extend: self.selection = {index}
        elif index in self.selection: self.selection.remove(index)
        else: self.selection.add(index)
        self.update_actions(); self.changed.emit()

    @Slot(int, bool)
    def move_selection(self, delta, extend=False):
        if self.visible_nodes:
            self.select_row(max(0,min(len(self.visible_nodes)-1,self.focus_index+delta)),extend)

    @Slot()
    def select_all(self):
        if self.busy: return
        self.selection=set(range(len(self.visible_nodes)))
        self.update_actions(); self.changed.emit()

    def render_nodes(self):
        query = self.search.text().casefold()
        self.visible_nodes = [n for n in self.nodes if query in n.name.casefold()]
        self.selection.clear()
        self.focus_index = 0
        self.filesChanged.emit()
        self.empty_label.setText(tr('Keine passenden Dateien in diesem Ordner.') if query else tr('Dieser Ordner ist leer.'))
        self.empty_label.setVisible(not self.visible_nodes)
        self.count_label.setText(tr('{p0} von {p1} Einträgen · Doppelklick öffnet Ordner').format(p0=len(self.visible_nodes), p1=len(self.nodes)))
        self.update_actions(); self.changed.emit()

    @Slot(int)
    def row_activated(self, index):
        if self.busy or not 0 <= index < len(self.visible_nodes): return
        node = self.visible_nodes[index]
        if not node.safe: return
        if node.type == "folder" and self.path != "/trash":
            self.path=node.path; self.crumbs.append((node.name,node.path)); self.nodes=[]
            self.search.clear(); self.render_nodes(); self.update_breadcrumb(); self.refresh()
        elif self.path != "/trash":
            self.selection={index}; self.download()

    def message(self, text, *, error=False):
        self.notice_error=error; self.notice_label.setText(diagnostic(str(text))); self.notice_label.show()

    def render_activity(self): self.changed.emit()

    def render_background_status(self, value):
        error=value.get("error") or (value.get("event_source") or {}).get("error")
        self.background_status.setText((tr('Dienst läuft') if value.get("running") else tr('Dienst ist gestoppt')) + (" · " + diagnostic(str(error)) if error else ""))
        names={"idle":tr('Aktuell'),"syncing":tr('Wird synchronisiert'),"paused":tr('Pausiert'),"conflicts":tr('Konflikt'),"conflict":tr('Konflikt'),"blocked":tr('Blockiert'),"offline":tr('Offline'), "waiting":tr("Waiting"), "error":tr("Failed"), "stopped":tr("Dienst ist gestoppt")}
        self.jobs=[dict(id=j["id"],name=j.get("name") or j.get("local_root",tr('Sync-Ordner')),
            detail=str(j.get("local_root",""))+" ↔ "+str(j.get("remote_root","")), paused=j.get("paused",False),
            status=names.get(j.get("state"),str(j.get("state","waiting")))+" · "+diagnostic(str(j.get("error") or ""))+" "+"; ".join(map(diagnostic,j.get("conflicts",[])[:5]))) for j in value.get("jobs",[])]
        self.changed.emit()

    def render_local_trash(self, values):
        self.local_trash_status.setText(tr('{p0} registrierte Einträge').format(p0=len(values)) + (tr(' · Demo ohne Dateizugriff') if self.demo else ""))
        self.local_trash_empty.setVisible(not values)
        self.trash_entries=[dict(state=str(state),id=e.id,name=str(e.original_path),detail={"trashed":tr("In trash"),"prepared":tr("Preparing"),"restoring":tr("Restoring"),"restored":tr("Restored"),"recovery":tr("Recovery required"),"missing":tr("Missing")}.get(e.status,e.status)+" · "+str(e.trash_path),
            error=diagnostic(str(getattr(e,"error","") or "")),restorePath=str(getattr(e,"restore_path","") or ""),
            enabled=not self.demo and e.status in {"trashed","prepared","restoring","recovery"}) for state,e in values]
        self.changed.emit()

    @Slot(str, str)
    def job_action(self, action, ident):
        if action in {"pause","resume","sync","remove"}: self.background_action(action, ident)

    @Slot(int)
    def restore_entry(self, index):
        if 0 <= index < len(self.trash_entries):
            entry=self.trash_entries[index]
            if entry["enabled"]: self.restore_local_trash(entry["state"],entry["id"])

    @Slot(str)
    def action(self, name):
        actions = {"refresh":self.refresh,"upload":lambda:self.choose_upload(False),"uploadFolder":lambda:self.choose_upload(True),
            "mkdir":self.new_folder,"Download":self.download,"Umbenennen":self.rename,"Verschieben":self.move,"Freigabe":self.share,
            "Papierkorb":self.trash,"Wiederherstellen":self.restore,
            "chooseBackground":lambda:self.choose(tr('Sync-Ordner wählen'),folder=True,callback=self.choose_background_local),
            "addJob":self.add_background_job,"start":lambda:self.background_action("start"),"stop":lambda:self.background_action("stop"),
            "chooseMount":lambda:self.choose(tr('Leeren Einhängeordner wählen'),folder=True,callback=self.choose_mount_local),
            "mount":self.mount_drive,"unmount":lambda:self.mount_action("unmount"),"refreshMount":lambda:self.mount_action("refresh"),
            "pin":lambda:self.mount_action("pin",path=self.cache_path.text()),"evict":lambda:self.mount_action("evict",path=self.cache_path.text()),
            "helperLogin":lambda:self.mount_action("login"),"refreshTrash":self.refresh_local_trash,"login":self.login,"openLogin":self.open_login,
            "logout":self.logout,"version":lambda:self.submit(tr('Backend prüfen'),self.backend.version,self.show_version),
            "autostart":lambda:self.change_autostart(True),"noAutostart":lambda:self.change_autostart(False),
            "cli":lambda:self.open_url("https://proton.me/download/drive/cli/index.html"),"project":lambda:self.open_url("https://github.com/SigmaCodez/NeutronDrive"),"web":lambda:self.open_url("https://drive.proton.me"),"cancel":self.backend.cancel}
        if name in self.action_buttons and not self.action_buttons[name].enabled: return
        if name in actions: actions[name]()

    def open_url(self, url):
        if not QDesktopServices.openUrl(QUrl(url)): self.message(tr('Browser konnte nicht geöffnet werden: ')+url.split("?")[0],error=True)

    def _dialog(self, kind, title, description, callback, **options):
        self.dialog_serial += 1
        self.dialog_callback = callback
        self.dialog_state = dict(kind=kind,title=title,description=description,token=self.dialog_serial,**options)
        self.dialogRequested.emit(self.dialog_state)

    @Slot(int, bool, "QVariant")
    def answer_dialog(self, token, accepted, value):
        if token != self.dialog_serial or not self.dialog_state: return
        callback=self.dialog_callback
        self.dialog_callback=None; self.dialog_state={}
        if accepted and callback: callback(value)

    def choose(self, title, *, folder=False, multiple=False, callback):
        def selected(values):
            paths=[Path(QUrl(v).toLocalFile()) for v in values if QUrl(v).isLocalFile()]
            if paths: callback(paths)
        self._dialog("folder" if folder else "files",title,"",selected,multiple=multiple)

    def prompt(self,title,description,initial,callback):
        self._dialog("prompt",title,description,callback,initial=initial)

    def confirm(self,title,description,callback,destructive=False):
        self._dialog("confirm",title,description,lambda _:callback(),destructive=destructive)

    def share_dialog(self,node,value):
        url=((value or {}).get("urlAccess") or {}).get("url","")
        def response(action):
            if action == "copy" and url:
                self.qt.clipboard().setText(url); self.share_dialog(node,value)
            elif action == "revoke" and url:
                self.confirm(tr('Freigabelink deaktivieren?'),tr('Der bisherige Link funktioniert anschließend nicht mehr.'),lambda:self.submit(tr('Freigabelink entfernen'),lambda:self.backend.remove_link(node.path),lambda _:self.message(tr('Freigabelink entfernt.'))))
            elif action == "create" and not url:
                self.submit(tr('Freigabelink erstellen'),lambda:self.backend.create_link(node.path),lambda result:self.share_dialog(node,result))
        self._dialog("share",tr('Freigabe · ')+node.name,tr('Jeder mit diesem Link erhält Lesezugriff. Passwort, Ablaufdatum und Mitgliederverwaltung sind in der Proton-Web-App verfügbar.'),response,url=url)

    @Slot(result=bool)
    def request_close(self): return not self._close()

    def _close(self):
        if self.closing and not self.busy:
            self.service_timer.stop();self.portal_timer.stop()
            self.executor.shutdown(wait=False,cancel_futures=True)
            self.status_executor.shutdown(wait=False,cancel_futures=True)
            return False
        if self.busy:
            def stop():
                self.closing=True;self.backend.cancel();self.status_label.setText(tr('Vorgang wird beendet …'))
            self.confirm(tr('Laufenden Vorgang abbrechen?'),tr('Bereits übertragene Dateien bleiben erhalten. Die App wartet auf das Ende des Backends.'),stop)
            return True
        if os.environ.get('FLATPAK_ID') and not self.demo:
            try:
                if self.background_clients()[1].status().get('running'):
                    self.window.hide();return True
            except (BackendError,SyncError):pass
        self.closing=True
        return self._close()

    def refresh_local_trash(self):
        if self.demo:
            self.render_local_trash([])
            return
        def work():
            from .local_trash import LocalTrash
            state = data_dir() / "local-trash"
            try:
                return [(str(state), entry) for entry in LocalTrash(state).list_entries()] if state.exists() else []
            except (OSError, ValueError) as exc:
                raise SyncError(str(exc)) from exc
        self.submit(tr('Lokalen Papierkorb laden'), work, self.render_local_trash)

    def restore_local_trash(self, state, receipt_id):
        if self.demo:
            self.message(tr('Der Demo-Modus stellt keine Dateien wieder her.'))
            return
        def work():
            from .local_trash import LocalTrash
            def check_cancel():
                if self.backend.cancel_event.is_set():
                    raise Cancelled(tr('Wiederherstellung abgebrochen; erhaltene Daten bleiben im Papierkorb oder Wiederherstellungsziel.'))
            try:
                return LocalTrash(Path(state)).restore(receipt_id, check_cancel=check_cancel)
            except (OSError, ValueError) as exc:
                raise SyncError(str(exc)) from exc
        def done(_):
            self.message(tr('Datei wiederhergestellt; bestehende Dateien wurden nicht überschrieben.'))
            self.refresh_local_trash()
        self.submit(tr('Datei wiederherstellen'), work, done)

    def background_clients(self):
        from .background import Controller, JobStore
        state = data_dir() / "background"
        return JobStore(state), Controller(state, executable=self.backend.executable)

    def choose_background_local(self, paths):
        self.background_local = paths[0]
        self.background_local_label.setText(str(paths[0]))

    def choose_mount_local(self, paths):
        self.mount_local = paths[0]
        self.mount_label.setText(str(paths[0]))

    def add_background_job(self):
        if self.demo:
            self.message(tr('Im Demo-Modus werden keine Hintergrund-Aufträge angelegt.'))
            return
        if not self.background_local:
            self.message(tr('Bitte zuerst den lokalen Sync-Ordner auswählen.'), error=True)
            return
        local, remote = self.background_local, self.background_remote.text()
        def work():
            store, controller = self.background_clients()
            store.add(local, remote)
            controller.start()
            controller.command("reload")
            return controller.status()
        def activate():
            if os.environ.get("FLATPAK_ID"):
                from .desktop_integration import request_portal_background
                def permission(ok):
                    if ok:
                        self.submit(tr('Hintergrund-Sync aktivieren'), work, self.render_background_status)
                    else:
                        self.message(tr('Der Desktop hat Hintergrundbetrieb nicht freigegeben. Es wurde kein Auftrag aktiviert.'), error=True)
                request_portal_background(True, permission)
            else:
                self.submit(tr('Hintergrund-Sync aktivieren'), work, self.render_background_status)
        self.confirm(tr('Automatischen Sync aktivieren?'), tr('Neue und geänderte Dateien werden künftig automatisch übertragen. Nach einem bestätigten gemeinsamen Stand werden Löschungen auf der Gegenseite in den Papierkorb übernommen. Bei gleichzeitigen Änderungen bleiben Dateien als Konflikt erhalten. Flatpak fragt zusätzlich nach Hintergrundbetrieb und Autostart.'), activate)

    def background_action(self, action, job_id=None):
        if self.demo:
            self.message(tr('Der Demo-Modus startet keine Hintergrunddienste.'))
            return
        def work():
            store, controller = self.background_clients()
            if action == "start":
                controller.start()
            elif action == "stop":
                controller.stop()
            elif action == "remove":
                if controller.status().get("running"):
                    controller.command("pause", job_id)
                store.remove(job_id)
                if controller.status().get("running"):
                    controller.command("reload")
            else:
                controller.start()
                controller.command(action, job_id)
            return controller.status()
        self.submit(tr('Hintergrund-Sync'), work, self.render_background_status)

    def mount_drive(self):
        if not self.mount_local:
            self.message(tr('Bitte zuerst einen leeren Einhängeordner auswählen.'), error=True)
            return
        self.mount_action("mount", mountpoint=str(self.mount_local), remote_root=self.mount_remote.text(),
                          writable=self.mount_writable.isChecked(), offline=self.mount_offline.isChecked())

    def mount_action(self, action, **options):
        if self.demo:
            self.message(tr('Im Demo-Modus wird kein Laufwerk verbunden oder verändert.'))
            return
        if action == "mount" and os.environ.get("SNAP"):
            self.message(tr('Online-Laufwerke sind im Snap nicht verfügbar. Bitte das native DEB- oder RPM-Paket verwenden; Dateien durchsuchen und synchronisieren bleibt im Snap möglich.'), error=True)
            return
        from .desktop_service import DesktopClient
        client = DesktopClient(self.backend.executable)
        def done(value):
            self.render_mount_status(value)
            self.message(tr('Laufwerksaktion abgeschlossen.'))
        self.submit(tr('Online-Laufwerk'), lambda: client.request(action, **options), done)

    def render_mount_status(self, value):
        state = tr('Verbunden') if value.get("mounted") else tr('Nicht verbunden')
        path = value.get("mountpoint", "")
        recovery = value.get("pending_recovery", value.get("recovery", []))
        text = state + (" · " + str(path) if path else "")
        if recovery:
            text += tr('\nNicht hochgeladene Änderungen im Wiederherstellungsspeicher: ') + str(recovery)
        if value.get("error"):
            text += "\n" + diagnostic(str(value["error"]))
        self.mount_status.setText(text)
        configuration = value.get("configuration", {})
        if configuration and not self.mount_local:
            self.mount_local = Path(configuration.get("mountpoint", ""))
            self.mount_label.setText(str(self.mount_local))
            self.mount_remote.setText(configuration.get("remote_root", "/my-files"))

    def poll_services(self):
        if self.closing:
            return False
        if self.demo or self.busy or self.status_polling or self.current_page not in {"background", "ondemand"}:
            return True
        self.status_polling = True
        page = self.current_page
        def work():
            if page == "background":
                return self.background_clients()[1].status()
            from .desktop_service import DesktopClient
            return DesktopClient(self.backend.executable).request("status")
        def finish(future):
            self.status_polling = False
            if not self.closing and self.current_page == page:
                try:
                    value = future.result()
                    if page == "background":
                        self.render_background_status(value)
                    else:
                        self.render_mount_status(value)
                except Exception:
                    pass  # Explicit actions show errors; a status probe must not interrupt editing.
            return False
        self.status_executor.submit(work).add_done_callback(lambda f:self.dispatch(finish, f))
        return True

    def change_autostart(self, enabled=True):
        if self.demo:
            return
        import os
        if os.environ.get("FLATPAK_ID"):
            from .desktop_integration import request_portal_background
            request_portal_background(enabled, lambda ok:self.message(tr('Autostart eingerichtet.') if ok else tr('Das Desktop-Portal hat Autostart nicht freigegeben.'), error=not ok))
        else:
            from .desktop_integration import set_autostart
            def work():
                set_autostart(enabled, self.backend.executable)
                return enabled
            self.submit(tr('Autostart'), work, lambda enabled:self.message(tr('Autostart aktiviert.') if enabled else tr('Autostart deaktiviert.')))

    def navigate_root(self, title, path):
        if self.busy:
            self.message(tr('Bitte den laufenden Vorgang beenden oder abbrechen.'))
            return
        self.crumbs = [(title, path)]
        self.path = path
        self.search.setText("")
        self.nodes = []
        self.render_nodes()
        self.show_page("files")
        self.update_breadcrumb()
        self.refresh()

    def go_crumb(self, index):
        if self.busy:
            return
        self.crumbs = self.crumbs[:index + 1]
        self.path = self.crumbs[-1][1]
        self.nodes = []
        self.search.setText("")
        self.render_nodes()
        self.update_breadcrumb()
        self.refresh()

    def refresh(self):
        if self.busy:
            return
        path = self.path
        self.submit(tr('Dateiliste laden'), lambda: self.backend.list_nodes(path), self.loaded)

    def loaded(self, nodes):
        self.nodes = nodes
        self.empty_label.setText(tr('Dieser Ordner ist leer.'))
        self.render_nodes()

    def update_actions(self):
        if not hasattr(self, "action_buttons"):
            return
        nodes = self.selected()
        safe = bool(nodes) and all(n.safe for n in nodes) and not self.busy
        trash = self.path == "/trash"
        for name, widget in self.action_buttons.items():
            widget.setVisible((name == "Wiederherstellen") == trash)
            allowed = safe and (len(nodes) == 1 if name in {"Umbenennen", "Verschieben", "Freigabe"} else True)
            if name in {"Download", "Freigabe"} and trash:
                allowed = False
            widget.setEnabled(allowed)
        writable_folder = self.path.startswith("/my-files") or self.path.count("/") > 1
        for widget in (self.upload_button, self.folder_upload_button, self.mkdir_button):
            widget.setEnabled(writable_folder and not trash and not self.busy)
        self.refresh_button.setEnabled(not self.busy)

    def submit(self, title, work, done=None):
        if self.busy:
            self.message(tr('Es läuft bereits ein Vorgang. Bitte warten oder abbrechen.'))
            return
        self.busy = True
        self.backend.reset_cancel()
        self.spinner.setVisible(True)
        self.cancel_button.setVisible(True)
        self.status_label.setText(title + " …")
        self.file_list.setEnabled(False)
        self.update_actions()
        self.records.insert(0, [datetime.now().strftime("%H:%M:%S"), title, tr('Läuft …')])
        record = self.records[0]
        self.render_activity()
        future = self.executor.submit(work)
        future.add_done_callback(lambda result: self.dispatch(self._completed, result, title, done, record))

    def _completed(self, future, title, done, record):
        self.busy = False
        self.spinner.setVisible(False)
        self.cancel_button.setVisible(False)
        self.login_button.setVisible(False)
        self.login_url = None
        self.file_list.setEnabled(True)
        self.update_actions()
        try:
            value = future.result()
        except Exception as exc:
            text = diagnostic(str(exc)) if isinstance(exc, (BackendError, SyncError)) else tr('Interner Fehler ({p0}). Es wurde kein Erfolg angenommen.').format(p0=type(exc).__name__)
            record[2] = tr('Fehlgeschlagen · ') + text
            self.status_label.setText(title + tr(' · nicht abgeschlossen'))
            self.message(text, error=True)
        else:
            record[2] = tr('Abgeschlossen')
            self.status_label.setText(title + tr(' · abgeschlossen'))
            if isinstance(value, dict) and "transferredItems" in value:
                text = tr('{p0} übertragen · {p1} übersprungen · {p2}').format(p0=value['transferredItems'], p1=value.get('skippedItems', 0), p2=size_text(value.get('transferredBytes', 0)))
                record[2] = text
                self.message(text)
            if done and not self.closing:
                try:
                    done(value)
                except Exception as exc:
                    record[2] = tr('Anzeige fehlgeschlagen')
                    self.message(tr('Ergebnis konnte nicht angezeigt werden ({p0}). Bitte aktualisieren.').format(p0=type(exc).__name__), error=True)
        self.render_activity()
        if self.closing:
            self.window.close()
            self.quit()
        return False

    def open_login(self):
        if self.login_url:
            self.open_url(self.login_url)

    def login(self):
        self.message(tr('Die Anmeldung erfolgt bei Proton im Browser. Diese App ist unabhängig und wird nicht von Proton unterstützt.'))
        def on_url(url):
            self.dispatch(self._login_ready, url)
        self.submit(tr("Anmelden"), lambda: self.backend.login(on_url), lambda _: self.refresh())

    def _login_ready(self, url):
        self.login_url = url
        self.login_button.setVisible(True)
        self.message(tr('Bitte die Anmeldung im geöffneten Proton-Browserfenster abschließen. Falls kein Fenster erscheint, den Anmeldeknopf verwenden.'))
        return False

    def logout(self):
        def work():
            if not self.demo:
                self.background_clients()[1].stop()
                if not os.environ.get("FLATPAK_ID"):
                    from .desktop_service import DesktopClient
                    client = DesktopClient(self.backend.executable)
                    if client.request("status").get("mounted"):
                        client.request("unmount")
            return self.backend.logout()
        self.confirm(tr('Von Proton abmelden?'), tr('Hintergrund-Sync wird gestoppt. Native Online-Laufwerke müssen vorher alle geöffneten Dateien schließen. Dies beendet auch die gemeinsam verwendete native CLI-Sitzung.'),
                     lambda: self.submit(tr('Abmelden'), work, self.logged_out))

    def logged_out(self, _):
        self.nodes = []
        self.render_nodes()
        self.message(tr('Von Proton abgemeldet.'))

    def show_version(self, value):
        self.backend_label.setText(str(value))

    def choose_upload(self, folder):
        target = self.path
        self.choose(tr('Ordner hochladen') if folder else tr('Dateien hochladen'), folder=folder, multiple=not folder,
                    callback=lambda paths: self.submit(tr('Upload'), lambda: self.backend.upload(paths, target), lambda _: self.refresh()))

    def download(self):
        paths = [n.path for n in self.selected()]
        if not paths:
            return
        self.choose(tr('Download-Zielordner wählen'), folder=True,
                    callback=lambda selected: self.submit(tr('Download'), lambda: self.backend.download(paths, selected[0])))

    def new_folder(self):
        target = self.path
        self.prompt(tr('Neuer Ordner'), tr('Name des Ordners'), "", lambda value: self.submit(tr('Ordner erstellen'), lambda: self.backend.mkdir(target, value), lambda _: self.refresh()))

    def rename(self):
        nodes = self.selected()
        if len(nodes) != 1:
            return
        node = nodes[0]
        self.prompt(tr('Umbenennen'), tr('Neuer Name'), node.name, lambda value: self.submit(tr('Umbenennen'), lambda: self.backend.rename(node.path, value), lambda _: self.refresh()))

    def move(self):
        paths = [n.path for n in self.selected()]
        self.prompt(tr('Verschieben'), tr('Cloud-Zielordner (z. B. /my-files/Dokumente)'), "/my-files",
                    lambda value: self.submit(tr('Verschieben'), lambda: self.backend.move(paths, value), lambda _: self.refresh()))

    def trash(self):
        nodes = self.selected()
        if not nodes:
            return
        paths = [n.path for n in nodes]
        self.confirm(tr('In den Papierkorb verschieben?'), tr('{p0} ausgewählte Einträge werden in den Proton-Papierkorb verschoben.').format(p0=len(nodes)),
                     lambda: self.submit(tr('Papierkorb'), lambda: self.backend.trash(paths), lambda _: self.refresh()), destructive=True)

    def restore(self):
        paths = [n.path for n in self.selected()]
        if paths:
            self.submit(tr('Wiederherstellen'), lambda: self.backend.restore(paths), lambda _: self.refresh())

    def share(self):
        nodes = self.selected()
        if len(nodes) == 1:
            self.submit(tr('Freigabestatus laden'), lambda: self.backend.sharing(nodes[0].path), lambda value: self.share_dialog(nodes[0], value))
