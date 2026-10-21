"""Follow desktop appearance without changing the user's theme preference."""
from PySide6.QtCore import QObject, Property, Signal, Qt


class SystemAppearance(QObject):
    changed = Signal()

    def __init__(self, application, parent=None):
        super().__init__(parent)
        self.application = application
        self._portal_scheme = 0
        self._generation = 0
        self._closed = False
        self._connection = None
        self._subscription = None
        self._cancel = None
        application.styleHints().colorSchemeChanged.connect(self._qt_changed)
        application.paletteChanged.connect(self._qt_changed)
        self._start_portal()

    def _qt_changed(self, *args):
        self.changed.emit()

    @Property(bool, notify=changed)
    def dark(self):
        if self._portal_scheme in (1, 2):
            return self._portal_scheme == 1
        scheme = self.application.styleHints().colorScheme()
        if scheme != Qt.ColorScheme.Unknown:
            return scheme == Qt.ColorScheme.Dark
        return self.application.palette().window().color().lightnessF() < 0.5

    def _apply_portal(self, value):
        if self._closed or type(value) is not int or value not in (0, 1, 2):
            return
        self._generation += 1
        self._portal_scheme = value
        self.changed.emit()

    def _start_portal(self):
        # Bundled Qt need not have the host's GTK/KDE theme plugin. The Settings
        # portal provides a desktop-neutral signal, including in Flatpak/Snap.
        try:
            from gi.repository import Gio, GLib
        except ImportError:
            return
        self._cancel = Gio.Cancellable()

        def changed(connection, sender, path, interface, signal, parameters, data):
            namespace, key, value = parameters.unpack()
            if namespace == 'org.freedesktop.appearance' and key == 'color-scheme':
                self._apply_portal(value)

        def connected(source, result):
            try:
                connection = Gio.bus_get_finish(result)
            except GLib.Error:
                return
            if self._closed:
                return
            self._connection = connection
            self._subscription = connection.signal_subscribe(
                'org.freedesktop.portal.Desktop', 'org.freedesktop.portal.Settings',
                'SettingChanged', '/org/freedesktop/portal/desktop',
                'org.freedesktop.appearance', Gio.DBusSignalFlags.NONE, changed, None)
            generation = self._generation

            def read_done(connection, result):
                try:
                    value = connection.call_finish(result).unpack()[0]
                except GLib.Error:
                    return
                # A newer change notification wins over the initial read.
                if generation == self._generation:
                    self._apply_portal(value)

            connection.call('org.freedesktop.portal.Desktop',
                '/org/freedesktop/portal/desktop', 'org.freedesktop.portal.Settings',
                'Read', GLib.Variant('(ss)', ('org.freedesktop.appearance', 'color-scheme')),
                GLib.VariantType.new('(v)'), Gio.DBusCallFlags.NONE, 3000,
                self._cancel, read_done)

        Gio.bus_get(Gio.BusType.SESSION, self._cancel, connected)

    def close(self):
        self._closed = True
        if self._cancel:
            self._cancel.cancel()
        if self._connection is not None and self._subscription is not None:
            self._connection.signal_unsubscribe(self._subscription)
            self._subscription = None
        self.application.styleHints().colorSchemeChanged.disconnect(self._qt_changed)
        self.application.paletteChanged.disconnect(self._qt_changed)
