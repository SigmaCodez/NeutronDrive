"""GTK4 desktop UI. All network/subprocess work stays off the GTK thread."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
import json
import os
from pathlib import Path

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk, Pango

from . import APP_ID, __version__
from .backend import BackendError, Cancelled, data_dir
from .sync import SyncError


CSS = b"""
window { background: #111a23; color: #e5ecf1; }
headerbar { background: #151f29; border-bottom: 1px solid #293541; box-shadow: none; }
button { border-radius: 8px; padding: 7px 12px; }
button.suggested-action { background: #67dfbf; color: #092d25; font-weight: 700; }
button.destructive-action { background: #843e48; color: white; }
.sidebar { background: #151f29; border-right: 1px solid #293541; padding: 16px 14px; }
.brand { font-size: 23px; font-weight: 800; letter-spacing: -0.5px; }
.brand-icon { color: #67dfbf; }
.eyebrow { font-size: 10px; letter-spacing: 1.5px; color: #8e9daa; font-weight: 700; }
.muted { color: #a2afba; }
.small { font-size: 12px; }
.nav { background: transparent; border: none; box-shadow: none; padding: 8px 12px; }
.nav.active { background: #24453f; color: #91edd2; }
.title { font-size: 30px; font-weight: 750; letter-spacing: -0.5px; }
.subtitle { color: #a2afba; font-size: 13px; }
.panel { background: #18232e; border: 1px solid #2b3945; border-radius: 12px; padding: 18px; }
.file-list { background: transparent; }
.file-list row { border-bottom: 1px solid #25323d; padding: 13px 16px; border-radius: 0; }
.file-list row:selected { background: #24453f; }
.file-list row:hover { background: #1b2d37; }
.file-icon { color: #72d9bd; margin-right: 12px; }
.file-name { font-size: 14px; font-weight: 600; }
.table-heading { color: #91a2ae; font-size: 11px; font-weight: 700; padding: 12px 16px; }
.status { background: #213a3a; border: 1px solid #355654; border-radius: 8px; padding: 12px; }
.warning { background: #493b25; border-color: #79623e; }
.chip { background: #24453f; color: #91edd2; border-radius: 6px; padding: 4px 8px; font-size: 10px; font-weight: 700; }
entry, textview, textview text { background: #101922; color: #e5ecf1; }
.empty { color: #a2afba; padding: 40px; }
dialog { background: #18232e; }
"""


def label(text="", css=None, xalign=0):
    widget = Gtk.Label(label=text, xalign=xalign)
    if css:
        for cls in css.split():
            widget.add_css_class(cls)
    return widget


def button(text=None, icon=None, callback=None, css=None):
    widget = Gtk.Button()
    if text and icon:
        box = Gtk.Box(spacing=8)
        box.append(Gtk.Image.new_from_icon_name(icon))
        box.append(label(text))
        widget.set_child(box)
    elif icon:
        widget.set_icon_name(icon)
    else:
        widget.set_label(text)
    if callback:
        widget.connect("clicked", lambda *_: callback())
    if css:
        widget.add_css_class(css)
    return widget


def margin(widget, size):
    widget.set_margin_top(size)
    widget.set_margin_bottom(size)
    widget.set_margin_start(size)
    widget.set_margin_end(size)
    return widget


def size_text(size):
    if size is None:
        return "—"
    value = float(size)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if value < 1024 or unit == "TB":
            return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"
        value /= 1024


def date_text(value):
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone().strftime("%d.%m.%Y")
    except (ValueError, AttributeError):
        return "—"


class Application(Gtk.Application):
    def __init__(self, backend, *, demo=False):
        super().__init__(application_id=APP_ID + (".Demo" if demo else ""))
        if demo:
            from .demo import DemoBackend
            backend = DemoBackend()
        self.backend = backend
        self.demo = demo
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="neutrondrive")
        self.status_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="neutrondrive-status")
        self.status_polling = False
        self.window = None
        self.busy = False
        self.closing = False
        self.nodes = []
        self.path = "/my-files"
        self.crumbs = [("Meine Dateien", self.path)]
        self.current_page = "files"
        self.nav_buttons = {}
        self.records = []
        self.sync_plan = None
        self.sync_local = None
        self.login_url = None

    def do_activate(self):
        if self.window:
            self.window.present()
            return
        settings = Gtk.Settings.get_default()
        if settings:
            settings.set_property("gtk-application-prefer-dark-theme", True)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.window = Gtk.ApplicationWindow(application=self, title="NeutronDrive" + (" · DEMO" if self.demo else ""))
        self.window.set_default_size(1180, 760)
        self.window.set_size_request(850, 580)
        self.window.connect("close-request", self._close)
        self._build()
        self.window.present()
        self.service_timer = GLib.timeout_add_seconds(3, self.poll_services)
        if self.backend.available:
            self.refresh()
        else:
            self.show_page("settings")
            self.message("Das Proton-SDK-Backend wurde nicht gefunden. Installationshinweise stehen unter Einstellungen.", error=True)

    def _build(self):
        header = Gtk.HeaderBar()
        header.set_title_widget(label("NEUTRONDRIVE", "eyebrow"))
        version = label("ALPHA  ·  " + __version__, "chip")
        header.pack_start(version)
        if self.demo:
            header.pack_start(label("DEMO · Beispieldaten", "chip"))
        self.spinner = Gtk.Spinner()
        header.pack_end(self.spinner)
        self.cancel_button = button("Abbrechen", callback=self.backend.cancel)
        self.cancel_button.set_visible(False)
        header.pack_end(self.cancel_button)
        self.window.set_titlebar(header)
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.window.set_child(root)
        body = Gtk.Box()
        body.set_vexpand(True)
        root.append(body)
        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        sidebar.set_size_request(218, -1)
        sidebar.add_css_class("sidebar")
        sidebar_scroll = Gtk.ScrolledWindow()
        sidebar_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        sidebar_scroll.set_child(sidebar)
        body.append(sidebar_scroll)
        brand_box = Gtk.Box(spacing=10)
        brand_icon = Gtk.Image.new_from_icon_name("network-server-symbolic")
        brand_icon.add_css_class("brand-icon")
        brand_icon.set_pixel_size(30)
        brand_box.append(brand_icon)
        brand_box.append(label("Neutron", "brand"))
        sidebar.append(brand_box)
        desc = label("Deine Dateien. Dein Linux.", "muted small")
        desc.set_margin_bottom(20)
        sidebar.append(desc)
        sidebar.append(label("CLOUD", "eyebrow"))
        for title, icon, path in [
            ("Meine Dateien", "folder-symbolic", "/my-files"),
            ("Mit mir geteilt", "system-users-symbolic", "/shared-with-me"),
            ("Computer", "computer-symbolic", "/devices"),
            ("Papierkorb", "user-trash-symbolic", "/trash"),
        ]:
            nav = button(title, icon, lambda p=path, t=title: self.navigate_root(t, p), "nav")
            sidebar.append(nav)
            self.nav_buttons[path] = nav
        section = label("ARBEITSPLATZ", "eyebrow")
        section.set_margin_top(18)
        sidebar.append(section)
        for title, icon, page in [
            ("Synchronisierung", "emblem-synchronizing-symbolic", "sync"),
            ("Automatischer Sync", "view-refresh-symbolic", "background"),
            ("Online-Laufwerk", "network-server-symbolic", "ondemand"),
            ("Lokaler Papierkorb", "user-trash-symbolic", "local-trash"),
            ("Aktivität", "document-open-recent-symbolic", "activity"),
            ("Einstellungen", "preferences-system-symbolic", "settings"),
        ]:
            nav = button(title, icon, lambda p=page: self.show_page(p), "nav")
            sidebar.append(nav)
            self.nav_buttons[page] = nav
        spacer = Gtk.Box()
        spacer.set_vexpand(True)
        sidebar.append(spacer)
        privacy = label("Verschlüsselung durch\ndas Proton SDK", "muted small")
        privacy.set_margin_bottom(12)
        sidebar.append(privacy)
        sidebar.append(button("Bei Proton anmelden", "avatar-default-symbolic", self.login))
        sidebar.append(button("Proton Drive im Web", "web-browser-symbolic", lambda: self.open_url("https://drive.proton.me"), "flat"))
        footnote = label("Unabhängige Drittanbieter-App\nNicht von Proton unterstützt", "muted small")
        footnote.set_margin_top(8)
        sidebar.append(footnote)
        content = margin(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16), 26)
        content.set_hexpand(True)
        body.append(content)
        self.notice = Gtk.Revealer()
        notice_box = Gtk.Box(spacing=12)
        notice_box.add_css_class("status")
        self.notice_box = notice_box
        self.notice_label = label()
        self.notice_label.set_wrap(True)
        self.notice_label.set_hexpand(True)
        notice_box.append(self.notice_label)
        notice_box.append(button(icon="window-close-symbolic", callback=lambda: self.notice.set_reveal_child(False)))
        self.notice.set_child(notice_box)
        content.append(self.notice)
        self.login_button = button("Anmeldung im Browser öffnen", "web-browser-symbolic", self.open_login, "suggested-action")
        self.login_button.set_visible(False)
        content.append(self.login_button)
        self.stack = Gtk.Stack()
        self.stack.set_vexpand(True)
        self.stack.set_hexpand(True)
        content.append(self.stack)
        self._files_page()
        self._sync_page()
        self._background_page()
        self._ondemand_page()
        self._local_trash_page()
        self._activity_page()
        self._settings_page()
        footer = Gtk.Box(spacing=10)
        footer.append(label("●", "brand-icon"))
        self.status_label = label("Bereit", "muted small")
        self.status_label.set_hexpand(True)
        footer.append(self.status_label)
        footer.append(label("Open Source · Linux", "muted small"))
        content.append(footer)
        self._set_nav("/my-files")

    def _files_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        self.stack.add_named(page, "files")
        title_row = Gtk.Box(spacing=12)
        titles = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
        titles.set_hexpand(True)
        self.files_title = label("Meine Dateien", "title")
        titles.append(self.files_title)
        titles.append(label("Sicher gespeichert. Auf Linux zu Hause.", "subtitle"))
        title_row.append(titles)
        self.search = Gtk.SearchEntry()
        # The property predates the convenience setter (introduced in GTK 4.10).
        self.search.set_property("placeholder-text", "In diesem Ordner suchen")
        self.search.set_valign(Gtk.Align.CENTER)
        self.search.set_size_request(230, -1)
        self.search.connect("search-changed", lambda *_: self.render_nodes())
        title_row.append(self.search)
        page.append(title_row)
        self.file_controls = Gtk.Box(spacing=8)
        self.upload_button = button("Dateien hochladen", "go-up-symbolic", lambda: self.choose_upload(False), "suggested-action")
        self.folder_upload_button = button("Ordner hochladen", callback=lambda: self.choose_upload(True))
        self.mkdir_button = button("Neuer Ordner", "folder-new-symbolic", self.new_folder)
        self.file_controls.append(self.upload_button)
        self.file_controls.append(self.folder_upload_button)
        self.file_controls.append(self.mkdir_button)
        grow = Gtk.Box()
        grow.set_hexpand(True)
        self.file_controls.append(grow)
        self.refresh_button = button(icon="view-refresh-symbolic", callback=self.refresh)
        self.refresh_button.set_tooltip_text("Aktualisieren")
        self.file_controls.append(self.refresh_button)
        page.append(self.file_controls)
        self.breadcrumb = Gtk.Box(spacing=4)
        page.append(self.breadcrumb)
        table = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        table.add_css_class("panel")
        table.set_vexpand(True)
        page.append(table)
        heading = Gtk.Box(spacing=14)
        heading.add_css_class("table-heading")
        name_heading = label("NAME")
        name_heading.set_hexpand(True)
        heading.append(name_heading)
        for text, width in [("GRÖSSE", 90), ("GEÄNDERT", 110)]:
            head = label(text)
            head.set_size_request(width, -1)
            heading.append(head)
        table.append(heading)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        table.append(scroll)
        self.file_list = Gtk.ListBox()
        self.file_list.add_css_class("file-list")
        self.file_list.set_selection_mode(Gtk.SelectionMode.MULTIPLE)
        self.file_list.connect("row-activated", self.row_activated)
        self.file_list.connect("selected-rows-changed", lambda *_: self.update_actions())
        self.empty_label = label("Noch keine Dateien geladen.\nBei Proton anmelden oder aktualisieren.", "empty", 0.5)
        self.empty_label.set_wrap(True)
        self.file_list.set_placeholder(self.empty_label)
        scroll.set_child(self.file_list)
        actions = Gtk.Box(spacing=6)
        self.action_buttons = {}
        for text, action in [("Download", self.download), ("Umbenennen", self.rename), ("Verschieben", self.move),
                             ("Freigabe", self.share), ("Papierkorb", self.trash), ("Wiederherstellen", self.restore)]:
            widget = button(text, callback=action)
            self.action_buttons[text] = widget
            actions.append(widget)
        page.append(actions)
        self.count_label = label("", "muted small")
        page.append(self.count_label)
        self.update_breadcrumb()
        self.update_actions()

    def _sync_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        self.stack.add_named(page, "sync")
        page.append(label("Synchronisierung", "title"))
        intro = label("Zwei Ordner manuell abgleichen. Prüfe zuerst die Vorschau; Konflikte bleiben zur Klärung erhalten.", "subtitle")
        intro.set_wrap(True)
        page.append(intro)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        box.add_css_class("panel")
        page.append(box)
        row = Gtk.Box(spacing=12)
        self.local_label = label("Kein lokaler Ordner gewählt", "muted")
        self.local_label.set_hexpand(True)
        self.local_label.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        row.append(self.local_label)
        self.local_choose = button("Lokalen Ordner wählen", "folder-open-symbolic", self.choose_sync_local)
        row.append(self.local_choose)
        box.append(row)
        box.append(label("Cloud-Ordner (z. B. /my-files/Dokumente)", "small muted"))
        self.remote_entry = Gtk.Entry(text="/my-files")
        self.remote_entry.connect("changed", lambda *_: self.invalidate_plan())
        box.append(self.remote_entry)
        actions = Gtk.Box(spacing=10)
        self.plan_button = button("Vorschau erstellen", "view-refresh-symbolic", self.plan_sync, "suggested-action")
        self.execute_button = button("Geprüften Plan ausführen", callback=self.execute_sync)
        self.execute_button.set_sensitive(False)
        actions.append(self.plan_button)
        actions.append(self.execute_button)
        box.append(actions)
        note = label("Automatische Aufträge unter »Automatischer Sync« einrichten. Bestätigte Dateilöschungen werden auf der Gegenseite in den Papierkorb verschoben. Ganze Ordnerlöschungen bleiben zur sicheren Prüfung als Konflikt erhalten. Gleichzeitige Änderungen werden als Konflikte behandelt; lokale ersetzte Versionen werden gesichert.", "muted small")
        note.set_wrap(True)
        page.append(note)
        self.sync_text = Gtk.TextView(editable=False, cursor_visible=False, monospace=True, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.sync_text.set_top_margin(16)
        self.sync_text.set_left_margin(16)
        self.sync_text.set_right_margin(16)
        self.sync_text.get_buffer().set_text("Wähle einen lokalen Ordner und den zugehörigen Cloud-Ordner.")
        scroll = Gtk.ScrolledWindow()
        scroll.set_vexpand(True)
        scroll.set_child(self.sync_text)
        page.append(scroll)

    def _background_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        page_scroll = Gtk.ScrolledWindow()
        page_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        page_scroll.set_child(page)
        self.stack.add_named(page_scroll, "background")
        page.append(label("Automatischer Sync", "title"))
        intro = label("Änderungen werden im Hintergrund abgeglichen – auch nach dem Schließen dieses Fensters.", "subtitle")
        intro.set_wrap(True)
        page.append(intro)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        box.add_css_class("panel")
        page.append(box)
        row = Gtk.Box(spacing=10)
        self.background_local = None
        self.background_local_label = label("Kein lokaler Ordner gewählt", "muted")
        self.background_local_label.set_hexpand(True)
        self.background_local_label.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        row.append(self.background_local_label)
        row.append(button("Ordner wählen", "folder-open-symbolic", lambda: self.choose("Sync-Ordner wählen", folder=True, callback=self.choose_background_local)))
        box.append(row)
        box.append(label("Cloud-Ordner unter /my-files", "small muted"))
        self.background_remote = Gtk.Entry(text="/my-files")
        box.append(self.background_remote)
        controls = Gtk.Box(spacing=8)
        self.background_add_button = button("Automatischen Sync aktivieren", "emblem-synchronizing-symbolic", self.add_background_job, "suggested-action")
        self.background_add_button.set_sensitive(not self.demo)
        controls.append(self.background_add_button)
        controls.append(button("Dienst starten", callback=lambda: self.background_action("start")))
        controls.append(button("Dienst stoppen", callback=lambda: self.background_action("stop")))
        box.append(controls)
        self.background_status = label("Noch keine Hintergrund-Aufträge geladen.", "muted small")
        self.background_status.set_wrap(True)
        page.append(self.background_status)
        self.background_jobs = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.background_jobs.add_css_class("file-list")
        self.background_jobs.set_placeholder(label("Noch keine automatischen Sync-Ordner eingerichtet.", "empty"))
        scroll = Gtk.ScrolledWindow()
        scroll.set_min_content_height(160)
        scroll.set_max_content_height(320)
        scroll.set_propagate_natural_height(True)
        scroll.set_child(self.background_jobs)
        page.append(scroll)
        note = label("Konflikte bleiben erhalten und erscheinen beim Auftrag. Dateilöschungen werden nur bei unverändertem Gegenstück in den Papierkorb übertragen. Ganze Ordnerlöschungen bleiben als Konflikt erhalten. Ein entsperrter Schlüsselbund und eine aktive Anmeldung sind erforderlich.", "muted small")
        note.set_wrap(True)
        page.append(note)

    def _ondemand_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        page_scroll = Gtk.ScrolledWindow()
        page_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        page_scroll.set_child(page)
        self.stack.add_named(page_scroll, "ondemand")
        page.append(label("Online-Laufwerk", "title"))
        intro = label("Dateien im Dateimanager sehen und beim Öffnen laden. Offline vorhalten oder lokalen Speicher wieder freigeben.", "subtitle")
        intro.set_wrap(True)
        page.append(intro)
        panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        panel.add_css_class("panel")
        page.append(panel)
        panel.append(label("Leerer lokaler Ordner als Laufwerk", "small muted"))
        self.mount_local = None
        self.mount_label = label("Kein Einhängeordner gewählt", "muted")
        self.mount_label.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        row = Gtk.Box(spacing=10)
        self.mount_label.set_hexpand(True)
        row.append(self.mount_label)
        row.append(button("Ordner wählen", "folder-open-symbolic", lambda: self.choose("Leeren Einhängeordner wählen", folder=True, callback=self.choose_mount_local)))
        panel.append(row)
        panel.append(label("Cloud-Ordner", "small muted"))
        self.mount_remote = Gtk.Entry(text="/my-files")
        panel.append(self.mount_remote)
        self.mount_writable = Gtk.CheckButton(label="Dateien im Laufwerk auch bearbeiten und hochladen")
        panel.append(self.mount_writable)
        self.mount_offline = Gtk.CheckButton(label="Offline aus vorhandenem Cache öffnen")
        panel.append(self.mount_offline)
        controls = Gtk.Box(spacing=8)
        self.mount_button = button("Laufwerk verbinden", "network-server-symbolic", self.mount_drive, "suggested-action")
        self.mount_button.set_sensitive(not self.demo and not os.environ.get("SNAP"))
        if os.environ.get("SNAP"):
            self.mount_button.set_tooltip_text("Online-Laufwerke sind im Snap nicht verfügbar. Bitte das native DEB- oder RPM-Paket verwenden.")
        controls.append(self.mount_button)
        controls.append(button("Trennen", callback=lambda: self.mount_action("unmount")))
        controls.append(button("Aktualisieren", callback=lambda: self.mount_action("refresh")))
        panel.append(controls)
        self.mount_status = label("Noch kein Online-Laufwerk verbunden.", "muted")
        self.mount_status.set_wrap(True)
        page.append(self.mount_status)
        cache = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        cache.add_css_class("panel")
        page.append(cache)
        cache.append(label("Dateipfad im Laufwerk, z. B. Dokumente/Notizen.txt", "small muted"))
        self.cache_path = Gtk.Entry()
        cache.append(self.cache_path)
        actions = Gtk.Box(spacing=8)
        actions.append(button("Offline vorhalten", callback=lambda: self.mount_action("pin", path=self.cache_path.get_text())))
        actions.append(button("Speicher freigeben", callback=lambda: self.mount_action("evict", path=self.cache_path.get_text())))
        cache.append(actions)
        self.mount_notice = label("Für Flatpak zusätzlich das native NeutronDrive-Paket als Laufwerkshelfer installieren. Online-Laufwerke sind im Snap derzeit nicht verfügbar; dafür das native DEB- oder RPM-Paket verwenden. Dateien durchsuchen, übertragen und synchronisieren bleibt im Snap möglich. Nicht hochgeladene Änderungen bleiben im Wiederherstellungsspeicher erhalten.", "muted small")
        self.mount_notice.set_wrap(True)
        page.append(self.mount_notice)
        page.append(button("Laufwerkshelfer bei Proton anmelden", "avatar-default-symbolic", lambda: self.mount_action("login")))

    def _local_trash_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        self.stack.add_named(page, "local-trash")
        page.append(label("Lokaler Papierkorb", "title"))
        note = label("Dateien, die NeutronDrive nach einer Cloud-Löschung lokal in den Papierkorb verschoben hat. Cloud-Dateien findest du im Papierkorb unter CLOUD.", "subtitle")
        note.set_wrap(True)
        page.append(note)
        refresh = button("Aktualisieren", "view-refresh-symbolic", self.refresh_local_trash)
        refresh.set_halign(Gtk.Align.START)
        page.append(refresh)
        self.local_trash_status = label("Noch keine Einträge geladen.", "muted small")
        self.local_trash_status.set_wrap(True)
        page.append(self.local_trash_status)
        self.local_trash_rows = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.local_trash_rows.add_css_class("file-list")
        self.local_trash_empty = label("Keine von NeutronDrive registrierten Papierkorbeinträge.", "empty")
        self.local_trash_empty.set_wrap(True)
        self.local_trash_rows.set_placeholder(self.local_trash_empty)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        scroll.set_child(self.local_trash_rows)
        page.append(scroll)
        note = label("Wiederherstellen überschreibt keine vorhandenen Dateien. Bei aktivem Sync wird die Wiederherstellung erneut synchronisiert. NeutronDrive leert den Papierkorb nicht automatisch.", "muted small")
        note.set_wrap(True)
        page.append(note)

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

    def render_local_trash(self, values):
        while (row := self.local_trash_rows.get_row_at_index(0)) is not None:
            self.local_trash_rows.remove(row)
        self.local_trash_status.set_text(f"{len(values)} registrierte Einträge" + (" · Demo ohne Dateizugriff" if self.demo else ""))
        for state, entry in values:
            row = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=7)
            title = label(str(entry.original_path), "file-name")
            title.set_wrap(True)
            title.set_selectable(True)
            row.append(title)
            states = {"prepared":"Vorbereitet", "trashed":"Im Papierkorb", "restoring":"Wiederherstellung läuft", "restored":"Wiederhergestellt", "recovery":"Prüfung erforderlich", "missing":"Nicht mehr im Papierkorb", "aborted":"Abgebrochen"}
            detail = label(states.get(entry.status, entry.status) + " · " + str(entry.trash_path), "muted small")
            detail.set_wrap(True)
            detail.set_selectable(True)
            row.append(detail)
            if getattr(entry, "error", ""):
                reason = label(entry.error, "small")
                reason.set_wrap(True)
                row.append(reason)
            if getattr(entry, "restore_path", ""):
                destination = label("Wiederherstellungsziel: " + entry.restore_path, "muted small")
                destination.set_wrap(True)
                destination.set_selectable(True)
                row.append(destination)
            restore = button("Am ursprünglichen Ort wiederherstellen" if entry.status == "trashed" else "Wiederherstellung prüfen", "edit-undo-symbolic",
                             lambda s=state,i=entry.id:self.restore_local_trash(s, i))
            restore.set_halign(Gtk.Align.START)
            restore.set_sensitive(not self.demo and entry.status in {"trashed", "prepared", "restoring", "recovery"})
            row.append(restore)
            self.local_trash_rows.append(row)

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
        self.background_local_label.set_text(str(paths[0]))

    def choose_mount_local(self, paths):
        self.mount_local = paths[0]
        self.mount_label.set_text(str(paths[0]))

    def add_background_job(self):
        if self.demo:
            self.message("Im Demo-Modus werden keine Hintergrund-Aufträge angelegt.")
            return
        if not self.background_local:
            self.message("Bitte zuerst den lokalen Sync-Ordner auswählen.", error=True)
            return
        local, remote = self.background_local, self.background_remote.get_text()
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

    def render_background_status(self, value):
        self.background_status.set_text("Dienst läuft" if value.get("running") else "Dienst ist gestoppt")
        source = value.get("event_source")
        if isinstance(source, dict) and source.get("error"):
            self.background_status.set_text(self.background_status.get_text() + " · " + str(source["error"]))
        if value.get("error"):
            self.background_status.set_text(self.background_status.get_text() + " · " + str(value["error"]))
        self.clear_box(self.background_jobs)
        names = {"idle":"Aktuell", "syncing":"Wird synchronisiert", "paused":"Pausiert", "error":"Fehler", "conflict":"Konflikt", "conflicts":"Konflikt", "blocked":"Blockiert", "waiting":"Wartet", "offline":"Offline"}
        for job in value.get("jobs", []):
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
            title = label(job.get("name") or job.get("local_root", "Sync-Ordner"), "file-name")
            title.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
            box.append(title)
            detail = label(str(job.get("local_root", "")) + " ↔ " + str(job.get("remote_root", "")), "muted small")
            detail.set_wrap(True)
            box.append(detail)
            state = str(job.get("state", "waiting"))
            detail = label(names.get(state, state) + (" · " + str(job["error"]) if job.get("error") else ""), "muted small")
            detail.set_wrap(True)
            box.append(detail)
            conflicts = job.get("conflicts", [])
            if conflicts:
                conflict_label = label("Konflikte: " + "; ".join(str(c) for c in conflicts[:5]), "small")
                conflict_label.set_wrap(True)
                box.append(conflict_label)
            actions = Gtk.Box(spacing=8)
            paused = job.get("paused", False)
            job_id = job["id"]
            actions.append(button("Fortsetzen" if paused else "Pausieren", callback=lambda i=job_id,p=paused:self.background_action("resume" if p else "pause", i)))
            actions.append(button("Jetzt abgleichen", callback=lambda i=job_id:self.background_action("sync", i)))
            actions.append(button("Auftrag entfernen", callback=lambda i=job_id:self.background_action("remove", i)))
            box.append(actions)
            self.background_jobs.append(box)

    def mount_drive(self):
        if not self.mount_local:
            self.message("Bitte zuerst einen leeren Einhängeordner auswählen.", error=True)
            return
        self.mount_action("mount", mountpoint=str(self.mount_local), remote_root=self.mount_remote.get_text(),
                          writable=self.mount_writable.get_active(), offline=self.mount_offline.get_active())

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
        self.mount_status.set_text(text)
        configuration = value.get("configuration", {})
        if configuration and not self.mount_local:
            self.mount_local = Path(configuration.get("mountpoint", ""))
            self.mount_label.set_text(str(self.mount_local))
            self.mount_remote.set_text(configuration.get("remote_root", "/my-files"))

    def poll_services(self):
        if self.closing:
            return GLib.SOURCE_REMOVE
        if self.demo or self.busy or self.status_polling or self.current_page not in {"background", "ondemand"}:
            return GLib.SOURCE_CONTINUE
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
            return GLib.SOURCE_REMOVE
        self.status_executor.submit(work).add_done_callback(lambda f:GLib.idle_add(finish, f))
        return GLib.SOURCE_CONTINUE

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

    def _activity_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        self.stack.add_named(page, "activity")
        page.append(label("Aktivität", "title"))
        page.append(label("Vorgänge dieser Sitzung · keine dauerhafte Speicherung von Dateinamen", "subtitle"))
        self.activity_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.activity_list.add_css_class("file-list")
        self.activity_list.set_placeholder(label("Noch keine Vorgänge in dieser Sitzung.", "empty"))
        scroll = Gtk.ScrolledWindow()
        scroll.set_vexpand(True)
        scroll.set_child(self.activity_list)
        page.append(scroll)

    def _settings_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        self.stack.add_named(page, "settings")
        page.append(label("Einstellungen", "title"))
        page.append(label("NeutronDrive " + __version__ + " · unabhängiges Open-Source-Projekt", "subtitle"))
        panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        panel.add_css_class("panel")
        page.append(panel)
        panel.append(label("Proton-Backend", "file-name"))
        self.backend_label = label(self.backend.executable or "Nicht installiert", "muted small")
        self.backend_label.set_wrap(True)
        self.backend_label.set_selectable(True)
        panel.append(self.backend_label)
        row = Gtk.Box(spacing=10)
        row.append(button("Backend prüfen", callback=lambda: self.submit("Backend prüfen", self.backend.version, self.show_version)))
        row.append(button("Anmelden", callback=self.login))
        row.append(button("Abmelden", callback=self.logout))
        panel.append(row)
        text = label("Die Anmeldung erfolgt ausschließlich auf account.proton.me im Browser. NeutronDrive fragt kein Proton-Passwort ab. Das SDK-Backend speichert die Sitzung im Linux-Schlüsselbund (GNOME Keyring/KWallet) oder in pass. Native Sitzungen werden gemeinsam genutzt; Flatpak kann einen getrennten, geschützten Kontospeicher verwenden.", "muted")
        text.set_wrap(True)
        panel.append(text)
        links = Gtk.Box(spacing=10)
        for text, url in [("Projekt & Installation", "https://github.com/SigmaCodez/NeutronDrive"),
                          ("Offizielle CLI", "https://proton.me/download/drive/cli/index.html")]:
            links.append(button(text, "web-browser-symbolic", lambda u=url: self.open_url(u)))
        page.append(links)
        startup = Gtk.Box(spacing=10)
        startup.append(button("Autostart aktivieren", callback=lambda:self.change_autostart(True)))
        startup.append(button("Autostart deaktivieren", callback=lambda:self.change_autostart(False)))
        page.append(startup)
        limits = label("Alpha-Version: noch kein vollständiger Ersatz der Windows-/macOS-App. Hintergrund-Sync ist verfügbar. Das Online-Laufwerk benötigt das native Paket; Flatpak nutzt dessen Helfer, Snap unterstützt es noch nicht. Löschungen verwenden den Papierkorb; Konflikte bleiben erhalten. Versionsverwaltung und Proton Docs/Sheets sind noch nicht integriert. Datei- und Kontozugriffe erfolgen über den Proton SDK; die Datenschutzregeln des Dienstes gelten weiter.", "muted small")
        limits.set_wrap(True)
        page.append(limits)

    def _set_nav(self, selected):
        for key, widget in self.nav_buttons.items():
            if key == selected:
                widget.add_css_class("active")
            else:
                widget.remove_css_class("active")

    def show_page(self, page):
        self.current_page = page
        self.stack.set_visible_child_name(page)
        self._set_nav(self.path.split("/", 2)[0] + "/" + self.path.split("/", 2)[1] if page == "files" else page)
        if page == "local-trash":
            self.refresh_local_trash()

    def navigate_root(self, title, path):
        if self.busy:
            self.message("Bitte den laufenden Vorgang beenden oder abbrechen.")
            return
        self.crumbs = [(title, path)]
        self.path = path
        self.search.set_text("")
        self.nodes = []
        self.render_nodes()
        self.show_page("files")
        self.update_breadcrumb()
        self.refresh()

    def update_breadcrumb(self):
        self.clear_box(self.breadcrumb)
        for index, (name, _) in enumerate(self.crumbs):
            if index:
                self.breadcrumb.append(label("/", "muted"))
            self.breadcrumb.append(button(name, callback=lambda i=index: self.go_crumb(i), css="flat"))
        self.files_title.set_text(self.crumbs[-1][0])

    def go_crumb(self, index):
        if self.busy:
            return
        self.crumbs = self.crumbs[:index + 1]
        self.path = self.crumbs[-1][1]
        self.nodes = []
        self.search.set_text("")
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
        self.empty_label.set_text("Dieser Ordner ist leer.")
        self.render_nodes()

    def render_nodes(self):
        self.clear_box(self.file_list)
        query = self.search.get_text().casefold()
        visible = [n for n in self.nodes if query in n.name.casefold()]
        if query:
            self.empty_label.set_text("Keine passenden Dateien in diesem Ordner.")
        for node in visible:
            row = Gtk.ListBoxRow()
            row.node = node
            box = Gtk.Box(spacing=14)
            row.set_child(box)
            icon_name = "folder-symbolic" if node.type == "folder" else "text-x-generic-symbolic"
            if node.name.lower().endswith((".jpg", ".jpeg", ".png", ".webp")):
                icon_name = "image-x-generic-symbolic"
            if not node.safe:
                icon_name = "dialog-warning-symbolic"
            icon = Gtk.Image.new_from_icon_name(icon_name)
            icon.set_pixel_size(26)
            icon.add_css_class("file-icon")
            box.append(icon)
            names = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            names.set_hexpand(True)
            filename = label(node.name, "file-name")
            filename.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
            filename.set_max_width_chars(55)
            filename.set_tooltip_text(node.name)
            names.append(filename)
            kind = "Ordner" if node.type == "folder" else (node.name.rsplit(".", 1)[-1].upper() + " · Datei" if "." in node.name else "Datei")
            if not node.safe:
                kind = "Nicht eindeutig oder nicht entschlüsselbar · Aktion gesperrt"
            names.append(label(kind, "small muted"))
            box.append(names)
            amount = label(size_text(node.size) if node.type != "folder" else "—", "muted small")
            amount.set_size_request(90, -1)
            box.append(amount)
            date = label(date_text(node.modified), "muted small")
            date.set_size_request(110, -1)
            box.append(date)
            self.file_list.append(row)
        self.count_label.set_text(f"{len(visible)} von {len(self.nodes)} Einträgen · Doppelklick öffnet Ordner")
        self.update_actions()

    @staticmethod
    def clear_box(box):
        child = box.get_first_child()
        while child:
            following = child.get_next_sibling()
            box.remove(child)
            child = following

    def selected(self):
        return [row.node for row in self.file_list.get_selected_rows()]

    def update_actions(self):
        if not hasattr(self, "action_buttons"):
            return
        nodes = self.selected()
        safe = bool(nodes) and all(n.safe for n in nodes) and not self.busy
        trash = self.path == "/trash"
        for name, widget in self.action_buttons.items():
            widget.set_visible((name == "Wiederherstellen") == trash)
            allowed = safe and (len(nodes) == 1 if name in {"Umbenennen", "Verschieben", "Freigabe"} else True)
            if name in {"Download", "Freigabe"} and trash:
                allowed = False
            widget.set_sensitive(allowed)
        writable_folder = self.path.startswith("/my-files") or self.path.count("/") > 1
        for widget in (self.upload_button, self.folder_upload_button, self.mkdir_button):
            widget.set_sensitive(writable_folder and not trash and not self.busy)
        self.refresh_button.set_sensitive(not self.busy)

    def row_activated(self, _list, row):
        node = row.node
        if self.busy or not node.safe:
            return
        if node.type == "folder" and self.path != "/trash":
            self.path = node.path
            self.crumbs.append((node.name, node.path))
            self.nodes = []
            self.search.set_text("")
            self.render_nodes()
            self.update_breadcrumb()
            self.refresh()
        elif self.path != "/trash":
            self.file_list.unselect_all()
            self.file_list.select_row(row)
            self.download()

    def message(self, text, *, error=False):
        self.notice_label.set_text(text)
        if error:
            self.notice_box.add_css_class("warning")
        else:
            self.notice_box.remove_css_class("warning")
        self.notice.set_reveal_child(True)

    def submit(self, title, work, done=None):
        if self.busy:
            self.message("Es läuft bereits ein Vorgang. Bitte warten oder abbrechen.")
            return
        self.busy = True
        self.backend.reset_cancel()
        self.spinner.start()
        self.cancel_button.set_visible(True)
        self.status_label.set_text(title + " …")
        self.file_list.set_sensitive(False)
        self.update_actions()
        self.plan_button.set_sensitive(False)
        self.execute_button.set_sensitive(False)
        self.local_choose.set_sensitive(False)
        self.remote_entry.set_sensitive(False)
        self.records.insert(0, [datetime.now().strftime("%H:%M:%S"), title, "Läuft …"])
        record = self.records[0]
        self.render_activity()
        future = self.executor.submit(work)
        future.add_done_callback(lambda result: GLib.idle_add(self._completed, result, title, done, record))

    def _completed(self, future, title, done, record):
        self.busy = False
        self.spinner.stop()
        self.cancel_button.set_visible(False)
        self.login_button.set_visible(False)
        self.login_url = None
        self.file_list.set_sensitive(True)
        self.plan_button.set_sensitive(True)
        self.local_choose.set_sensitive(True)
        self.remote_entry.set_sensitive(True)
        self.update_actions()
        try:
            value = future.result()
        except Exception as exc:
            text = str(exc) if isinstance(exc, (BackendError, SyncError)) else f"Interner Fehler ({type(exc).__name__}). Es wurde kein Erfolg angenommen."
            record[2] = "Fehlgeschlagen · " + text
            self.status_label.set_text(title + " · nicht abgeschlossen")
            self.message(text, error=True)
        else:
            record[2] = "Abgeschlossen"
            self.status_label.set_text(title + " · abgeschlossen")
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
            self.window.destroy()
            self.quit()
        return GLib.SOURCE_REMOVE

    def render_activity(self):
        self.clear_box(self.activity_list)
        for timestamp, title, status in self.records[:100]:
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
            box.append(label(timestamp + "   " + title, "file-name"))
            description = label(status, "muted small")
            description.set_wrap(True)
            box.append(description)
            self.activity_list.append(box)

    def open_url(self, url):
        try:
            Gio.AppInfo.launch_default_for_uri(url, self.window.get_display().get_app_launch_context())
        except GLib.Error:
            self.message("Der Browser konnte nicht geöffnet werden. Bitte die URL manuell aufrufen: " + url.split("?")[0], error=True)

    def open_login(self):
        if self.login_url:
            self.open_url(self.login_url)

    def login(self):
        self.message("Die Anmeldung erfolgt bei Proton im Browser. Diese App ist unabhängig und wird nicht von Proton unterstützt.")
        def on_url(url):
            GLib.idle_add(self._login_ready, url)
        self.submit("Browser-Anmeldung", lambda: self.backend.login(on_url), lambda _: self.refresh())

    def _login_ready(self, url):
        self.login_url = url
        self.login_button.set_visible(True)
        self.message("Bitte die Anmeldung im geöffneten Proton-Browserfenster abschließen. Falls kein Fenster erscheint, den Anmeldeknopf verwenden.")
        return GLib.SOURCE_REMOVE

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
        self.backend_label.set_text(str(value))

    def choose(self, title, *, folder=False, multiple=False, callback):
        chooser = Gtk.FileChooserNative.new(title, self.window,
                    Gtk.FileChooserAction.SELECT_FOLDER if folder else Gtk.FileChooserAction.OPEN, "Auswählen", "Abbrechen")
        chooser.set_select_multiple(multiple)
        def response(dialog, code):
            try:
                if code == Gtk.ResponseType.ACCEPT:
                    files = dialog.get_files()
                    paths = [Path(files.get_item(i).get_path()) for i in range(files.get_n_items()) if files.get_item(i).get_path()]
                    if paths:
                        callback(paths)
            finally:
                dialog.destroy()
        chooser.connect("response", response)
        chooser.show()

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

    def share_dialog(self, node, value):
        dialog = Gtk.Dialog(title="Freigabe · " + node.name, transient_for=self.window, modal=True)
        dialog.set_default_size(560, -1)
        dialog.add_button("Schließen", Gtk.ResponseType.CLOSE)
        box = margin(dialog.get_content_area(), 20)
        box.set_spacing(14)
        access = (value or {}).get("urlAccess") or {}
        url = access.get("url", "")
        if url:
            box.append(label("Jeder mit diesem Link kann auf die Datei zugreifen.", "muted"))
            entry = Gtk.Entry(text=url, editable=False)
            box.append(entry)
            box.append(button("Link kopieren", callback=lambda: self.window.get_clipboard().set(url)))
            def revoke():
                dialog.destroy()
                self.confirm("Freigabelink deaktivieren?", "Der bisherige Link funktioniert anschließend nicht mehr.",
                             lambda: self.submit("Freigabelink entfernen", lambda: self.backend.remove_link(node.path), lambda _: self.message("Freigabelink entfernt.")))
            box.append(button("Link deaktivieren", callback=revoke))
        else:
            info = label("Es besteht kein öffentlicher Link. Beim Erstellen erhält jeder mit dem Link Lesezugriff. Für Passwort, Ablaufdatum und Mitgliederverwaltung die Proton-Web-App verwenden.", "muted")
            info.set_wrap(True)
            box.append(info)
            def create():
                dialog.destroy()
                self.submit("Freigabelink erstellen", lambda: self.backend.create_link(node.path), lambda result: self.share_dialog(node, result))
            box.append(button("Öffentlichen Leselink erstellen", callback=create, css="suggested-action"))
        dialog.connect("response", lambda d, *_: d.destroy())
        dialog.present()

    def prompt(self, title, description, initial, callback):
        dialog = Gtk.Dialog(title=title, transient_for=self.window, modal=True)
        dialog.add_button("Abbrechen", Gtk.ResponseType.CANCEL)
        dialog.add_button("Übernehmen", Gtk.ResponseType.OK)
        dialog.set_default_response(Gtk.ResponseType.OK)
        box = margin(dialog.get_content_area(), 20)
        box.set_spacing(12)
        box.append(label(description, "muted"))
        entry = Gtk.Entry(text=initial, activates_default=True)
        entry.set_width_chars(45)
        box.append(entry)
        def response(d, code):
            value = entry.get_text()
            d.destroy()
            if code == Gtk.ResponseType.OK:
                callback(value)
        dialog.connect("response", response)
        dialog.present()
        entry.grab_focus()

    def confirm(self, title, description, callback, destructive=False):
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=True, message_type=Gtk.MessageType.QUESTION,
                                   buttons=Gtk.ButtonsType.NONE, text=title, secondary_text=description)
        dialog.add_button("Abbrechen", Gtk.ResponseType.CANCEL)
        accept = dialog.add_button("Bestätigen", Gtk.ResponseType.OK)
        if destructive:
            accept.add_css_class("destructive-action")
        def response(d, code):
            d.destroy()
            if code == Gtk.ResponseType.OK:
                callback()
        dialog.connect("response", response)
        dialog.present()

    def choose_sync_local(self):
        def chosen(paths):
            self.sync_local = paths[0]
            self.local_label.set_text(str(paths[0]))
            self.invalidate_plan()
        self.choose("Lokalen Sync-Ordner auswählen", folder=True, callback=chosen)

    def invalidate_plan(self):
        self.sync_plan = None
        if hasattr(self, "execute_button"):
            self.execute_button.set_sensitive(False)

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
        local, remote = self.sync_local, self.remote_entry.get_text()
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
        self.sync_text.get_buffer().set_text("\n".join(lines))
        # Executing a plan with no transfers can still establish the baseline.
        self.execute_button.set_sensitive(True)

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
        self.sync_text.get_buffer().set_text("\n".join(lines))
        if self.records:
            self.records[0][2] = lines[0]
        self.status_label.set_text(lines[0])
        self.message(lines[0], error=bool(result.errors or result.conflicts))

    def _close(self, *_):
        if self.busy:
            def stop():
                self.closing = True
                self.backend.cancel()
                self.status_label.set_text("Vorgang wird beendet …")
            self.confirm("Laufenden Vorgang abbrechen?", "Bereits übertragene Dateien bleiben erhalten. Die App wartet auf das Ende des Backends.", stop)
            return True
        if os.environ.get("FLATPAK_ID") and not self.demo:
            try:
                if self.background_clients()[1].status().get("running"):
                    # Keep the sandbox's primary process alive while its worker
                    # is active. Reopening the app presents this same window.
                    self.window.hide()
                    return True
            except (BackendError, SyncError):
                pass
        self.closing = True
        self.executor.shutdown(wait=False, cancel_futures=True)
        self.status_executor.shutdown(wait=False, cancel_futures=True)
        return False
