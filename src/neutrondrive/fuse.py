"""Optional pyfuse3 adapter and single-mount lifecycle manager.

Mounts are private to the calling Unix user. The kernel's writeback cache is
intentionally disabled: flush/fsync can report upload failures to applications.
No credentials, remote file names, or subprocess diagnostics are logged here.
"""
from __future__ import annotations

import errno
import functools
import hashlib
import importlib.util
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import threading

from .backend import BackendError
from .ondemand import OnDemandError, OnDemandFS

try:
    import pyfuse3
    import trio
except ImportError:
    pyfuse3 = None
    trio = None


_SESSION_LOCK = threading.Lock()


def availability():
    if os.environ.get('FLATPAK_ID'):
        return False, 'Flatpak requires the separately installed native NeutronDrive desktop service for FUSE mounts.'
    if os.environ.get('SNAP'):
        return False, 'On-demand FUSE mounts are unavailable inside strict Snap confinement. Install and run the native NeutronDrive DEB/RPM package for this feature.'
    if pyfuse3 is None or trio is None:
        return False, 'Install python3-pyfuse3, python3-trio and fuse3 for on-demand files.'
    if not Path('/dev/fuse').exists():
        return False, 'The FUSE device /dev/fuse is unavailable in this environment.'
    if not shutil.which('fusermount3'):
        return False, 'fusermount3 is missing. Install the fuse3 package.'
    return True, ''


def _operation(method):
    @functools.wraps(method)
    async def wrapper(self, *args, **kwargs):
        try:
            return await method(self, *args, **kwargs)
        except pyfuse3.FUSEError:
            raise
        except OSError as exc:
            raise pyfuse3.FUSEError(exc.errno or errno.EIO) from None
        except BackendError:
            raise pyfuse3.FUSEError(errno.EIO) from None
        except Exception:
            self.fs.last_error = 'A filesystem operation failed; no unconfirmed write is treated as successful.'
            raise pyfuse3.FUSEError(errno.EIO) from None
    return wrapper


