"""Temporary FUSE/desktop binding for the owned Rust online-filesystem runtime.

All mount state, file lifecycle, cache, recovery and cloud transactions are native.
This module adapts bytes/DTOs and serializes calls from the transitional desktop bridge.
"""
from dataclasses import dataclass
import os
from pathlib import Path
import threading

from .backend import BackendError, Cancelled, Node, service_call


class OnDemandError(OSError):
    """An errno suitable for returning to a filesystem caller."""


def _unwrap(value):
    if 'errno' in value:
        raise OnDemandError(value['errno'], value['message'])
    if 'backend_error' in value:
        raise (Cancelled if value.get('cancelled') else BackendError)(value['backend_error'])
    return value.get('value')


def _io(action, **arguments):
    from .rust_core import backend_value
    return _unwrap(backend_value('ondemand_io', operation={'action': action, **arguments}))


def _key(node):
    return _io('key', node=node)


def _private_dir(path):
    return Path(_io('private_directory', path=os.path.abspath(path)))


def _atomic_json(path, value):
    return _io('write_json', path=str(path), value=value)


def _read_json(path):
    return _io('read_json', path=str(path))


@dataclass
class Entry:
    inode: int
    parent: int
    node: Node
    local: bool = False


def _entry(value):
    return Entry(value['inode'], value['parent'], Node(**value['node']), value['local'])


class OnDemandFS:
    def __init__(self, backend, cache_dir, remote_root='/my-files', *, writable=False,
                 offline=False, metadata_ttl=15.0):
        self.backend = backend
        self.cache_dir = Path(os.path.abspath(cache_dir))
        self.objects, self.recovery, self.staging = (self.cache_dir / name for name in ('objects', 'recovery', 'staging'))
        self.remote_root = remote_root
        self.metadata_ttl = metadata_ttl
        self._mutex = threading.RLock()
        self._status = {}
        self._closed = False
        self._id = self._invoke({'action': 'create', 'config': {
            'cache': str(self.cache_dir), 'remote_root': remote_root,
            'writable': bool(writable), 'offline': bool(offline), 'metadata_ttl': float(metadata_ttl)}})

    def _invoke(self, operation):
        value = service_call(self.backend, 'ondemand_fs', operation)
        self._status.update(value.get('status', {}))
        return _unwrap(value)

    def try_recovery(self):
        if self._closed:
            return self.pending_recovery()
        value = service_call(self.backend, 'ondemand_fs', {'action': 'try_recovery', 'id': self._id})
        self._status.update(value.get('status', {}))
        return None if value.get('busy') else _unwrap(value)

    def _call(self, action, **arguments):
        with self._mutex:
            return self._invoke({'action': 'call', 'id': self._id,
                                 'operation': {'action': action, **arguments}})

    @property
    def writable(self): return self._status['writable']

    @writable.setter
    def writable(self, value): self._call('set_writable', writable=bool(value))

    @property
    def offline(self): return self._status['offline']

    @property
    def account_blocked(self): return self._status['account_blocked']

    @property
    def root_uid(self): return self._status['root_uid']

    @property
    def last_error(self): return self._status['last_error']

    @last_error.setter
    def last_error(self, value): self._call('set_error', message=value)

    @property
    def entries(self): return {entry['inode']: _entry(entry) for entry in self._call('snapshot')['tree']['entries']}

    @property
    def children(self): return {int(key): value for key, value in self._call('snapshot')['tree']['children'].items()}

    @property
    def handles(self): return dict.fromkeys(self._call('snapshot')['handles'])

    def block_account(self): return self._call('block_account')

    def invalidate(self, *_args, **_kwargs): return self._call('invalidate')

    def refresh(self, inode=1): return [_entry(e) for e in self._call('refresh', inode=inode)]

    def listdir(self, inode=1, *, refresh=True):
        return [_entry(e) for e in self._call('listdir', inode=inode, refresh=refresh)]

    def lookup(self, parent, name): return _entry(self._call('lookup', parent=parent, name=name))

    def metadata(self, inode): return self._call('metadata', inode=inode)

    def pin(self, inode, pinned=True): return self._call('pin', inode=inode, pinned=pinned)

    def evict(self, inode): return self._call('evict', inode=inode)

    def open(self, inode, flags=os.O_RDONLY): return self._call('open', inode=inode, flags=flags)

    def read(self, handle, offset, size):
        return bytes.fromhex(self._call('read', handle=handle, offset=offset, size=size))

    def write(self, handle, offset, content):
        return self._call('write', handle=handle, offset=offset, content=content[:8 * 1024 * 1024].hex())

    def truncate(self, inode, size, handle=None):
        return self._call('truncate', inode=inode, size=size, handle=handle)

    def create(self, parent, name, flags=os.O_RDWR):
        handle, entry = self._call('create', parent=parent, name=name, flags=flags)
        return handle, _entry(entry)

    def flush(self, number): return self._call('flush', handle=number)

    def release(self, number): return self._call('release', handle=number)

    def pending_recovery(self):
        with self._mutex:
            if self._closed:
                from .rust_core import backend_value
                return _unwrap(backend_value('ondemand_handles', operation={
                    'action': 'recovery', 'directory': str(self.recovery), 'active': []}))
            return self._call('recovery')

    def mkdir(self, parent, name): return _entry(self._call('mkdir', parent=parent, name=name))

    def rename(self, parent, name, new_parent, new_name, *, no_replace=False):
        return self._call('rename', parent=parent, name=name, new_parent=new_parent,
                          new_name=new_name, no_replace=no_replace)

    def unlink(self, parent, name, *, directory=False):
        return self._call('unlink', parent=parent, name=name, directory=directory)

    def close(self):
        with self._mutex:
            if not self._closed:
                self._invoke({'action': 'close', 'id': self._id})
                self._closed = True
