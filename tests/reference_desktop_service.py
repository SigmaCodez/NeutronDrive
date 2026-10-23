"""Test-only worker injection and reference Gio transport for native core contracts."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import signal
import sys

from core_contract import APP_ID
from core_contract.backend import Backend, BackendError, data_dir

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
    """Temporary scoped worker binding; service decisions and state live in Rust."""
    def __init__(self, backend, state_dir=None, manager_factory=None, event_factory=None):
        from core_contract.rust_core import backend_value
        self.backend = backend
        self.state_dir = Path(state_dir or data_dir() / 'ondemand-service')
        self._id = backend_value('desktop_service', operation=dict(action='create', state_dir=str(self.state_dir)))
        if manager_factory is None:
            from core_contract.fuse import MountManager
            manager_factory = MountManager
        self.manager = manager_factory(backend, self.state_dir)
        self.configuration = self.state_dir / 'mount.json'
        self.events = None
        self.event_factory = event_factory

    def _call(self, action, **arguments):
        from core_contract.rust_core import backend_value
        from core_contract.backend import Cancelled
        errors = []
        def event(value):
            try:
                result = self._host(value['method'], value['arguments'])
            except BaseException as error:
                errors.append(error)
                backend_value('host_failure', id=value['id'], message=str(error), cancelled=isinstance(error, Cancelled))
            else:
                backend_value('host_reply', id=value['id'], value=result)
        try:
            return backend_value('desktop_service', operation=dict(action=action, id=self._id, **arguments), on_event=event)
        except BackendError:
            if errors:
                raise errors[-1]
            raise

    def _host(self, method, args):
        if method == 'manager_status': return self.manager.status()
        if method == 'manager_mount': return self.manager.mount(**args)
        if method == 'manager_unmount': return self.manager.unmount()
        if method == 'manager_pin': return self.manager.pin(args['path'], pinned=True)
        if method == 'manager_evict': return self.manager.evict(args['path'])
        if method == 'manager_refresh': return self.manager.refresh()
        if method == 'save': return self._save(args)
        if method == 'reset_cancel': return self.backend.reset_cancel()
        if method == 'login':
            self.backend.login(lambda _url: None)
            self.backend.info('/my-files')
            return None
        if method == 'events_snapshot':
            return dict(exists=self.events is not None, running=bool(getattr(self.events, 'running', False)),
                        status=self.events.status if self.events else None)
        if method == 'events_drop':
            self.events = None
            return None
        if method == 'events_stop': return self.events.stop()
        if method == 'events_start':
            factory = self.event_factory
            if factory is None:
                from core_contract.events import EventSource
                factory = EventSource
            event_backend = self.backend
            if isinstance(self.backend, Backend):
                event_backend = Backend(self.backend.executable)
                event_backend._mutex = self.backend._mutex
            self.events = factory(event_backend, self.state_dir / 'events', roots=(args['root'],))
            self.events.start(lambda event: self.manager.invalidate(event) if event.kind != 'error' else None)
            return None
        raise BackendError('Unexpected native desktop service request')

    def _save(self, value): return self._call('save', value=value)
    def configuration_value(self): return self._call('configuration')
    def status(self): return self._call('status')
    def request(self, request): return self._call('request', request=request)
    @property
    def _operation(self): return self._call('current')

    def __del__(self):
        if hasattr(self, '_id'):
            try:
                self._call('forget')
            except Exception:
                pass


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
            from core_contract.ondemand import OnDemandError
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