class FuseOperations(pyfuse3.Operations if pyfuse3 else object):
    enable_writeback_cache = False

    def __init__(self, filesystem):
        if pyfuse3 is None:
            raise OnDemandError(errno.ENOSYS, 'pyfuse3 is not installed')
        super().__init__()
        self.fs = filesystem
        self.directories = {}
        self.next_directory = 1

    async def _call(self, name, *args, **kwargs):
        return await trio.to_thread.run_sync(functools.partial(getattr(self.fs, name), *args, **kwargs))

    def _attrs(self, values):
        entry = pyfuse3.EntryAttributes()
        mode = (0o700 if values['folder'] else 0o600) if values['writable'] else (0o500 if values['folder'] else 0o400)
        entry.st_mode = (stat.S_IFDIR if values['folder'] else stat.S_IFREG) | mode
        entry.st_ino = values['inode']
        entry.st_size = values['size']
        entry.st_uid, entry.st_gid = os.getuid(), os.getgid()
        entry.st_nlink = 2 if values['folder'] else 1
        entry.st_atime_ns = entry.st_mtime_ns = entry.st_ctime_ns = values['mtime_ns']
        entry.st_rdev = 0
        entry.st_blksize = 4096
        entry.st_blocks = (values['size'] + 511) // 512 if values['cached'] else 0
        entry.generation = 0
        entry.entry_timeout = 0
        entry.attr_timeout = 0
        return entry

    @_operation
    async def getattr(self, inode, ctx=None):
        return self._attrs(await self._call('metadata', inode))

    @_operation
    async def lookup(self, parent_inode, name, ctx=None):
        entry = await self._call('lookup', parent_inode, os.fsdecode(name))
        return await self.getattr(entry.inode)

    @_operation
    async def opendir(self, inode, ctx):
        entries = await self._call('listdir', inode)
        number = self.next_directory
        self.next_directory += 1
        # Stable per-open snapshot: concurrent changes cannot duplicate/skip an
        # entry while the kernel resumes readdir using an offset.
        self.directories[number] = entries
        return number

    @_operation
    async def readdir(self, fh, start_id, token):
        for offset, entry in enumerate(self.directories[fh][start_id:], start_id + 1):
            attributes = await self.getattr(entry.inode)
            if not pyfuse3.readdir_reply(token, os.fsencode(entry.node.name), attributes, offset):
                break

    async def releasedir(self, fh):
        self.directories.pop(fh, None)

    @_operation
    async def open(self, inode, flags, ctx):
        number = await self._call('open', inode, flags)
        info = pyfuse3.FileInfo(fh=number)
        info.direct_io = True
        info.keep_cache = False
        return info

    @_operation
    async def read(self, fh, off, size):
        return await self._call('read', fh, off, size)

    @_operation
    async def write(self, fh, off, buf):
        return await self._call('write', fh, off, bytes(buf))

    @_operation
    async def flush(self, fh):
        await self._call('flush', fh)

    @_operation
    async def fsync(self, fh, datasync):
        await self._call('flush', fh)

    async def release(self, fh):
        # Release has no useful error return to applications. The model leaves
        # dirty content in recovery; flush/fsync already returned the failure.
        try:
            await self._call('release', fh)
        except Exception:
            self.fs.last_error = 'A closed handle requires local recovery.'

    @_operation
    async def create(self, parent_inode, name, mode, flags, ctx):
        number, entry = await self._call('create', parent_inode, os.fsdecode(name), flags)
        info = pyfuse3.FileInfo(fh=number)
        info.direct_io = True
        info.keep_cache = False
        return info, await self.getattr(entry.inode)

    @_operation
    async def setattr(self, inode, attr, fields, fh, ctx):
        if fields.update_uid and attr.st_uid != os.getuid():
            raise pyfuse3.FUSEError(errno.EPERM)
        if fields.update_gid and attr.st_gid != os.getgid():
            raise pyfuse3.FUSEError(errno.EPERM)
        if fields.update_mode:
            current = await self.getattr(inode)
            if stat.S_IMODE(attr.st_mode) != stat.S_IMODE(current.st_mode):
                raise pyfuse3.FUSEError(errno.ENOTSUP)
        if fields.update_size:
            await self._call('truncate', inode, attr.st_size, fh)
        # Proton CLI cannot set arbitrary timestamps. Return the actual cloud
        # times rather than claiming an unsupported metadata mutation succeeded.
        if fields.update_atime or fields.update_mtime:
            raise pyfuse3.FUSEError(errno.ENOTSUP)
        return await self.getattr(inode)

    @_operation
    async def mkdir(self, parent_inode, name, mode, ctx):
        entry = await self._call('mkdir', parent_inode, os.fsdecode(name))
        return await self.getattr(entry.inode)

    @_operation
    async def unlink(self, parent_inode, name, ctx):
        await self._call('unlink', parent_inode, os.fsdecode(name))

    @_operation
    async def rmdir(self, parent_inode, name, ctx):
        await self._call('unlink', parent_inode, os.fsdecode(name), directory=True)

    @_operation
    async def rename(self, parent_inode_old, name_old, parent_inode_new, name_new, flags, ctx):
        if flags & ~getattr(pyfuse3, 'RENAME_NOREPLACE', 1):
            raise pyfuse3.FUSEError(errno.ENOTSUP)
        await self._call('rename', parent_inode_old, os.fsdecode(name_old), parent_inode_new, os.fsdecode(name_new), no_replace=bool(flags & 1))

    @_operation
    async def statfs(self, ctx):
        usage = os.statvfs(self.fs.cache_dir)
        result = pyfuse3.StatvfsData()
        result.f_bsize, result.f_frsize = usage.f_bsize, usage.f_frsize
        result.f_blocks, result.f_bfree, result.f_bavail = usage.f_blocks, usage.f_bfree, usage.f_bavail
        result.f_files, result.f_ffree, result.f_favail = usage.f_files, usage.f_ffree, usage.f_favail
        result.f_namemax = 255
        return result

    @_operation
    async def listxattr(self, inode, ctx):
        return [b'user.neutrondrive.state']

    @_operation
    async def getxattr(self, inode, name, ctx):
        if name != b'user.neutrondrive.state':
            raise pyfuse3.FUSEError(getattr(errno, 'ENODATA', errno.ENOENT))
        value = await self._call('metadata', inode)
        return b'pinned' if value['pinned'] else b'cached' if value['cached'] else b'online-only'


