"""Narrow session-bus interface for native on-demand filesystem integration.

A Flatpak may speak only to this application-specific service. It receives no
host command execution, shell access, credentials or raw backend responses.
"""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time

from . import APP_ID
from .backend import Backend, BackendError, data_dir
from .ondemand import _atomic_json, _private_dir, _read_json

# A confined Snap must never accidentally bind to an installed native helper
# with another state directory/keyring context. Native and Flatpak use .Service.
BUS_NAME = APP_ID + ('.SnapService' if os.environ.get('SNAP') else '.Service')
OBJECT_PATH = '/io/github/SigmaCodez/NeutronDrive/Service'
INTERFACE = BUS_NAME
INTROSPECTION = f'''<node><interface name="{INTERFACE}">
<method name="Request"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
</interface></node>'''
MAX_REQUEST = 16384


def _gio():
    import gi
    from gi.repository import Gio, GLib
    return Gio, GLib


class ServiceCore:
    """Testable operations behind the bus; all paths are user-selected data."""
    def __init__(self, backend, state_dir=None, manager_factory=None, event_factory=None):
        self.backend = backend
        self.state_dir = _private_dir(Path(state_dir or data_dir() / 'ondemand-service'))
        if manager_factory is None:
            from .fuse import MountManager
            manager_factory = MountManager
        self.manager = manager_factory(backend, self.state_dir)
        self.configuration = self.state_dir / 'mount.json'
        self.events = None
        self.event_factory = event_factory
        self._event_error = ''
        self._operation = ''
        self._request_lock = threading.RLock()

    def _save(self, value):
        _atomic_json(self.configuration, value)

    def configuration_value(self):
        if not self.configuration.exists() and not self.configuration.is_symlink():
            return {}
        if self.configuration.lstat().st_size > MAX_REQUEST:
            raise BackendError('Unsichere Laufwerkskonfiguration.')
        try:
            value = _read_json(self.configuration)
        except (ValueError, OSError) as exc:
            raise BackendError('Unsichere oder unlesbare Laufwerkskonfiguration.') from exc
        allowed = {'mountpoint', 'remote_root', 'writable', 'offline', 'restore'}
        if not isinstance(value, dict) or set(value) - allowed:
            raise BackendError('Ungültige Laufwerkskonfiguration.')
        return value

    def status(self):
        return {'available': True, **self.manager.status(), 'configuration': self.configuration_value(),
                'operation': self._operation, 'busy': bool(self._operation),
                'events': self.events.status if self.events else
                          {'state': 'error' if self._event_error else 'stopped', 'error': self._event_error}}

    def _start_events(self, root):
        self._event_error = ''
        factory = self.event_factory
        if factory is None:
            from .events import EventSource
            factory = EventSource
        # EventSource.stop() cancels its Backend. Share serialization with FUSE,
        # but never share the cancellation flag with ongoing file operations.
        event_backend = self.backend
        if isinstance(self.backend, Backend):
            event_backend = Backend(self.backend.executable)
            event_backend._mutex = self.backend._mutex
        try:
            events = factory(event_backend, self.state_dir / 'events', roots=(root,))
            self.events = events
            events.start(lambda event: self.manager.invalidate(event) if event.kind != 'error' else None)
        except Exception:
            self._event_error = 'Drive-Ereignisse sind nicht verfügbar. Das Laufwerk bleibt über Aktualisieren nutzbar.'
            if self.events:
                try:
                    self.events.stop()
                except Exception:
                    pass
                if not getattr(self.events, 'running', False):
                    self.events = None

    def _stop_events(self):
        if self.events:
            self.events.stop()
            if getattr(self.events, 'running', False):
                raise BackendError('Die Ereignisüberwachung beendet noch einen laufenden Abruf. Bitte erneut versuchen.')
            self.events = None

    def request(self, request):
        if not isinstance(request, dict) or not isinstance(request.get('action'), str):
            raise BackendError('Ungültige Dienstanfrage.')
        if request['action'] == 'status':
            return self.status()
        with self._request_lock:
            self._operation = request['action']
            try:
                result = self._request(request)
                if isinstance(result, dict) and 'busy' in result:
                    result = {**result, 'busy': False, 'operation': ''}
                return result
            finally:
                self.backend.reset_cancel()
                self._operation = ''

    def _request(self, request):
        if not isinstance(request, dict) or not isinstance(request.get('action'), str):
            raise BackendError('Ungültige Dienstanfrage.')
        action = request['action']
        if action == 'status':
            return self.status()
        if action == 'mount':
            allowed = {'action', 'mountpoint', 'remote_root', 'writable', 'offline'}
            if set(request)-allowed or not isinstance(request.get('mountpoint'), str):
                raise BackendError('Ungültige Laufwerksoptionen.')
            for key in ('writable','offline'):
                if key in request and not isinstance(request[key], bool):
                    raise BackendError('Ungültige Laufwerksoptionen.')
            root = request.get('remote_root','/my-files')
            if not isinstance(root,str) or not (root == '/my-files' or root.startswith('/my-files/')):
                raise BackendError('Online-Laufwerke benötigen einen Ordner unter /my-files.')
            config = {'mountpoint':request['mountpoint'], 'remote_root':root,
                      'writable':request.get('writable',False), 'offline':request.get('offline',False)}
            self.backend.reset_cancel()
            # Avoid a local sync job recursively reading its own virtual drive.
            from .background import JobStore
            point = Path(config['mountpoint']).absolute()
            for job in JobStore(data_dir() / 'background').list():
                local = Path(job.local_root)
                if point == local or point in local.parents or local in point.parents:
                    raise BackendError('Online-Laufwerk und Sync-Ordner dürfen sich nicht überlappen.')
            if self.manager.status().get('mounted'):
                raise BackendError('Es ist bereits ein Online-Laufwerk eingehängt.')
            self._stop_events()
            self.manager.mount(**config)
            try:
                self._save({**config, 'restore':True})
            except Exception:
                self.manager.unmount()
                raise
            if not config['offline']:
                self._start_events(root)
            return self.status()
        if action == 'unmount':
            # A busy mount stays live with its event watcher and cancellation
            # state intact. Stop the watcher only after successful unmount.
            self.manager.unmount()
            self._save({**self.configuration_value(), 'restore':False})
            self._stop_events()
            return self.status()
        if action in {'pin','evict'}:
            path = request.get('path')
            if not isinstance(path,str) or len(path)>4096:
                raise BackendError('Ungültiger relativer Dateipfad.')
            self.backend.reset_cancel()
            if action == 'pin':
                self.manager.pin(path, pinned=True)
            else:
                self.manager.evict(path)
            return self.manager.status()
        if action == 'refresh':
            self.backend.reset_cancel()
            self.manager.refresh()
            return self.manager.status()
        if action == 'restore':
            config=self.configuration_value()
            if config.pop('restore',False) and not self.manager.status().get('mounted'):
                return self.request({'action':'mount',**config})
            return self.manager.status()
        if action == 'login':
            self.backend.reset_cancel()
            if self.manager.status().get('mounted'):
                raise BackendError('Vor einem Kontowechsel das Online-Laufwerk aushängen.')
            self.backend.login(lambda _url: None)  # The CLI opens exactly one browser window.
            self.backend.info('/my-files')
            return {'authenticated':True}
        raise BackendError('Unbekannte Dienstaktion.')


