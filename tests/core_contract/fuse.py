"""Temporary Python call/thread binding for the owned Rust mount manager.

Validation, lifecycle, status, mount identity, recovery, relative-path commands
and event invalidation are native. Qt and Gio callers retain their existing API.
"""
import threading

from .backend import service_call
from .ondemand import _unwrap
from .rust_core import backend_value


def availability():
    return tuple(_unwrap(backend_value('mount_manager', operation={'action': 'availability'})))


class MountManager:
    def __init__(self, backend, state_dir):
        self.backend = backend
        self._thread = None
        self._id = _unwrap(backend_value('mount_manager', operation={
            'action': 'create', 'state_dir': str(state_dir)}))

    def _call(self, action, **arguments):
        return _unwrap(service_call(self.backend, 'mount_manager', {
            'action': action, 'id': self._id, **arguments}))

    def mount(self, mountpoint, remote_root='/my-files', writable=False, offline=False):
        self._call('prepare', mountpoint=str(mountpoint), remote_root=remote_root,
                   writable=bool(writable), offline=bool(offline))
        try:
            self._thread = threading.Thread(target=self._run, name='neutrondrive-fuse', daemon=True)
            self._thread.start()
        except BaseException:
            self._call('abort')
            raise
        return self._call('wait')

    start = mount

    def _run(self):
        # Native lifecycle records failure and releases filesystem resources.
        # The initiating wait/status call exposes the error to the caller.
        try:
            self._call('run')
        except Exception:
            pass

    def unmount(self):
        return self._call('unmount')

    stop = unmount

    def status(self): return self._call('status')

    def pin(self, relative_path, pinned=True):
        return self._call('pin', path=str(relative_path), pinned=bool(pinned))

    def evict(self, relative_path): return self._call('evict', path=str(relative_path))

    def refresh(self): return self._call('refresh')

    def invalidate(self, event=None):
        return self._call('invalidate', root_uid=getattr(event, 'root_uid', None),
                          kind=getattr(event, 'kind', ''))

    def __del__(self):
        if hasattr(self, '_id'):
            try:
                backend_value('mount_manager', operation={'action': 'forget', 'id': self._id})
            except Exception:
                pass


def run_mount(backend, mountpoint, state_dir, remote_root='/my-files', writable=False, offline=False):
    manager = MountManager(backend, state_dir)
    manager.mount(mountpoint, remote_root, writable, offline)
    try:
        while manager._thread.is_alive():
            manager._thread.join(0.5)
    except KeyboardInterrupt:
        manager.unmount()
    return manager.status()
