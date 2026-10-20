"""User-session startup integration; never installs a system/root daemon."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import sys
import tempfile
import stat

from . import APP_ID
from .backend import BackendError


def app_command(*arguments: str) -> list[str]:
    """Return the installed launcher, including its package boundary."""
    if os.environ.get('FLATPAK_ID'):
        return ['flatpak', 'run', APP_ID, *arguments]
    if os.environ.get('SNAP'):
        # snap userd replaces argv[0] with this app's snap wrapper. Additional
        # `snap run neutrondrive` arguments would incorrectly reach argparse.
        return ['neutrondrive', *arguments]
    launcher = shutil.which('neutrondrive')
    if launcher:
        return [str(Path(launcher).absolute()), *arguments]
    # Running from source remains supported without depending on PYTHONPATH
    # inherited from the interactive shell at the next desktop login.
    source = Path(__file__).resolve().parents[2] / 'bin/neutrondrive'
    if source.is_file():
        return [sys.executable, str(source), *arguments]
    raise BackendError('Kein installierter NeutronDrive-Starter gefunden.')


def desktop_quote(argument: str) -> str:
    if any(c in argument for c in '\n\r\x00'):
        raise BackendError('Ungültiges Zeichen im Autostart-Pfad.')
    escaped = argument.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
    # Desktop string-value unescaping happens before Exec argument parsing.
    escaped = escaped.replace('\\', '\\\\')
    return '"' + escaped + '"'


def autostart_path() -> Path:
    if os.environ.get('SNAP'):
        data = os.environ.get('SNAP_USER_DATA')
        if not data:
            raise BackendError('SNAP_USER_DATA fehlt für den Snap-Autostart.')
        return Path(data) / '.config/autostart' / (APP_ID + '.desktop')
    return Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config'))) / 'autostart' / (APP_ID + '.desktop')


def _read_autostart(path):
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise BackendError('Der Autostart-Pfad enthält einen symbolischen Link.')
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return None
    with os.fdopen(fd, 'r') as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1 or info.st_size > 65536:
            raise BackendError('Unsicherer Autostart-Eintrag.')
        return stream.read()


def autostart_enabled() -> bool:
    try:
        content = _read_autostart(autostart_path())
    except (BackendError, OSError, UnicodeError):
        return False
    return bool(content and 'X-NeutronDrive-Managed=true' in content.splitlines())


def set_autostart(enabled: bool, cli: str | None = None) -> None:
    """Native XDG autostart. Flatpak uses the Background portal in the GUI."""
    if os.environ.get('FLATPAK_ID'):
        raise BackendError('Flatpak-Autostart wird über das Desktop-Portal eingerichtet.')
    path = autostart_path()
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise BackendError('Der Autostart-Pfad enthält einen symbolischen Link.')
    previous = _read_autostart(path)
    if previous is not None and 'X-NeutronDrive-Managed=true' not in previous.splitlines():
        raise BackendError('Ein bestehender fremder Autostart-Eintrag wird nicht überschrieben.')
    if not enabled:
        path.unlink(missing_ok=True)
        return
    arguments = ['--background']
    if cli and not os.environ.get('SNAP'):
        arguments += ['--cli', str(Path(cli).absolute())]
    command = app_command(*arguments)
    content = ('[Desktop Entry]\nType=Application\nName=NeutronDrive Hintergrunddienst\n'
               'Comment=Gespeicherte Synchronisierung und Online-Laufwerke starten\n'
               'Exec=' + ' '.join(desktop_quote(p) for p in command) + '\n'
               'Terminal=false\nX-GNOME-Autostart-enabled=true\nX-NeutronDrive-Managed=true\n')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(prefix='.neutrondrive-', suffix='.desktop.tmp', dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def request_portal_background(enabled, callback):
    """Ask the desktop's Background portal; success is its Response, not dispatch."""
    from gi.repository import Gio, GLib
    import uuid
    token='neutrondrive_' + uuid.uuid4().hex
    try:
        connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
        sender=connection.get_unique_name()[1:].replace('.','_')
        request_path=f'/org/freedesktop/portal/desktop/request/{sender}/{token}'
        subscription=[None]
        completed=[False]
        timer=[None]
        def finish(ok):
            if completed[0]:
                return
            completed[0]=True
            if subscription[0] is not None:
                connection.signal_unsubscribe(subscription[0])
            if timer[0] is not None:
                GLib.source_remove(timer[0])
                timer[0] = None
            callback(ok)
        def response(_connection,_sender,_path,_interface,_signal,parameters):
            code,results=parameters.unpack()
            finish(code == 0 and (not enabled or bool(results.get('background'))) and bool(results.get('autostart')) == enabled)
        subscription[0]=connection.signal_subscribe('org.freedesktop.portal.Desktop','org.freedesktop.portal.Request','Response',request_path,None,Gio.DBusSignalFlags.NONE,response)
        options={'handle_token':GLib.Variant('s',token),'reason':GLib.Variant('s','Ausgewählte Proton-Drive-Ordner automatisch synchronisieren'),
                 'autostart':GLib.Variant('b',enabled),'commandline':GLib.Variant('as',['neutrondrive','--background']),
                 'dbus-activatable':GLib.Variant('b',False)}
        def returned(conn,result):
            try:
                returned_path = conn.call_finish(result).unpack()[0]
                if returned_path != request_path and not completed[0]:
                    connection.signal_unsubscribe(subscription[0])
                    subscription[0] = connection.signal_subscribe('org.freedesktop.portal.Desktop', 'org.freedesktop.portal.Request', 'Response', returned_path, None, Gio.DBusSignalFlags.NONE, response)
            except GLib.Error:
                finish(False)
        def expired():
            timer[0] = None
            finish(False)
            return GLib.SOURCE_REMOVE
        timer[0] = GLib.timeout_add_seconds(120, expired)
        connection.call('org.freedesktop.portal.Desktop','/org/freedesktop/portal/desktop','org.freedesktop.portal.Background','RequestBackground',
                        GLib.Variant('(sa{sv})',('',options)),GLib.VariantType.new('(o)'),Gio.DBusCallFlags.NONE,120000,None,returned)
    except GLib.Error:
        callback(False)