class DesktopClient:
    def __init__(self, executable=None):
        self.executable = executable

    def request(self, action, **options):
        Gio, GLib = _gio()
        payload = json.dumps({'action': action, **options})
        if len(payload.encode()) > MAX_REQUEST:
            raise BackendError('Dienstanfrage ist zu groß.')
        flags = (Gio.DBusProxyFlags.DO_NOT_LOAD_PROPERTIES | Gio.DBusProxyFlags.DO_NOT_CONNECT_SIGNALS |
                 Gio.DBusProxyFlags.DO_NOT_AUTO_START_AT_CONSTRUCTION)
        if action == 'status':
            flags |= Gio.DBusProxyFlags.DO_NOT_AUTO_START
        try:
            proxy = Gio.DBusProxy.new_for_bus_sync(Gio.BusType.SESSION, flags, None, BUS_NAME, OBJECT_PATH, INTERFACE, None)
        except GLib.Error as exc:
            raise BackendError('Der Sitzungsbus ist nicht erreichbar.') from exc
        if action == 'status' and not proxy.get_name_owner():
            return {'available': False, 'mounted': False}
        try:
            answer = proxy.call_sync('Request', GLib.Variant('(s)', (payload,)), Gio.DBusCallFlags.NONE,
                                     5000 if action == 'status' else 650000, None)
        except GLib.Error as exc:
            remote_error = Gio.DBusError.get_remote_error(exc)
            missing = remote_error in {'org.freedesktop.DBus.Error.ServiceUnknown', 'org.freedesktop.DBus.Error.NameHasNoOwner'}
            # Only a request that was never delivered may start a source helper
            # and retry. Never replay a mutation after NoReply/Disconnected.
            if not os.environ.get('FLATPAK_ID') and missing:
                env = os.environ.copy()
                source = str(Path(__file__).resolve().parents[1])
                env['PYTHONPATH'] = source + (os.pathsep + env['PYTHONPATH'] if env.get('PYTHONPATH') else '')
                command = [sys.executable, '-m', 'neutrondrive', '--desktop-service']
                if self.executable:
                    command += ['--cli', self.executable]
                try:
                    process = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                               stderr=subprocess.DEVNULL, start_new_session=True)
                    threading.Thread(target=process.wait, name='desktop-service-reaper', daemon=True).start()
                    for _ in range(40):
                        time.sleep(.1)
                        proxy = Gio.DBusProxy.new_for_bus_sync(Gio.BusType.SESSION, flags, None, BUS_NAME, OBJECT_PATH, INTERFACE, None)
                        if proxy.get_name_owner():
                            break
                        if process.poll() is not None:
                            raise BackendError('Der native Laufwerksdienst wurde vorzeitig beendet.')
                    else:
                        raise BackendError('Der native Laufwerksdienst konnte nicht starten. FUSE-Abhängigkeiten prüfen.')
                    answer = proxy.call_sync('Request', GLib.Variant('(s)', (payload,)), Gio.DBusCallFlags.NONE, 650000, None)
                except (GLib.Error, OSError) as retry_error:
                    raise BackendError('Der native Laufwerksdienst ist nicht erreichbar. Status vor erneutem Versuch prüfen.') from retry_error
            else:
                raise BackendError('Der native NeutronDrive-Helfer ist nicht erreichbar. Für Flatpak das passende native Paket installieren. Bei einer unterbrochenen Aktion zuerst den Status prüfen.') from exc
        try:
            result = json.loads(answer.unpack()[0])
            if not isinstance(result, dict) or not isinstance(result.get('ok'), bool):
                raise ValueError('invalid response')
        except (ValueError, TypeError, IndexError) as exc:
            raise BackendError('Der Laufwerksdienst lieferte eine ungültige Antwort.') from exc
        if not result['ok']:
            raise BackendError(result.get('error', 'Laufwerksaktion fehlgeschlagen.'))
        value = result.get('result', {})
        if not isinstance(value, dict):
            raise BackendError('Der Laufwerksdienst lieferte einen ungültigen Status.')
        return value