class MountManager:
    """Own one kernel mount and its thread; one active instance per process."""
    def __init__(self, backend, state_dir):
        self.backend = backend
        self.state_dir = Path(state_dir)
        self.fs = None
        self.mountpoint = None
        self._thread = None
        self._ready = threading.Event()
        self._error = ''
        self._mounted = False
        self._session_owned = False
        self._last_recovery = []
        self._lifecycle = threading.RLock()

    def mount(self, mountpoint, remote_root='/my-files', writable=False, offline=False):
        with self._lifecycle:
            available, reason = availability()
            if not available:
                raise OnDemandError(errno.ENOSYS, reason)
            if self._thread and self._thread.is_alive():
                raise OnDemandError(errno.EBUSY, 'This manager already has a mount')
            point = Path(os.path.abspath(Path(mountpoint).expanduser()))
            for part in (point, *point.parents):
                if part.is_symlink():
                    raise OnDemandError(errno.ELOOP, 'Mountpoint must not contain symbolic links')
            if os.environ.get('SNAP'):
                roots = [Path(p) for p in (os.environ.get('SNAP_USER_COMMON'), os.environ.get('SNAP_USER_DATA')) if p]
                if not any(point.is_relative_to(root) and point != root for root in roots):
                    raise OnDemandError(errno.EACCES, 'Snap mounts must be below SNAP_USER_COMMON or SNAP_USER_DATA; connect fuse-support first.')
            point.mkdir(parents=True, exist_ok=True, mode=0o700)
            if point.stat().st_uid != os.getuid() or not point.is_dir() or os.path.ismount(point) or any(point.iterdir()):
                raise OnDemandError(errno.EBUSY, 'Mountpoint must be an empty, unmounted directory owned by you')
            if self.state_dir.absolute().is_relative_to(point) or point.is_relative_to(self.state_dir.absolute()):
                raise OnDemandError(errno.EINVAL, 'Mountpoint and cache/state directories must not overlap')
            if not _SESSION_LOCK.acquire(blocking=False):
                raise OnDemandError(errno.EBUSY, 'Only one native FUSE mount per desktop-service process is supported')
            self._session_owned = True
            try:
                key = hashlib.sha256((str(point) + '\0' + remote_root).encode()).hexdigest()
                self.fs = OnDemandFS(self.backend, self.state_dir / 'ondemand' / key,
                                     remote_root, writable=writable, offline=offline)
                self.mountpoint = point
                self._error = ''
                self._ready.clear()
                self._thread = threading.Thread(target=self._run, name='neutrondrive-fuse', daemon=True)
                self._thread.start()
            except BaseException:
                self._session_owned = False
                _SESSION_LOCK.release()
                raise
        if not self._ready.wait(15):
            raise OnDemandError(errno.ETIMEDOUT, 'FUSE mount did not become ready')
        if not self._mounted:
            raise OnDemandError(errno.EIO, self._error or 'FUSE mount failed')
        return self.status()

    start = mount

    def _run(self):
        initialized = False
        try:
            options = set(pyfuse3.default_options) | {'fsname=NeutronDrive', 'subtype=neutrondrive', 'default_permissions', 'nodev', 'nosuid'}
            if not self.fs.writable:
                options.add('ro')
            pyfuse3.init(FuseOperations(self.fs), str(self.mountpoint), options)
            initialized = True
            self._mounted = True
            self._ready.set()
            trio.run(pyfuse3.main)
        except BaseException:
            self._error = 'FUSE mount failed. Check /dev/fuse permissions, fuse3 installation and sandbox fuse-support.'
        finally:
            self._mounted = False
            try:
                if initialized:
                    pyfuse3.close(unmount=True)
            finally:
                try:
                    if self.fs:
                        self.fs.close()
                finally:
                    if self._session_owned:
                        self._session_owned = False
                        _SESSION_LOCK.release()
                    self._ready.set()

    def unmount(self):
        with self._lifecycle:
            if not self._thread or not self._thread.is_alive():
                return self.status()
            result = subprocess.run(['fusermount3', '-u', str(self.mountpoint)], stdin=subprocess.DEVNULL,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
            if result.returncode:
                raise OnDemandError(errno.EBUSY, 'Mount is busy; close files and terminals using it before unmounting')
            self._thread.join(15)
            if self._thread.is_alive():
                raise OnDemandError(errno.EBUSY, 'FUSE is still finishing active work')
            return self.status()

    stop = unmount

    def status(self):
        value = {'status': 'mounted' if self._mounted else 'stopped', 'mounted': self._mounted,
                 'mountpoint': str(self.mountpoint) if self.mountpoint else '',
                 'error': self._error, 'read_only': (not self.fs.writable or self.fs.account_blocked) if self.fs else True,
                 'cache_dir': str(self.fs.cache_dir) if self.fs else '',
                 'remote_root': self.fs.remote_root if self.fs else '',
                 'offline': self.fs.offline if self.fs else False}
        if self.fs:
            value['error'] = value['error'] or self.fs.last_error
            # Status must remain responsive while a worker hydrates/uploads.
            if self.fs._mutex.acquire(blocking=False):
                try:
                    self._last_recovery = self.fs.pending_recovery()
                finally:
                    self.fs._mutex.release()
            value['recovery'] = list(self._last_recovery)
        return value

    def _resolve(self, relative_path):
        if not self._mounted or not self.fs:
            raise OnDemandError(errno.ENOTCONN, 'No active on-demand mount')
        path = PurePosixPath(relative_path)
        if path.is_absolute() or '..' in path.parts:
            raise OnDemandError(errno.EINVAL, 'Use a path relative to the mount')
        inode = 1
        for name in path.parts:
            inode = self.fs.lookup(inode, name).inode
        return inode

    def pin(self, relative_path, pinned=True):
        self.fs.pin(self._resolve(relative_path), pinned)
        return self.status()

    def evict(self, relative_path):
        inode = self._resolve(relative_path)
        self.fs.pin(inode, False)
        self.fs.evict(inode)
        return self.status()

    def refresh(self):
        if not self.fs or not self._mounted:
            raise OnDemandError(errno.ENOTCONN, 'No active on-demand mount')
        self.fs.invalidate()
        self.fs.refresh()
        return self.status()

    def invalidate(self, event=None):
        if self.fs:
            root_uid = getattr(event, 'root_uid', None)
            if (root_uid and root_uid != self.fs.entries[1].node.uid) or getattr(event, 'kind', '') == 'tree_remove':
                self.fs.block_account()
            else:
                self.fs.invalidate()


def run_mount(backend, mountpoint, state_dir, remote_root='/my-files', writable=False, offline=False):
    manager = MountManager(backend, state_dir)
    manager.mount(mountpoint, remote_root, writable, offline)
    try:
        while manager._thread.is_alive():
            manager._thread.join(0.5)
    except KeyboardInterrupt:
        manager.unmount()
    return manager.status()
