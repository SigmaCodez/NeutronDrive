"""User-session startup integration; never installs a system/root daemon."""
from __future__ import annotations

import os
from pathlib import Path

from .backend import BackendError


def _native(action, **values):
    from .rust_core import backend_value
    return backend_value('autostart', operation=dict(action=action, **values))


def app_command(*arguments: str) -> list[str]:
    return _native('command', arguments=list(arguments),
                   source_launcher=str(Path(__file__).resolve().parents[2] / 'bin/neutrondrive'),
                   interpreter='/bin/sh')


def desktop_quote(argument: str) -> str:
    return _native('quote', argument=argument)


def autostart_path() -> Path:
    return Path(_native('path'))


def _read_autostart(path):
    return _native('read', path=str(path))


def autostart_enabled() -> bool:
    try:
        return _native('enabled', path=str(autostart_path()))
    except BackendError:
        return False


def set_autostart(enabled: bool, cli: str | None = None) -> None:
    arguments = ['--background']
    if cli and not os.environ.get('SNAP'):
        arguments += ['--cli', str(Path(cli).absolute())]
    _native('set', enabled=enabled, path=str(autostart_path()),
            command=app_command(*arguments) if enabled else [])


def request_portal_background(enabled, callback):
    """Temporary main-context callback binding for the native portal transaction."""
    import threading
    from gi.repository import GLib
    from .rust_core import backend_value
    def work():
        try:
            accepted = backend_value('portal', operation=dict(action='background', enabled=enabled))
        except BackendError:
            accepted = False
        def finished():
            callback(accepted)
            return GLib.SOURCE_REMOVE
        GLib.idle_add(finished)
    try:
        threading.Thread(target=work, name='background-portal-binding', daemon=True).start()
    except RuntimeError:
        def failed():
            callback(False)
            return GLib.SOURCE_REMOVE
        GLib.idle_add(failed)