def run_service(backend=None, *, core=None):
    if os.environ.get('FLATPAK_ID'):
        raise BackendError('Der Laufwerkshelfer muss nativ außerhalb von Flatpak laufen.')
    Gio, GLib = _gio()
    backend = backend or (core.backend if core is not None else Backend())
    core = core or ServiceCore(backend)
    loop=GLib.MainLoop()
    executor=ThreadPoolExecutor(max_workers=1,thread_name_prefix='ondemand-service')
    registrations=[]

    def completed(future,invocation):
        try:
            response={'ok':True,'result':future.result()}
        except Exception as exc:
            # Do not send raw process output, credentials, or tracebacks.
            from .ondemand import OnDemandError
            message=str(exc) if isinstance(exc,(BackendError,OnDemandError,OSError)) else f'Laufwerksaktion fehlgeschlagen ({type(exc).__name__}).'
            response={'ok':False,'error':message}
        invocation.return_value(GLib.Variant('(s)',(json.dumps(response),)))
        return GLib.SOURCE_REMOVE

    def called(connection,sender,path,interface,method,parameters,invocation):
        try:
            identity=connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixUser',GLib.Variant('(s)',(sender,)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,3000,None)
            if identity.unpack()[0]!=os.getuid():
                raise BackendError('Dienstzugriff verweigert.')
            raw=parameters.unpack()[0]
            if len(raw.encode())>MAX_REQUEST:
                raise BackendError('Dienstanfrage ist zu groß.')
            request=json.loads(raw)
            if not isinstance(request, dict) or not isinstance(request.get('action'), str):
                raise BackendError('Ungültige Dienstanfrage.')
            if request.get('action') == 'status':
                invocation.return_value(GLib.Variant('(s)', (json.dumps({'ok': True, 'result': core.status()}),)))
                return
            if request.get('action')=='cancel':
                if core._operation:
                    backend.cancel()
                invocation.return_value(GLib.Variant('(s)',(json.dumps({'ok':True,'result':{}}),)))
                return
        except Exception:
            invocation.return_dbus_error(INTERFACE+'.InvalidRequest','Ungültige Dienstanfrage.')
            return
        future=executor.submit(core.request,request)
        future.add_done_callback(lambda f: GLib.idle_add(completed,f,invocation))

    def acquired(connection,_name):
        info=Gio.DBusNodeInfo.new_for_xml(INTROSPECTION)
        registrations.append((connection,connection.register_object(OBJECT_PATH,info.interfaces[0],called,None,None)))

    owner=Gio.bus_own_name(Gio.BusType.SESSION,BUS_NAME,Gio.BusNameOwnerFlags.NONE,acquired,None,lambda *_:loop.quit())
    def stop(*_):
        backend.cancel()
        loop.quit()
    previous={sig:signal.signal(sig,stop) for sig in (signal.SIGTERM,signal.SIGINT)}
    try:
        loop.run()
    finally:
        backend.cancel()
        if core.events:
            core.events.stop()
        executor.shutdown(wait=True,cancel_futures=True)
        try:
            core.manager.unmount()
        finally:
            for connection,registration in registrations:
                connection.unregister_object(registration)
            Gio.bus_unown_name(owner)
            for sig,handler in previous.items():
                signal.signal(sig,handler)
    return 0
