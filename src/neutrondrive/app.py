"""Qt 6 Widgets interface; all transfers and service calls run outside the UI thread."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
import os
from pathlib import Path
import sys

from PySide6.QtCore import QObject, Qt, QTimer, Signal, Slot, QUrl
from PySide6.QtGui import QDesktopServices, QIcon
from PySide6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout,
    QLabel, QPushButton, QLineEdit, QCheckBox, QPlainTextEdit, QStackedWidget, QScrollArea,
    QTreeWidget, QTreeWidgetItem, QAbstractItemView, QHeaderView, QProgressBar,
    QFileDialog, QInputDialog, QMessageBox, QDialog)

from . import APP_ID, __version__
from .backend import BackendError, Cancelled, data_dir
from .sync import SyncError

STYLE = """
QMainWindow, QDialog { background: #111a23; }
QWidget { background: #111a23; color: #e5ecf1; font-size: 13px; }
QLabel { background: transparent; }
QPushButton { background: #22333f; border: 1px solid #364b59; border-radius: 6px; padding: 8px 12px; }
QPushButton:hover { background: #2d4c50; }
QPushButton:checked { background: #244f44; border-color: #67dfbf; }
QPushButton:disabled { color: #75818b; background: #1b252e; }
QPushButton[primary="true"] { background: #67dfbf; color: #092d25; font-weight: bold; }
QPushButton[primary="true"]:disabled { background: #1b252e; color: #75818b; }
QCheckBox::indicator { width: 16px; height: 16px; border: 1px solid #8e9daa; border-radius: 3px; background: #18232e; }
QCheckBox::indicator:checked { background: #67dfbf; border-color: #67dfbf; }
QLineEdit, QPlainTextEdit, QTreeWidget { background: #18232e; border: 1px solid #344652; border-radius: 6px; padding: 7px; }
QTreeWidget::item { padding: 9px 4px; }
QTreeWidget::item:selected { background: #28584c; }
QHeaderView::section { background: #22333f; color: #a2b5c3; border: 0; padding: 8px; }
QScrollArea, QStackedWidget { background: transparent; border: 0; }
QCheckBox { padding: 5px; }
QLabel[role="title"] { font-size: 26px; font-weight: bold; }
QLabel[role="muted"] { color: #a2afba; }
QLabel[role="notice"] { background: #213a3a; border: 1px solid #355654; border-radius: 6px; padding: 10px; }
QLabel[error="true"] { background: #493b25; border-color: #79623e; }
"""


def label(text="", role=""):
    item = QLabel(str(text))
    item.setTextFormat(Qt.TextFormat.PlainText)
    item.setWordWrap(True)
    if role: item.setProperty("role", role)
    return item


def button(text, callback=None, *, primary=False):
    item = QPushButton(text)
    if callback: item.clicked.connect(lambda checked=False: callback())
    if primary: item.setProperty("primary", True)
    return item


def box(vertical=True):
    widget = QWidget()
    layout = QVBoxLayout(widget) if vertical else QHBoxLayout(widget)
    layout.setContentsMargins(0, 0, 0, 0)
    layout.setSpacing(10)
    return widget


def add_row(parent, *widgets):
    row = box(False)
    for widget in widgets: row.layout().addWidget(widget)
    parent.layout().addWidget(row)
    return row


def size_text(size):
    if size is None: return "—"
    value = float(size)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if value < 1024 or unit == "TB":
            return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"
        value /= 1024


def date_text(value):
    try: return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone().strftime("%d.%m.%Y")
    except (ValueError, AttributeError): return "—"


class Dispatcher(QObject):
    ready = Signal(object, object)

    def __init__(self):
        super().__init__()
        self.ready.connect(self.invoke, Qt.ConnectionType.QueuedConnection)

    @Slot(object, object)
    def invoke(self, callback, arguments):
        callback(*arguments)


class Window(QMainWindow):
    def __init__(self, owner):
        super().__init__()
        self.owner = owner

    def closeEvent(self, event):
        if self.owner._close(): event.ignore()
        else: event.accept()


class Application(QObject):
    def __init__(self, backend, *, demo=False):
        self.qt = QApplication.instance() or QApplication([sys.argv[0]])
        super().__init__()
        self.qt.setApplicationName("NeutronDrive")
        self.qt.setDesktopFileName(APP_ID)
        self.qt.setStyle("Fusion")
        self.qt.setStyleSheet(STYLE)
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
        self.crumbs = [("Meine Dateien", self.path)]
        self.current_page = "files"
        self.nav_buttons, self.pages, self.dialogs = {}, {}, []
        self.sync_plan = self.sync_local = self.login_url = None
        self.service_timer = QTimer(self)
        self.service_timer.timeout.connect(self.poll_services)
        # Existing desktop portals use Gio asynchronously. Pump its context on
        # the Qt GUI thread; do not rely on Qt's optional GLib dispatcher build.
        self.portal_timer = QTimer(self)
        self.portal_timer.timeout.connect(self._pump_portal)

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
            self.window = Window(self)
            self.window.setWindowTitle("NeutronDrive" + (" · DEMO" if self.demo else ""))
            self.window.resize(1180, 780)
            self.window.setMinimumSize(820, 580)
            self._build()
            self.service_timer.start(3000)
            self.portal_timer.start(50)
            if self.backend.available: self.refresh()
            else:
                self.show_page("settings")
                self.message("Das Proton-SDK-Backend wurde nicht gefunden.", error=True)
        self.window.show()
        self.window.raise_()
        self.window.activateWindow()

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
            self.executor.shutdown(wait=False,cancel_futures=True)
            self.status_executor.shutdown(wait=False,cancel_futures=True)

    def quit(self):
        self.service_timer.stop()
        self.portal_timer.stop()
        self.qt.quit()

    def _build(self):
        root = box(False)
        root.layout().setContentsMargins(18, 18, 18, 18)
        self.window.setCentralWidget(root)
        sidebar = box()
        sidebar.setFixedWidth(205)
        sidebar.layout().addWidget(label("NeutronDrive", "title"))
        sidebar.layout().addWidget(label("Qt 6 · " + __version__ + (" · DEMO" if self.demo else ""), "muted"))
        for title, path in [("Meine Dateien", "/my-files"), ("Mit mir geteilt", "/shared-with-me"), ("Computer", "/devices"), ("Papierkorb", "/trash")]:
            item = button(title, lambda t=title,p=path: self.navigate_root(t,p))
            item.setCheckable(True)
            self.nav_buttons[path] = item
            sidebar.layout().addWidget(item)
        for title, page in [("Synchronisierung", "sync"), ("Automatischer Sync", "background"), ("Online-Laufwerk", "ondemand"), ("Lokaler Papierkorb", "local-trash"), ("Aktivität", "activity"), ("Einstellungen", "settings")]:
            item = button(title, lambda p=page: self.show_page(p))
            item.setCheckable(True)
            self.nav_buttons[page] = item
            sidebar.layout().addWidget(item)
        sidebar.layout().addStretch()
        sidebar.layout().addWidget(button("Bei Proton anmelden", self.login))
        sidebar.layout().addWidget(button("Proton Drive im Web", lambda:self.open_url("https://drive.proton.me")))
        sidebar.layout().addWidget(label("Unabhängige Open-Source-App\nVerschlüsselung durch Proton SDK", "muted"))
        scroll = QScrollArea(); scroll.setWidgetResizable(True); scroll.setWidget(sidebar)
        scroll.setFixedWidth(225)
        root.layout().addWidget(scroll)
        content = box()
        root.layout().addWidget(content, 1)
        self.notice_label = label("", "notice")
        self.notice_label.hide()
        content.layout().addWidget(self.notice_label)
        self.login_button = button("Anmeldung im Browser öffnen", self.open_login)
        self.login_button.hide(); content.layout().addWidget(self.login_button)
        self.stack = QStackedWidget()
        content.layout().addWidget(self.stack, 1)
        for method in (self._files_page, self._sync_page, self._background_page, self._ondemand_page,
                       self._local_trash_page, self._activity_page, self._settings_page): method()
        self.status_label = label("Bereit", "muted")
        self.spinner = QProgressBar(); self.spinner.setRange(0,0); self.spinner.setMaximumWidth(110); self.spinner.hide()
        self.cancel_button = button("Abbrechen", self.backend.cancel); self.cancel_button.hide()
        add_row(content,self.status_label,self.spinner,self.cancel_button)
        self.show_page("files")

    def page(self, name, title, intro=""):
        page = box()
        page.layout().setContentsMargins(8,8,12,8)
        if title: page.layout().addWidget(label(title,"title"))
        if intro: page.layout().addWidget(label(intro,"muted"))
        scroll = QScrollArea(); scroll.setWidgetResizable(True); scroll.setWidget(page)
        self.pages[name] = scroll
        self.stack.addWidget(scroll)
        return page

    def _files_page(self):
        page = self.page("files", "")
        self.files_title = label("Meine Dateien","title")
        self.search = QLineEdit(); self.search.setPlaceholderText("In diesem Ordner suchen")
        self.search.textChanged.connect(self.render_nodes)
        add_row(page,self.files_title,self.search)
        self.upload_button = button("Dateien hochladen", lambda:self.choose_upload(False), primary=True)
        self.folder_upload_button = button("Ordner hochladen", lambda:self.choose_upload(True))
        self.mkdir_button = button("Neuer Ordner",self.new_folder)
        self.refresh_button = button("Aktualisieren",self.refresh)
        add_row(page,self.upload_button,self.folder_upload_button,self.mkdir_button,self.refresh_button)
        self.breadcrumb = box(False);page.layout().addWidget(self.breadcrumb)
        self.file_list = QTreeWidget(); self.file_list.setRootIsDecorated(False)
        self.file_list.setHeaderLabels(["Name", "Typ", "Größe", "Geändert"])
        self.file_list.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.file_list.header().setSectionResizeMode(0,QHeaderView.ResizeMode.Stretch)
        self.file_list.itemActivated.connect(self.row_activated)
        self.file_list.itemSelectionChanged.connect(self.update_actions)
        page.layout().addWidget(self.file_list,1)
        self.empty_label = label("Noch keine Dateien geladen.","muted");page.layout().addWidget(self.empty_label)
        self.action_buttons = {}
        actions = box(False);page.layout().addWidget(actions)
        for title, action in [("Download",self.download),("Umbenennen",self.rename),("Verschieben",self.move),("Freigabe",self.share),("Papierkorb",self.trash),("Wiederherstellen",self.restore)]:
            item = button(title,action);self.action_buttons[title] = item;actions.layout().addWidget(item)
        self.count_label = label("","muted");page.layout().addWidget(self.count_label)
        self.update_breadcrumb();self.update_actions()

    def _sync_page(self):
        page = self.page("sync","Synchronisierung","Erst Vorschau prüfen, dann abgleichen. Konflikte bleiben erhalten.")
        self.local_label = label("Kein lokaler Ordner gewählt")
        self.local_choose = button("Lokalen Ordner wählen",self.choose_sync_local)
        add_row(page,self.local_label,self.local_choose)
        page.layout().addWidget(label("Cloud-Ordner"))
        self.remote_entry = QLineEdit("/my-files");self.remote_entry.textChanged.connect(self.invalidate_plan)
        page.layout().addWidget(self.remote_entry)
        self.plan_button = button("Vorschau erstellen",self.plan_sync,primary=True)
        self.execute_button = button("Geprüften Plan ausführen",self.execute_sync);self.execute_button.setEnabled(False)
        add_row(page,self.plan_button,self.execute_button)
        page.layout().addWidget(label("Dateilöschungen werden in den Papierkorb übertragen. Ganze Ordnerlöschungen bleiben als Konflikt erhalten. Ersetzte lokale Versionen werden gesichert.","muted"))
        self.sync_text = QPlainTextEdit();self.sync_text.setReadOnly(True)
        self.sync_text.setPlainText("Wähle einen lokalen Ordner und den zugehörigen Cloud-Ordner.")
        page.layout().addWidget(self.sync_text,1)

    def _background_page(self):
        page = self.page("background","Automatischer Sync","Änderungen im Hintergrund abgleichen – auch nach dem Schließen des Fensters.")
        self.background_local = None
        self.background_local_label = label("Kein lokaler Ordner gewählt")
        add_row(page,self.background_local_label,button("Ordner wählen",lambda:self.choose("Sync-Ordner wählen",folder=True,callback=self.choose_background_local)))
        self.background_remote = QLineEdit("/my-files");page.layout().addWidget(self.background_remote)
        self.background_add_button = button("Automatischen Sync aktivieren",self.add_background_job,primary=True)
        self.background_add_button.setEnabled(not self.demo)
        add_row(page,self.background_add_button,button("Dienst starten",lambda:self.background_action("start")),button("Dienst stoppen",lambda:self.background_action("stop")))
        self.background_status = label("Noch keine Hintergrund-Aufträge geladen.","muted");page.layout().addWidget(self.background_status)
        self.background_jobs = box();page.layout().addWidget(self.background_jobs)
        page.layout().addWidget(label("Dateilöschungen nur bei unverändertem Gegenstück; ganze Ordnerlöschungen bleiben Konflikte. Anmeldung und entsperrter Schlüsselbund erforderlich.","muted"))
        page.layout().addStretch()

    def _ondemand_page(self):
        page = self.page("ondemand","Online-Laufwerk","Dateien beim Öffnen laden, offline vorhalten oder lokalen Speicher freigeben.")
        self.mount_local = None
        self.mount_label = label("Kein Einhängeordner gewählt")
        add_row(page,self.mount_label,button("Ordner wählen",lambda:self.choose("Leeren Einhängeordner wählen",folder=True,callback=self.choose_mount_local)))
        self.mount_remote = QLineEdit("/my-files");page.layout().addWidget(self.mount_remote)
        self.mount_writable = QCheckBox("Dateien im Laufwerk bearbeiten und hochladen");page.layout().addWidget(self.mount_writable)
        self.mount_offline = QCheckBox("Offline aus vorhandenem Cache öffnen");page.layout().addWidget(self.mount_offline)
        self.mount_button = button("Laufwerk verbinden",self.mount_drive,primary=True)
        self.mount_button.setEnabled(not self.demo and not os.environ.get("SNAP"))
        if os.environ.get("SNAP"):self.mount_button.setToolTip("Bitte das native DEB- oder RPM-Paket verwenden.")
        add_row(page,self.mount_button,button("Trennen",lambda:self.mount_action("unmount")),button("Aktualisieren",lambda:self.mount_action("refresh")))
        self.mount_status = label("Noch kein Online-Laufwerk verbunden.");page.layout().addWidget(self.mount_status)
        self.cache_path = QLineEdit();self.cache_path.setPlaceholderText("Dateipfad, z. B. Dokumente/Notizen.txt");page.layout().addWidget(self.cache_path)
        add_row(page,button("Offline vorhalten",lambda:self.mount_action("pin",path=self.cache_path.text())),button("Speicher freigeben",lambda:self.mount_action("evict",path=self.cache_path.text())))
        self.mount_notice = label("Für Flatpak zusätzlich den nativen Laufwerkshelfer installieren. Online-Laufwerke sind im Snap derzeit nicht verfügbar; Dateien durchsuchen, übertragen und synchronisieren bleibt möglich. Nicht hochgeladene Änderungen bleiben im Wiederherstellungsspeicher.","muted")
        page.layout().addWidget(self.mount_notice)
        page.layout().addWidget(button("Laufwerkshelfer bei Proton anmelden",lambda:self.mount_action("login")))
        page.layout().addStretch()

    def _local_trash_page(self):
        page = self.page("local-trash","Lokaler Papierkorb","Nach Cloud-Löschung lokal gesicherte Dateien. Wiederherstellen überschreibt keine vorhandenen Dateien.")
        page.layout().addWidget(button("Aktualisieren",self.refresh_local_trash))
        self.local_trash_status = label("Noch keine Einträge geladen.");page.layout().addWidget(self.local_trash_status)
        self.local_trash_empty = label("Keine registrierten Papierkorbeinträge.","muted");page.layout().addWidget(self.local_trash_empty)
        self.local_trash_rows = box();page.layout().addWidget(self.local_trash_rows)
        page.layout().addWidget(label("Bei aktivem Sync wird eine Wiederherstellung erneut synchronisiert. NeutronDrive leert den Papierkorb nicht automatisch.","muted"))
        page.layout().addStretch()

    def _activity_page(self):
        page = self.page("activity","Aktivität","Vorgänge dieser Sitzung · keine dauerhafte Speicherung von Dateinamen")
        self.activity_list = box();page.layout().addWidget(self.activity_list);page.layout().addStretch()

    def _settings_page(self):
        page = self.page("settings","Einstellungen","NeutronDrive " + __version__ + " · Qt 6 · Open Source")
        self.backend_label = label(self.backend.executable or "Backend nicht installiert")
        self.backend_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        page.layout().addWidget(self.backend_label)
        add_row(page,button("Backend prüfen",lambda:self.submit("Backend prüfen",self.backend.version,self.show_version)),button("Anmelden",self.login),button("Abmelden",self.logout))
        page.layout().addWidget(label("Anmeldung ausschließlich auf account.proton.me im Browser. Die Sitzung liegt im Schlüsselbund oder in pass. Native CLI-Sitzungen werden gemeinsam genutzt; Flatpak kann einen eigenen Kontospeicher verwenden.","muted"))
        add_row(page,button("Autostart aktivieren",lambda:self.change_autostart(True)),button("Autostart deaktivieren",lambda:self.change_autostart(False)))
        add_row(page,button("Projekt & Installation",lambda:self.open_url("https://github.com/SigmaCodez/NeutronDrive")),button("Offizielle CLI",lambda:self.open_url("https://proton.me/download/drive/cli/index.html")))
        page.layout().addWidget(label("Alpha: keine vollständige Windows/macOS-Funktionsparität. Versionsverwaltung und Proton Docs/Sheets sind nicht integriert. Der Proton SDK übernimmt Konto, Verschlüsselung und Übertragungen.","muted"))
        page.layout().addStretch()

    @staticmethod
    def clear_box(widget):
        while widget.layout().count():
            item = widget.layout().takeAt(0)
            child = item.widget()
            if child is not None:
                child.setParent(None)
                child.deleteLater()

    def show_page(self, page):
        self.current_page = page
        self.stack.setCurrentWidget(self.pages[page])
        selected = '/' + self.path.split('/')[1] if page == 'files' else page
        for key,item in self.nav_buttons.items():item.setChecked(key == selected)
        if page == 'local-trash':self.refresh_local_trash()
        if page in {'background','ondemand'}:self.poll_services()

    def update_breadcrumb(self):
        self.clear_box(self.breadcrumb)
        for index,(name,_) in enumerate(self.crumbs):
            self.breadcrumb.layout().addWidget(button(name,lambda i=index:self.go_crumb(i)))
        self.breadcrumb.layout().addStretch()
        self.files_title.setText(self.crumbs[-1][0])

    def selected(self):
        return [item.data(0,Qt.ItemDataRole.UserRole) for item in self.file_list.selectedItems()]

    def render_nodes(self):
        self.file_list.clear()
        query = self.search.text().casefold()
        visible = [n for n in self.nodes if query in n.name.casefold()]
        for node in visible:
            kind = "Ordner" if node.type == "folder" else "Datei"
            if not node.safe: kind = "Nicht eindeutig · gesperrt"
            item = QTreeWidgetItem([node.name,kind,size_text(node.size) if node.type != 'folder' else '—',date_text(node.modified)])
            item.setData(0,Qt.ItemDataRole.UserRole,node)
            item.setToolTip(0,node.name)
            item.setIcon(0,QIcon.fromTheme('folder' if node.type == 'folder' else 'text-x-generic'))
            self.file_list.addTopLevelItem(item)
        self.empty_label.setText("Keine passenden Dateien in diesem Ordner." if query else "Dieser Ordner ist leer.")
        self.empty_label.setVisible(not visible)
        self.count_label.setText(f"{len(visible)} von {len(self.nodes)} Einträgen · Doppelklick öffnet Ordner")
        self.update_actions()

    def row_activated(self, item, column=0):
        node = item.data(0,Qt.ItemDataRole.UserRole)
        if self.busy or not node.safe:return
        if node.type == 'folder' and self.path != '/trash':
            self.path = node.path;self.crumbs.append((node.name,node.path));self.nodes=[]
            self.search.clear();self.render_nodes();self.update_breadcrumb();self.refresh()
        elif self.path != '/trash':
            self.file_list.clearSelection();item.setSelected(True);self.download()

    def message(self, text, *, error=False):
        self.notice_label.setText(str(text));self.notice_label.setProperty('error',error)
        self.notice_label.style().unpolish(self.notice_label);self.notice_label.style().polish(self.notice_label)
        self.notice_label.show()

    def render_activity(self):
        self.clear_box(self.activity_list)
        for timestamp,title,status in self.records[:100]:
            self.activity_list.layout().addWidget(label(timestamp + "   " + title + "\n" + status))

    def render_background_status(self, value):
        text = "Dienst läuft" if value.get('running') else "Dienst ist gestoppt"
        error = value.get('error') or (value.get('event_source') or {}).get('error')
        self.background_status.setText(text + (' · '+str(error) if error else ''))
        self.clear_box(self.background_jobs)
        names = {'idle':'Aktuell','syncing':'Wird synchronisiert','paused':'Pausiert','conflicts':'Konflikt','conflict':'Konflikt','blocked':'Blockiert','offline':'Offline'}
        for job in value.get('jobs',[]):
            row = box();self.background_jobs.layout().addWidget(row)
            row.layout().addWidget(label(job.get('name') or job.get('local_root','Sync-Ordner')))
            row.layout().addWidget(label(str(job.get('local_root',''))+' ↔ '+str(job.get('remote_root','')),'muted'))
            state = job.get('state','waiting');detail = names.get(state,state)
            if job.get('error'):detail += '\n'+str(job['error'])
            if job.get('conflicts'):detail += '\nKonflikte: '+'; '.join(map(str,job['conflicts'][:5]))
            row.layout().addWidget(label(detail))
            ident,paused = job['id'],job.get('paused',False)
            add_row(row,button('Fortsetzen' if paused else 'Pausieren',lambda i=ident,p=paused:self.background_action('resume' if p else 'pause',i)),button('Jetzt abgleichen',lambda i=ident:self.background_action('sync',i)),button('Auftrag entfernen',lambda i=ident:self.background_action('remove',i)))

    def render_local_trash(self, values):
        self.clear_box(self.local_trash_rows)
        self.local_trash_status.setText(f"{len(values)} registrierte Einträge" + (" · Demo ohne Dateizugriff" if self.demo else ""))
        self.local_trash_empty.setVisible(not values)
        states = {'trashed':'Im Papierkorb','restored':'Wiederhergestellt','recovery':'Prüfung erforderlich','missing':'Nicht mehr im Papierkorb','prepared':'Vorbereitet','restoring':'Wiederherstellung läuft','aborted':'Abgebrochen'}
        for state,entry in values:
            row = box();self.local_trash_rows.layout().addWidget(row)
            row.layout().addWidget(label(str(entry.original_path)))
            row.layout().addWidget(label(states.get(entry.status,entry.status)+' · '+str(entry.trash_path),'muted'))
            for value in (getattr(entry,'error',''),getattr(entry,'restore_path','')):
                if value:row.layout().addWidget(label(value))
            action = button('Am ursprünglichen Ort wiederherstellen' if entry.status == 'trashed' else 'Wiederherstellung prüfen',lambda s=state,i=entry.id:self.restore_local_trash(s,i))
            action.setEnabled(not self.demo and entry.status in {'trashed','prepared','restoring','recovery'})
            row.layout().addWidget(action)

    def open_url(self,url):
        if not QDesktopServices.openUrl(QUrl(url)):
            self.message('Browser konnte nicht geöffnet werden: '+url.split('?')[0],error=True)

    def _dialog(self, dialog):
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.dialogs.append(dialog)
        dialog.finished.connect(lambda _:self.dialogs.remove(dialog) if dialog in self.dialogs else None)
        dialog.open()
        return dialog

    def choose(self,title,*,folder=False,multiple=False,callback):
        dialog = QFileDialog(self.window,title)
        dialog.setFileMode(QFileDialog.FileMode.Directory if folder else (QFileDialog.FileMode.ExistingFiles if multiple else QFileDialog.FileMode.ExistingFile))
        if folder:dialog.setOption(QFileDialog.Option.ShowDirsOnly)
        def selected():
            paths=[Path(path) for path in dialog.selectedFiles()]
            if paths:callback(paths)
        dialog.accepted.connect(selected)
        self._dialog(dialog)

    def prompt(self,title,description,initial,callback):
        dialog=QInputDialog(self.window);dialog.setWindowTitle(title);dialog.setLabelText(description);dialog.setTextValue(initial)
        dialog.setOkButtonText('Übernehmen');dialog.setCancelButtonText('Abbrechen')
        dialog.textValueSelected.connect(callback);self._dialog(dialog)

    def confirm(self,title,description,callback,destructive=False):
        dialog=QMessageBox(QMessageBox.Icon.Question,title,description,parent=self.window)
        dialog.setTextFormat(Qt.TextFormat.PlainText)
        accept=dialog.addButton('Bestätigen',QMessageBox.ButtonRole.AcceptRole)
        cancel=dialog.addButton('Abbrechen',QMessageBox.ButtonRole.RejectRole)
        dialog.setDefaultButton(cancel);dialog.setEscapeButton(cancel)
        dialog.buttonClicked.connect(lambda clicked:callback() if clicked is accept else None)
        self._dialog(dialog)

    def share_dialog(self,node,value):
        dialog=QDialog(self.window);dialog.setWindowTitle('Freigabe · '+node.name);dialog.resize(560,240)
        layout=QVBoxLayout(dialog);url=((value or {}).get('urlAccess') or {}).get('url','')
        if url:
            layout.addWidget(label('Jeder mit diesem Link kann auf die Datei zugreifen.'))
            entry=QLineEdit(url);entry.setReadOnly(True);layout.addWidget(entry)
            layout.addWidget(button('Link kopieren',lambda:self.qt.clipboard().setText(url)))
            def revoke():
                dialog.close()
                self.confirm('Freigabelink deaktivieren?','Der bisherige Link funktioniert anschließend nicht mehr.',lambda:self.submit('Freigabelink entfernen',lambda:self.backend.remove_link(node.path),lambda _:self.message('Freigabelink entfernt.')))
            layout.addWidget(button('Link deaktivieren',revoke))
        else:
            layout.addWidget(label('Es besteht kein öffentlicher Link. Jeder mit dem erstellten Link erhält Lesezugriff. Passwort, Ablaufdatum und Mitgliederverwaltung stehen in der Proton-Web-App bereit.'))
            def create():
                dialog.close();self.submit('Freigabelink erstellen',lambda:self.backend.create_link(node.path),lambda result:self.share_dialog(node,result))
            layout.addWidget(button('Öffentlichen Leselink erstellen',create))
        layout.addWidget(button('Schließen',dialog.close));self._dialog(dialog)

    def _close(self):
        if self.closing and not self.busy:
            self.service_timer.stop();self.portal_timer.stop()
            self.executor.shutdown(wait=False,cancel_futures=True)
            self.status_executor.shutdown(wait=False,cancel_futures=True)
            return False
        if self.busy:
            def stop():
                self.closing=True;self.backend.cancel();self.status_label.setText('Vorgang wird beendet …')
            self.confirm('Laufenden Vorgang abbrechen?','Bereits übertragene Dateien bleiben erhalten. Die App wartet auf das Ende des Backends.',stop)
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
        self.submit("Lokalen Papierkorb laden", work, self.render_local_trash)

    def restore_local_trash(self, state, receipt_id):
        if self.demo:
            self.message("Der Demo-Modus stellt keine Dateien wieder her.")
            return
        def work():
            from .local_trash import LocalTrash
            def check_cancel():
                if self.backend.cancel_event.is_set():
                    raise Cancelled("Wiederherstellung abgebrochen; erhaltene Daten bleiben im Papierkorb oder Wiederherstellungsziel.")
            try:
                return LocalTrash(Path(state)).restore(receipt_id, check_cancel=check_cancel)
            except (OSError, ValueError) as exc:
                raise SyncError(str(exc)) from exc
        def done(_):
            self.message("Datei wiederhergestellt; bestehende Dateien wurden nicht überschrieben.")
            self.refresh_local_trash()
        self.submit("Datei wiederherstellen", work, done)

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
            self.message("Im Demo-Modus werden keine Hintergrund-Aufträge angelegt.")
            return
        if not self.background_local:
            self.message("Bitte zuerst den lokalen Sync-Ordner auswählen.", error=True)
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
                        self.submit("Hintergrund-Sync aktivieren", work, self.render_background_status)
                    else:
                        self.message("Der Desktop hat Hintergrundbetrieb nicht freigegeben. Es wurde kein Auftrag aktiviert.", error=True)
                request_portal_background(True, permission)
            else:
                self.submit("Hintergrund-Sync aktivieren", work, self.render_background_status)
        self.confirm("Automatischen Sync aktivieren?", "Neue und geänderte Dateien werden künftig automatisch übertragen. Nach einem bestätigten gemeinsamen Stand werden Löschungen auf der Gegenseite in den Papierkorb übernommen. Bei gleichzeitigen Änderungen bleiben Dateien als Konflikt erhalten. Flatpak fragt zusätzlich nach Hintergrundbetrieb und Autostart.", activate)

    def background_action(self, action, job_id=None):
        if self.demo:
            self.message("Der Demo-Modus startet keine Hintergrunddienste.")
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
        self.submit("Hintergrund-Sync", work, self.render_background_status)

    def mount_drive(self):
        if not self.mount_local:
            self.message("Bitte zuerst einen leeren Einhängeordner auswählen.", error=True)
            return
        self.mount_action("mount", mountpoint=str(self.mount_local), remote_root=self.mount_remote.text(),
                          writable=self.mount_writable.isChecked(), offline=self.mount_offline.isChecked())

    def mount_action(self, action, **options):
        if self.demo:
            self.message("Im Demo-Modus wird kein Laufwerk verbunden oder verändert.")
            return
        if action == "mount" and os.environ.get("SNAP"):
            self.message("Online-Laufwerke sind im Snap nicht verfügbar. Bitte das native DEB- oder RPM-Paket verwenden; Dateien durchsuchen und synchronisieren bleibt im Snap möglich.", error=True)
            return
        from .desktop_service import DesktopClient
        client = DesktopClient(self.backend.executable)
        def done(value):
            self.render_mount_status(value)
            self.message("Laufwerksaktion abgeschlossen.")
        self.submit("Online-Laufwerk", lambda: client.request(action, **options), done)

    def render_mount_status(self, value):
        state = "Verbunden" if value.get("mounted") else "Nicht verbunden"
        path = value.get("mountpoint", "")
        recovery = value.get("pending_recovery", value.get("recovery", []))
        text = state + (" · " + str(path) if path else "")
        if recovery:
            text += "\nNicht hochgeladene Änderungen im Wiederherstellungsspeicher: " + str(recovery)
        if value.get("error"):
            text += "\n" + str(value["error"])
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
            request_portal_background(enabled, lambda ok:self.message("Autostart eingerichtet." if ok else "Das Desktop-Portal hat Autostart nicht freigegeben.", error=not ok))
        else:
            from .desktop_integration import set_autostart
            def work():
                set_autostart(enabled, self.backend.executable)
                return enabled
            self.submit("Autostart", work, lambda enabled:self.message("Autostart aktiviert." if enabled else "Autostart deaktiviert."))

    def navigate_root(self, title, path):
        if self.busy:
            self.message("Bitte den laufenden Vorgang beenden oder abbrechen.")
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
        self.submit("Dateiliste laden", lambda: self.backend.list_nodes(path), self.loaded)

    def loaded(self, nodes):
        self.nodes = nodes
        self.empty_label.setText("Dieser Ordner ist leer.")
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
            self.message("Es läuft bereits ein Vorgang. Bitte warten oder abbrechen.")
            return
        self.busy = True
        self.backend.reset_cancel()
        self.spinner.setVisible(True)
        self.cancel_button.setVisible(True)
        self.status_label.setText(title + " …")
        self.file_list.setEnabled(False)
        self.update_actions()
        self.plan_button.setEnabled(False)
        self.execute_button.setEnabled(False)
        self.local_choose.setEnabled(False)
        self.remote_entry.setEnabled(False)
        self.records.insert(0, [datetime.now().strftime("%H:%M:%S"), title, "Läuft …"])
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
        self.plan_button.setEnabled(True)
        self.local_choose.setEnabled(True)
        self.remote_entry.setEnabled(True)
        self.update_actions()
        try:
            value = future.result()
        except Exception as exc:
            text = str(exc) if isinstance(exc, (BackendError, SyncError)) else f"Interner Fehler ({type(exc).__name__}). Es wurde kein Erfolg angenommen."
            record[2] = "Fehlgeschlagen · " + text
            self.status_label.setText(title + " · nicht abgeschlossen")
            self.message(text, error=True)
        else:
            record[2] = "Abgeschlossen"
            self.status_label.setText(title + " · abgeschlossen")
            if isinstance(value, dict) and "transferredItems" in value:
                text = f"{value['transferredItems']} übertragen · {value.get('skippedItems', 0)} übersprungen · {size_text(value.get('transferredBytes', 0))}"
                record[2] = text
                self.message(text)
            if done and not self.closing:
                try:
                    done(value)
                except Exception as exc:
                    record[2] = "Anzeige fehlgeschlagen"
                    self.message(f"Ergebnis konnte nicht angezeigt werden ({type(exc).__name__}). Bitte aktualisieren.", error=True)
        self.render_activity()
        if self.closing:
            self.window.close()
            self.quit()
        return False

    def open_login(self):
        if self.login_url:
            self.open_url(self.login_url)

    def login(self):
        self.message("Die Anmeldung erfolgt bei Proton im Browser. Diese App ist unabhängig und wird nicht von Proton unterstützt.")
        def on_url(url):
            self.dispatch(self._login_ready, url)
        self.submit("Browser-Anmeldung", lambda: self.backend.login(on_url), lambda _: self.refresh())

    def _login_ready(self, url):
        self.login_url = url
        self.login_button.setVisible(True)
        self.message("Bitte die Anmeldung im geöffneten Proton-Browserfenster abschließen. Falls kein Fenster erscheint, den Anmeldeknopf verwenden.")
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
        self.confirm("Von Proton abmelden?", "Hintergrund-Sync wird gestoppt. Native Online-Laufwerke müssen vorher alle geöffneten Dateien schließen. Dies beendet auch die gemeinsam verwendete native CLI-Sitzung.",
                     lambda: self.submit("Abmelden", work, self.logged_out))

    def logged_out(self, _):
        self.nodes = []
        self.render_nodes()
        self.invalidate_plan()
        self.message("Von Proton abgemeldet.")

    def show_version(self, value):
        self.backend_label.setText(str(value))

    def choose_upload(self, folder):
        target = self.path
        self.choose("Ordner hochladen" if folder else "Dateien hochladen", folder=folder, multiple=not folder,
                    callback=lambda paths: self.submit("Upload", lambda: self.backend.upload(paths, target), lambda _: self.refresh()))

    def download(self):
        paths = [n.path for n in self.selected()]
        if not paths:
            return
        self.choose("Download-Zielordner wählen", folder=True,
                    callback=lambda selected: self.submit("Download", lambda: self.backend.download(paths, selected[0])))

    def new_folder(self):
        target = self.path
        self.prompt("Neuer Ordner", "Name des Ordners", "", lambda value: self.submit("Ordner erstellen", lambda: self.backend.mkdir(target, value), lambda _: self.refresh()))

    def rename(self):
        nodes = self.selected()
        if len(nodes) != 1:
            return
        node = nodes[0]
        self.prompt("Umbenennen", "Neuer Name", node.name, lambda value: self.submit("Umbenennen", lambda: self.backend.rename(node.path, value), lambda _: self.refresh()))

    def move(self):
        paths = [n.path for n in self.selected()]
        self.prompt("Verschieben", "Cloud-Zielordner (z. B. /my-files/Dokumente)", "/my-files",
                    lambda value: self.submit("Verschieben", lambda: self.backend.move(paths, value), lambda _: self.refresh()))

    def trash(self):
        nodes = self.selected()
        if not nodes:
            return
        paths = [n.path for n in nodes]
        self.confirm("In den Papierkorb verschieben?", f"{len(nodes)} ausgewählte Einträge werden in den Proton-Papierkorb verschoben.",
                     lambda: self.submit("Papierkorb", lambda: self.backend.trash(paths), lambda _: self.refresh()), destructive=True)

    def restore(self):
        paths = [n.path for n in self.selected()]
        if paths:
            self.submit("Wiederherstellen", lambda: self.backend.restore(paths), lambda _: self.refresh())

    def share(self):
        nodes = self.selected()
        if len(nodes) == 1:
            self.submit("Freigabestatus laden", lambda: self.backend.sharing(nodes[0].path), lambda value: self.share_dialog(nodes[0], value))

    def choose_sync_local(self):
        def chosen(paths):
            self.sync_local = paths[0]
            self.local_label.setText(str(paths[0]))
            self.invalidate_plan()
        self.choose("Lokalen Sync-Ordner auswählen", folder=True, callback=chosen)

    def invalidate_plan(self):
        self.sync_plan = None
        if hasattr(self, "execute_button"):
            self.execute_button.setEnabled(False)

    def plan_sync(self):
        if not self.sync_local:
            self.message("Bitte zuerst einen lokalen Ordner auswählen.")
            return
        if not self.demo:
            try:
                for job in self.background_clients()[0].list():
                    current, automatic = Path(self.sync_local).absolute(), Path(job.local_root)
                    if not job.paused and (current == automatic or current in automatic.parents or automatic in current.parents):
                        self.message("Diesen automatischen Auftrag zuerst pausieren, bevor ein manueller Plan erstellt wird.", error=True)
                        return
            except (BackendError, SyncError) as exc:
                self.message(str(exc), error=True)
                return
        from .sync import SyncEngine
        self.sync_engine = SyncEngine(self.backend, data_dir() / "sync")
        self.invalidate_plan()
        local, remote = self.sync_local, self.remote_entry.text()
        self.submit("Sync-Vorschau erstellen", lambda: self.sync_engine.plan(local, remote), self.sync_preview)

    def sync_preview(self, plan):
        self.sync_plan = plan
        lines = [plan.summary(), ""]
        actions = {"upload":"Hochladen", "download":"Herunterladen", "mkdir_local":"Lokalen Ordner anlegen",
                   "mkdir_remote":"Cloud-Ordner anlegen", "trash_local":"In lokalen Papierkorb",
                   "trash_remote":"In Cloud-Papierkorb"}
        for action in plan.actions:
            lines.append(f"{actions.get(action.kind, action.kind)}: {action.relative_path}")
        for conflict in plan.conflicts:
            lines.append(f"KONFLIKT: {conflict}")
        for skipped in plan.skipped:
            lines.append(f"ÜBERSPRUNGEN: {skipped}")
        self.sync_text.setPlainText("\n".join(lines))
        # Executing a plan with no transfers can still establish the baseline.
        self.execute_button.setEnabled(True)

    def execute_sync(self):
        if self.sync_plan is None:
            return
        plan = self.sync_plan
        self.confirm("Diesen Sync-Plan ausführen?", plan.summary(),
                     lambda: self.submit("Manueller Abgleich", lambda: self.sync_engine.execute(plan), self.sync_done))

    def sync_done(self, result):
        self.invalidate_plan()
        lines = [f"{len(result.completed)} abgeschlossen · {len(result.conflicts)} Konflikte · {len(result.errors)} Fehler", ""]
        lines += ["OK: " + str(x) for x in result.completed]
        lines += ["KONFLIKT: " + str(x) for x in result.conflicts]
        lines += ["FEHLER: " + str(x) for x in result.errors]
        lines += ["LOKALE SICHERUNG: " + str(x) for x in result.backups]
        lines += ["PAPIERKORB: " + str(x) for x in result.trashed]
        lines += ["ÜBERSPRUNGEN: " + str(x) for x in result.skipped]
        self.sync_text.setPlainText("\n".join(lines))
        if self.records:
            self.records[0][2] = lines[0]
        self.status_label.setText(lines[0])
        self.message(lines[0], error=bool(result.errors or result.conflicts))
