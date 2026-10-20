"""Verified local cache and filesystem model for the optional FUSE mount.

Metadata never creates content placeholders. Only open() downloads content. The
cache contains plaintext and is private to the current Unix user, like an opened
file in any file manager. Pinning retains verified content for explicit offline
mounts. Cloud writes are opt-in: the CLI has no compare-and-swap, so pre/post
revision checks cannot eliminate a concurrent remote writer's race. Failed or
uncertain writes retain fsync'ed local data and a recovery journal, never replayed
automatically after a crash.
"""
from __future__ import annotations

from collections import Counter
from contextlib import contextmanager
from dataclasses import asdict, dataclass, replace
from datetime import datetime
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
import threading
import time
import uuid

from .backend import BackendError, Node, join_remote, validate_name, validate_remote


class OnDemandError(OSError):
    """An errno suitable for returning to a filesystem caller."""


def _fail(code, message):
    raise OnDemandError(code, message)


def _safe_name(name):
    validate_name(name)
    if any(c in name for c in '*?[{') or any(ord(c) == 127 for c in name):
        _fail(errno.EINVAL, 'Name contains unsupported CLI glob syntax or control characters')
    try:
        encoded = os.fsencode(name)
    except UnicodeError:
        _fail(errno.EINVAL, 'File name cannot be represented on Linux')
    if len(encoded) > 255:
        _fail(errno.ENAMETOOLONG, 'Name exceeds the Linux component limit')
    return name


def _safe_node(node):
    if not node.safe or node.type not in {'file', 'folder'} or not node.uid:
        _fail(errno.EIO, 'Unsafe or unsupported remote node')
    _safe_name(node.name)
    validate_remote(node.path)
    if node.type == 'file' and (not isinstance(node.size, int) or node.size < 0 or
                              not re.fullmatch('[a-fA-F0-9]{40}', node.sha1) or not node.revision):
        _fail(errno.ENOTSUP, 'File has no verifiable size, SHA1 or revision (native documents are unsupported)')
    return node


def _revision(node):
    return node.uid, node.revision, node.sha1.lower(), node.size


def _key(node):
    prefix = hashlib.sha256(node.uid.encode()).hexdigest()
    return prefix + '-' + hashlib.sha256(json.dumps(_revision(node)).encode()).hexdigest()


def _node_dict(node):
    return {k: v for k, v in asdict(node).items() if k != 'raw'}


def _private_dir(path):
    """Reject links in every ancestor, then require user ownership at the leaf."""
    path = Path(os.path.abspath(path))
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in path.parts[1:]:
            try:
                os.mkdir(component, 0o700, dir_fd=fd)
            except FileExistsError:
                pass
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        if os.fstat(fd).st_uid != os.getuid():
            _fail(errno.EACCES, 'Cache directory belongs to another user')
        os.fchmod(fd, 0o700)
    finally:
        os.close(fd)
    return path


def _regular_fd(path, flags=os.O_RDONLY):
    fd = os.open(path, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
        os.close(fd)
        _fail(errno.EACCES, 'Cache object is not a private regular file')
    if info.st_mode & 0o077:
        os.close(fd)
        _fail(errno.EACCES, 'Cache object permissions are not private')
    return fd


def _digest_fd(fd):
    digest = hashlib.sha1()
    offset = 0
    before = os.fstat(fd)
    while chunk := os.pread(fd, 1024 * 1024, offset):
        digest.update(chunk)
        offset += len(chunk)
    after = os.fstat(fd)
    if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
        _fail(errno.EBUSY, 'Cache object changed during verification')
    return digest.hexdigest(), offset


def _atomic_json(path, value):
    payload = json.dumps(value, ensure_ascii=True, sort_keys=True).encode()
    fd, temp = tempfile.mkstemp(prefix='.metadata-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def _read_json(path):
    fd = _regular_fd(path)
    with os.fdopen(fd, 'r') as stream:
        if os.fstat(stream.fileno()).st_size > 32 * 1024 * 1024:
            _fail(errno.EFBIG, 'Metadata file is too large')
        return json.load(stream)


@dataclass
class Entry:
    inode: int
    parent: int
    node: Node
    local: bool = False


@dataclass
class Handle:
    number: int
    inode: int
    fd: int
    writable: bool
    baseline: Node
    work: Path | None = None
    journal: Path | None = None
    dirty: bool = False
    failed: str = ''
    append: bool = False
    intent: dict | None = None


class OnDemandFS:
    """Thread-safe metadata/cache API; adapter calls it from worker threads.

    Only /my-files is mountable. Cache metadata is bound to its root UID to
    prevent cross-account reuse. offline=True explicitly opens the saved tree;
    absent content returns ENETDOWN instead of fabricated zero bytes.
    """
    def __init__(self, backend, cache_dir, remote_root='/my-files', *, writable=False,
                 offline=False, metadata_ttl=15.0):
        validate_remote(remote_root)
        if remote_root.split('/', 2)[1] != 'my-files':
            _fail(errno.ENOTSUP, 'Only My files supports an on-demand mount')
        self.backend, self.remote_root = backend, remote_root
        self.writable, self.offline = bool(writable), bool(offline)
        if self.offline and self.writable:
            _fail(errno.EROFS, 'Offline mounts are read-only')
        self.cache_dir = _private_dir(cache_dir)
        self.objects = _private_dir(self.cache_dir / 'objects')
        self.recovery = _private_dir(self.cache_dir / 'recovery')
        self.staging = _private_dir(self.cache_dir / 'staging')
        self._lock_fd = os.open(self.cache_dir / '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            info = os.fstat(self._lock_fd)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1 or info.st_mode & 0o077:
                _fail(errno.EACCES, 'Unsafe cache lock file')
            fcntl.flock(self._lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(self._lock_fd)
            _fail(errno.EBUSY, 'This cache is already used by another mount')
        self._mutex = threading.RLock()
        self._closed = False
        self.entries = {}
        self.children = {}
        self.loaded = {}
        self.pinned = set()
        self.handles = {}
        self._next_inode, self._next_handle = 2, 1
        self.metadata_ttl = metadata_ttl
        self.last_error = ''
        self._observed = {}
        self.account_blocked = False
        try:
            self._load()
            if offline:
                if 1 not in self.entries:
                    _fail(errno.ENETDOWN, 'No saved directory metadata for an offline mount')
            else:
                root = backend.info(remote_root)
                if root.type != 'folder' or not root.safe or not root.uid:
                    _fail(errno.EIO, 'Cannot verify the remote mount root')
                if self.entries and self.entries[1].node.uid != root.uid:
                    _fail(errno.ESTALE, 'Cache belongs to another account or cloud root; choose a different cache')
                # All transfers address the immutable root UID. A credential
                # switch between root check and upload must not retarget the
                # dynamic /my-files alias to another account.
                root_path = join_remote('/my-files', root.uid)
                self.entries[1] = Entry(1, 1, replace(root, path=root_path))
                self.children.setdefault(1, {})
                self._save()
        except BaseException:
            os.close(self._lock_fd)
            self._closed = True
            raise

    def _load(self):
        path = self.cache_dir / 'metadata.json'
        if not path.exists():
            return
        try:
            value = _read_json(path)
            if value['version'] != 1 or value['remote_root'] != self.remote_root:
                raise ValueError('wrong cache')
            for item in value['entries']:
                node = Node(**item['node'])
                ino, parent = item['inode'], item['parent']
                if not isinstance(ino, int) or ino < 1 or not isinstance(parent, int):
                    raise ValueError('invalid inode')
                if ino != 1:
                    _safe_node(node)
                self.entries[ino] = Entry(ino, parent, node)
            if self.entries[1].node.type != 'folder':
                raise ValueError('invalid root')
            for ino, entry in self.entries.items():
                if entry.node.type == 'folder':
                    self.children.setdefault(ino, {})
                if ino != 1:
                    self.children.setdefault(entry.parent, {})[entry.node.name] = ino
            self.pinned = {int(i) for i in value.get('pinned', []) if int(i) in self.entries}
            # Persist exactly which directories were fully enumerated, including empty ones.
            self.loaded = {int(i): 0 for i in value.get('listed', []) if int(i) in self.entries}
            self._next_inode = max(self.entries) + 1
        except (ValueError, TypeError, KeyError, OSError) as exc:
            _fail(errno.EIO, 'Saved mount metadata is invalid; preserve it and choose a fresh cache')

    def _save(self):
        reachable, pending = set(), [1]
        while pending:
            inode = pending.pop()
            if inode in reachable:
                continue
            reachable.add(inode)
            pending.extend(self.children.get(inode, {}).values())
        _atomic_json(self.cache_dir / 'metadata.json', {
            'version': 1, 'remote_root': self.remote_root,
            'entries': [{'inode': e.inode, 'parent': e.parent, 'node': _node_dict(e.node)}
                        for e in self.entries.values() if not e.local and e.inode in reachable],
            'listed': list(self.loaded), 'pinned': sorted(self.pinned)})

    def _entry(self, inode):
        if inode not in self.entries:
            _fail(errno.ENOENT, 'Entry no longer exists')
        return self.entries[inode]

    def block_account(self):
        with self._mutex:
            self.account_blocked = True
            self.last_error = 'Cloud account or mount root changed. Unmount before reconnecting; unsaved content remains in recovery.'

    def _check_root(self):
        if self.account_blocked:
            _fail(errno.EACCES, self.last_error)
        if not self.offline:
            current = self.backend.info(self.remote_root)
            if current.uid != self.entries[1].node.uid or current.type != 'folder':
                self.block_account()
                _fail(errno.EACCES, self.last_error)

    def invalidate(self, *_args, **_kwargs):
        """Event-source hook; next directory lookup refreshes remote metadata."""
        with self._mutex:
            self.loaded = {key: 0 for key in self.loaded}

    def refresh(self, inode=1):
        with self._mutex:
            entry = self._entry(inode)
            if entry.node.type != 'folder':
                _fail(errno.ENOTDIR, 'Not a directory')
            if self.offline:
                if inode not in self.loaded:
                    _fail(errno.ENETDOWN, 'This folder was not listed before going offline')
                return self.listdir(inode, refresh=False)
            self._check_root()
            nodes = self.backend.list_nodes(entry.node.path)
            counts = Counter(n.name for n in nodes)
            known = {e.node.uid: e for e in self.entries.values()}
            updated = {}
            for node in nodes:
                try:
                    _safe_node(node)
                except (OSError, BackendError):
                    continue  # Unsupported documents/ambiguous names are never fake files.
                if counts[node.name] != 1:
                    continue
                existing = known.get(node.uid)
                if existing:
                    child = existing.inode
                    # Open file descriptors keep their snapshot until release.
                    if not any(h.inode == child for h in self.handles.values()):
                        if child in self.pinned and _revision(existing.node) != _revision(node):
                            node = replace(existing.node, name=node.name, path=node.path)
                        self.entries[child] = Entry(child, inode, node)
                else:
                    child = self._next_inode
                    self._next_inode += 1
                    self.entries[child] = Entry(child, inode, node)
                updated[node.name] = child
                if node.type == 'folder':
                    self.children.setdefault(child, {})
            # Local creates remain visible until their first successful upload.
            for name, child in self.children.get(inode, {}).items():
                if self.entries[child].local:
                    if name in updated:
                        self.last_error = 'Remote name conflicts with a pending local create; local data is retained'
                    updated[name] = child
            self.children[inode] = updated
            self.loaded[inode] = time.monotonic()
            self._save()
            return self.listdir(inode, refresh=False)

    def listdir(self, inode=1, *, refresh=True):
        with self._mutex:
            if self._entry(inode).node.type != 'folder':
                _fail(errno.ENOTDIR, 'Not a directory')
            if refresh and (inode not in self.loaded or
                            (not self.offline and time.monotonic() - self.loaded[inode] >= self.metadata_ttl)):
                try:
                    return self.refresh(inode)
                except BackendError:
                    if inode not in self.loaded:
                        raise
                    self.last_error = 'Cloud unavailable; showing saved metadata. Pinned files remain readable.'
            if self.offline and inode not in self.loaded:
                _fail(errno.ENETDOWN, 'Folder metadata is unavailable offline')
            return [self.entries[i] for _, i in sorted(self.children.get(inode, {}).items())]

    def lookup(self, parent, name):
        with self._mutex:
            if name == '.':
                return self._entry(parent)
            if name == '..':
                return self._entry(self._entry(parent).parent)
            self.listdir(parent)
            try:
                return self._entry(self.children[parent][name])
            except KeyError:
                _fail(errno.ENOENT, 'No such file')

    def metadata(self, inode):
        with self._mutex:
            entry = self._entry(inode)
            size = entry.node.size or 0
            for handle in self.handles.values():
                if handle.inode == inode and handle.writable:
                    size = os.fstat(handle.fd).st_size
                    break
            try:
                stamp = int(datetime.fromisoformat(entry.node.modified.replace('Z', '+00:00')).timestamp() * 1e9)
            except (ValueError, TypeError, OverflowError):
                stamp = 0
            return {'inode': inode, 'folder': entry.node.type == 'folder', 'size': size,
                    'mtime_ns': stamp, 'cached': (self.objects / _key(entry.node)).is_file(),
                    'pinned': inode in self.pinned, 'writable': self.writable}

    def _verified(self, path, node):
        fd = _regular_fd(path)
        try:
            digest, size = _digest_fd(fd)
            if digest != node.sha1.lower() or size != node.size:
                _fail(errno.EIO, 'Cached content does not match the remote SHA1 and size')
            return fd
        except BaseException:
            os.close(fd)
            raise

    def _hydrate(self, entry):
        if self.account_blocked:
            _fail(errno.EACCES, self.last_error)
        node = entry.node
        target = self.objects / _key(node)
        if target.exists() or target.is_symlink():
            return self._verified(target, node)
        if self.offline:
            _fail(errno.ENETDOWN, 'File content is not available offline; pin it while online')
        self._check_root()
        before = self.backend.info(node.path)
        if _revision(before) != _revision(node):
            _fail(errno.ESTALE, 'Remote file changed; refresh the folder before opening it')
        with tempfile.TemporaryDirectory(prefix='download-', dir=self.staging) as directory:
            self.backend.download([node.path], Path(directory))
            paths = list(Path(directory).iterdir())
            if len(paths) != 1 or paths[0].name != node.name:
                _fail(errno.EIO, 'Download did not produce exactly the expected file')
            # SDK download mode is normally inherited from umask; make private
            # before verification. chmod must never follow a download symlink.
            fd = os.open(paths[0], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                if not stat.S_ISREG(os.fstat(fd).st_mode) or os.fstat(fd).st_nlink != 1:
                    _fail(errno.EIO, 'Download produced a non-regular file')
                os.fchmod(fd, 0o600)
                digest, size = _digest_fd(fd)
                if digest != node.sha1.lower() or size != node.size:
                    _fail(errno.EIO, 'Downloaded content failed size/SHA1 verification')
                after = self.backend.info(node.path)
                if _revision(after) != _revision(node):
                    _fail(errno.ESTALE, 'Remote revision changed during download')
                os.fsync(fd)
                os.replace(paths[0], target)
            finally:
                os.close(fd)
        return self._verified(target, node)

    def pin(self, inode, pinned=True):
        with self._mutex:
            entry = self._entry(inode)
            if entry.node.type == 'folder':
                _fail(errno.EISDIR, 'Pin files individually; recursive folder pinning is not supported')
            if pinned:
                fd = self._hydrate(entry)
                os.close(fd)
                self.pinned.add(inode)
            else:
                self.pinned.discard(inode)
            self._save()

    def evict(self, inode):
        with self._mutex:
            if inode in self.pinned or any(h.inode == inode for h in self.handles.values()):
                _fail(errno.EBUSY, 'Unpin and close the file before freeing its cache')
            entry = self._entry(inode)
            prefix = hashlib.sha256(entry.node.uid.encode()).hexdigest() + '-'
            for path in self.objects.glob(prefix + '*'):
                if not re.fullmatch('[a-f0-9]{64}-[a-f0-9]{64}', path.name) or path.is_symlink():
                    _fail(errno.EACCES, 'Unsafe cache entry')
                fd = _regular_fd(path)
                os.close(fd)
                path.unlink()

    def _writable(self):
        if self.account_blocked:
            _fail(errno.EACCES, self.last_error)
        if not self.writable:
            _fail(errno.EROFS, 'This mount is read-only')

    def _journal(self, handle, state='dirty'):
        entry = self._entry(handle.inode)
        _atomic_json(handle.journal, {'version': 1, 'state': state,
            'data': handle.work.name, 'name': entry.node.name,
            'parent': self._entry(entry.parent).node.path,
            'baseline': _node_dict(handle.baseline), 'new_file': entry.local,
            'error': handle.failed, 'intent': handle.intent})

    def open(self, inode, flags=os.O_RDONLY):
        with self._mutex:
            if self.account_blocked:
                _fail(errno.EACCES, self.last_error)
            entry = self._entry(inode)
            if entry.node.type != 'file':
                _fail(errno.EISDIR, 'Cannot open a directory as a file')
            writable = flags & os.O_ACCMODE != os.O_RDONLY
            if not writable and flags & os.O_TRUNC:
                _fail(errno.EINVAL, 'Truncation requires a writable handle')
            if writable:
                self._writable()
                if any(h.inode == inode for h in self.handles.values()):
                    _fail(errno.EBUSY, 'Close other handles before editing this file')
                if any(r.get('baseline', {}).get('uid') == entry.node.uid for r in self.pending_recovery()):
                    _fail(errno.EBUSY, 'Recover the previous unsaved edit before editing again')
            elif any(h.inode == inode and h.writable for h in self.handles.values()):
                _fail(errno.EBUSY, 'A writer already has this file open')
            if not self.offline and not entry.local:
                try:
                    self._check_root()
                    current = _safe_node(self.backend.info(entry.node.path))
                except BackendError:
                    if writable or inode not in self.pinned:
                        raise
                    self.last_error = 'Cloud unavailable; opening the verified pinned snapshot.'
                else:
                    if _revision(current) != _revision(entry.node):
                        if inode in self.pinned:
                            try:
                                refreshed = self._hydrate(Entry(inode, entry.parent, current))
                                os.close(refreshed)
                            except BackendError:
                                if writable:
                                    raise
                                self.last_error = 'New cloud revision unavailable; using the verified pinned snapshot.'
                            else:
                                entry.node = current
                        else:
                            entry.node = current
                        self._save()
            if entry.local:
                _fail(errno.EBUSY, 'The new file is already being created')
            fd = self._hydrate(entry)
            self._observed[entry.node.uid] = entry.node
            number = self._next_handle
            self._next_handle += 1
            handle = Handle(number, inode, fd, writable, entry.node, append=bool(flags & os.O_APPEND))
            if writable:
                os.close(fd)
                token = uuid.uuid4().hex
                work = self.recovery / (token + '.data')
                journal = self.recovery / (token + '.json')
                fd = os.open(work, os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_NOFOLLOW, 0o600)
                source = self._verified(self.objects / _key(entry.node), entry.node)
                try:
                    with os.fdopen(source, 'rb') as stream, os.fdopen(os.dup(fd), 'wb') as output:
                        shutil.copyfileobj(stream, output)
                        output.flush()
                    os.fsync(fd)
                except BaseException:
                    os.close(fd)
                    work.unlink()
                    raise
                handle.fd, handle.work, handle.journal = fd, work, journal
                try:
                    self._journal(handle, 'clean')
                except BaseException:
                    os.close(fd)
                    work.unlink(missing_ok=True)
                    raise
            self.handles[number] = handle
            if writable and flags & os.O_TRUNC:
                self.truncate(inode, 0, number)
            return number

    def read(self, handle, offset, size):
        with self._mutex:
            if offset < 0 or size < 0:
                _fail(errno.EINVAL, 'Invalid read range')
            return os.pread(self.handles[handle].fd, size, offset)

    def write(self, handle, offset, content):
        with self._mutex:
            item = self.handles[handle]
            if not item.writable:
                _fail(errno.EBADF, 'Read-only handle')
            if offset < 0:
                _fail(errno.EINVAL, 'Invalid write offset')
            item.dirty = True
            item.failed = ''
            self._journal(item)
            if item.append:
                offset = os.fstat(item.fd).st_size
            written = os.pwrite(item.fd, content, offset)
            os.fsync(item.fd)
            return written

    def truncate(self, inode, size, handle=None):
        with self._mutex:
            if size < 0:
                _fail(errno.EINVAL, 'Negative size')
            own_handle = handle is None
            number = self.open(inode, os.O_RDWR) if own_handle else handle
            item = self.handles[number]
            try:
                if not item.writable:
                    _fail(errno.EBADF, 'Read-only handle')
                item.dirty = True
                self._journal(item)
                os.ftruncate(item.fd, size)
                os.fsync(item.fd)
                if own_handle:
                    self.flush(number)
            finally:
                if own_handle:
                    self.release(number)

    def create(self, parent, name, flags=os.O_RDWR):
        with self._mutex:
            self._writable()
            _safe_name(name)
            self.refresh(parent)
            if name in self.children[parent]:
                _fail(errno.EEXIST, 'Destination already exists')
            inode, number = self._next_inode, self._next_handle
            self._next_inode += 1
            self._next_handle += 1
            node = Node('local-' + uuid.uuid4().hex, name, 'file', 0, revision='local',
                        sha1=hashlib.sha1(b'').hexdigest(), path=join_remote(self._entry(parent).node.path, name))
            self.entries[inode] = Entry(inode, parent, node, local=True)
            self.children[parent][name] = inode
            token = uuid.uuid4().hex
            work = self.recovery / (token + '.data')
            fd = None
            try:
                fd = os.open(work, os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_NOFOLLOW, 0o600)
                os.fsync(fd)
                handle = Handle(number, inode, fd, True, node, work, self.recovery / (token + '.json'), True,
                                append=bool(flags & os.O_APPEND))
                self._journal(handle)
            except BaseException:
                if fd is not None:
                    os.close(fd)
                work.unlink(missing_ok=True)
                self.children[parent].pop(name, None)
                self.entries.pop(inode, None)
                raise
            self.handles[number] = handle
            return number, self.entries[inode]

    def flush(self, number):
        with self._mutex:
            handle = self.handles[number]
            os.fsync(handle.fd)
            if not handle.dirty:
                return
            entry = self._entry(handle.inode)
            parent = self._entry(entry.parent)
            try:
                self._writable()
                self._check_root()
                digest, size = _digest_fd(handle.fd)
                remote = self.backend.list_nodes(parent.node.path)
                matches = [n for n in remote if n.name == entry.node.name]
                if entry.local:
                    if matches:
                        _fail(errno.EEXIST, 'A remote file now uses this name; local data is in recovery')
                    strategy = 'rename'
                else:
                    if len(matches) != 1 or _revision(matches[0]) != _revision(handle.baseline):
                        _fail(errno.ESTALE, 'Remote revision changed; local edit retained in recovery')
                    current = self.backend.info(handle.baseline.path)
                    if _revision(current) != _revision(handle.baseline):
                        _fail(errno.ESTALE, 'Remote revision changed before upload; local edit retained')
                    strategy = 'create-new-revision'
                self._journal(handle, 'uploading')
                # Upload by exact logical basename from a private staging directory.
                with tempfile.TemporaryDirectory(prefix='upload-', dir=self.staging) as directory:
                    upload = Path(directory) / entry.node.name
                    out = os.open(upload, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
                    try:
                        offset = 0
                        while chunk := os.pread(handle.fd, 1024 * 1024, offset):
                            view = memoryview(chunk)
                            while view:
                                view = view[os.write(out, view):]
                            offset += len(chunk)
                        os.fsync(out)
                    finally:
                        os.close(out)
                    self.backend.upload([upload], parent.node.path, file_strategy=strategy)
                matches = [n for n in self.backend.list_nodes(parent.node.path) if n.name == entry.node.name]
                if len(matches) != 1:
                    _fail(errno.EIO, 'Cannot identify the uploaded revision; local recovery retained')
                after = _safe_node(self.backend.info(matches[0].path))
                if after.sha1.lower() != digest or after.size != size or (not entry.local and after.uid != handle.baseline.uid):
                    _fail(errno.ESTALE, 'Remote content changed after upload; local recovery retained')
                entry.node, entry.local = after, False
                handle.baseline = after
                target = self.objects / _key(after)
                # A distinct inode is required: modifying the work file later must
                # never mutate previously verified cache content.
                fd, temp = tempfile.mkstemp(prefix='verified-', dir=self.objects)
                try:
                    with os.fdopen(fd, 'wb') as output:
                        offset = 0
                        while chunk := os.pread(handle.fd, 1024 * 1024, offset):
                            output.write(chunk)
                            offset += len(chunk)
                        output.flush()
                        os.fsync(output.fileno())
                    os.replace(temp, target)
                finally:
                    if os.path.exists(temp):
                        os.unlink(temp)
                self._journal(handle, 'clean')
                self._save()
                handle.dirty, handle.failed = False, ''
                self._observed[after.uid] = after
            except Exception:
                handle.dirty = True
                handle.failed = 'Writeback failed or conflicted; local content retained. Inspect recovery before retrying.'
                self.last_error = handle.failed
                self._journal(handle, 'recovery')
                raise

    def release(self, number):
        with self._mutex:
            handle = self.handles.pop(number)
            # FUSE release errors are not reported to applications. Never attempt
            # the first upload here: flush/fsync owns reporting and writeback.
            try:
                if handle.dirty:
                    handle.failed = handle.failed or 'Handle closed without confirmed writeback'
                    self._journal(handle, 'recovery')
                    self.last_error = handle.failed
            finally:
                os.close(handle.fd)
            if handle.work and not handle.dirty:
                handle.work.unlink(missing_ok=True)
                handle.journal.unlink(missing_ok=True)

    def pending_recovery(self):
        with self._mutex:
            result = []
            active = {h.journal for h in self.handles.values()}
            for path in sorted(self.recovery.glob('*.json')):
                if path in active:
                    continue
                value = _read_json(path)
                if value.get('state') == 'clean':
                    continue
                filename = value.get('data', '')
                if not re.fullmatch('[0-9a-f]{32}\\.data', filename):
                    _fail(errno.EIO, 'Invalid recovery metadata')
                fd = _regular_fd(self.recovery / filename)
                os.close(fd)
                result.append({**value, 'data_path': str(self.recovery / filename), 'journal_path': str(path)})
            return result

    def _not_open(self, inode):
        if any(h.inode == inode for h in self.handles.values()):
            _fail(errno.EBUSY, 'Close this file before changing its cloud name or deleting it')

    def mkdir(self, parent, name):
        with self._mutex:
            self._writable()
            _safe_name(name)
            self.refresh(parent)
            if name in self.children[parent]:
                _fail(errno.EEXIST, 'Destination already exists')
            _safe_node(self.backend.mkdir(self._entry(parent).node.path, name))
            self.refresh(parent)
            return self.lookup(parent, name)

    def _replace_file(self, source, target):
        """Editor temp-save: verified target revision first, source trash last.

        Cloud APIs cannot make this multi-step operation atomic. A crash or
        error after target upload can leave both files; recovery describes that
        state and always retains the replacement bytes.
        """
        if source.node.type != 'file' or target.node.type != 'file':
            _fail(errno.ENOTSUP, 'Only regular files support replacement')
        self._not_open(target.inode)
        expected = self._observed.get(target.node.uid, target.node)
        self._check_root()
        current = self.backend.info(target.node.path)
        if _revision(expected) != _revision(current):
            _fail(errno.ESTALE, 'Destination changed since it was read; source file was retained')
        source_node = source.node
        source_fd = self._hydrate(source)
        number = None
        try:
            number = self.open(target.inode, os.O_RDWR)
            handle = self.handles[number]
            if _revision(handle.baseline) != _revision(expected):
                _fail(errno.ESTALE, 'Destination changed before replacement; source retained')
            handle.intent = {'operation': 'replace', 'source': _node_dict(source_node),
                             'destination': _node_dict(expected)}
            self.truncate(target.inode, 0, number)
            offset = 0
            while chunk := os.pread(source_fd, 1024 * 1024, offset):
                self.write(number, offset, chunk)
                offset += len(chunk)
            self.flush(number)
            # Persist the remaining namespace step before asking the server.
            handle.dirty = True
            self._journal(handle, 'replacement-uploaded-source-trash-pending')
            if _revision(self.backend.info(source_node.path)) != _revision(source_node):
                _fail(errno.ESTALE, 'Replacement uploaded but source changed; both files retained')
            self.backend.trash([source_node.path])
            self.children[source.parent].pop(source_node.name, None)
            self.pinned.discard(source.inode)
            self._save()
            handle.dirty = False
            self._journal(handle, 'clean')
        except Exception:
            if number is not None and self.handles[number].dirty:
                handle.failed = 'File replacement was interrupted; recovery retains its content and operation details.'
                self.last_error = handle.failed
                self._journal(handle, 'recovery')
            raise
        finally:
            os.close(source_fd)
            if number is not None:
                self.release(number)

    def rename(self, parent, name, new_parent, new_name, *, no_replace=False):
        with self._mutex:
            self._writable()
            _safe_name(new_name)
            source = self.lookup(parent, name)
            self._not_open(source.inode)
            if parent == new_parent and name == new_name:
                return
            # Keep the previously observed target baseline before fetching a
            # fresh listing; a refresh must not bless another client's edit.
            previous = self.children.get(new_parent, {}).get(new_name)
            expected = self.entries[previous].node if previous else None
            self.refresh(new_parent)
            if new_name in self.children[new_parent]:
                if no_replace:
                    _fail(errno.EEXIST, 'Destination already exists')
                target = self.entries[self.children[new_parent][new_name]]
                if expected and _revision(expected) != _revision(target.node):
                    _fail(errno.ESTALE, 'Destination changed before replacement; source retained')
                return self._replace_file(source, target)
            if parent != new_parent and name != new_name:
                _fail(errno.EXDEV, 'Move and rename in two separate operations')
            if parent == new_parent:
                self.backend.rename(source.node.path, new_name)
            else:
                self.backend.move([source.node.path], self._entry(new_parent).node.path)
            self.refresh(parent)
            if new_parent != parent:
                self.refresh(new_parent)

    def unlink(self, parent, name, *, directory=False):
        with self._mutex:
            self._writable()
            entry = self.lookup(parent, name)
            self._not_open(entry.inode)
            if directory != (entry.node.type == 'folder'):
                _fail(errno.ENOTDIR if directory else errno.EISDIR, 'Wrong entry type')
            if directory and self.refresh(entry.inode):
                _fail(errno.ENOTEMPTY, 'Directory is not empty')
            self._check_root()
            before = self.backend.info(entry.node.path)
            if _revision(before) != _revision(entry.node):
                _fail(errno.ESTALE, 'Remote entry changed before trashing')
            self.backend.trash([entry.node.path])
            self.children[parent].pop(name, None)
            self.pinned.discard(entry.inode)
            self._save()

    def close(self):
        with self._mutex:
            if self._closed:
                return
            try:
                for number in list(self.handles):
                    try:
                        self.release(number)
                    except OSError:
                        self.last_error = 'Local recovery metadata could not be updated; preserve the cache directory.'
                try:
                    self._save()
                except OSError:
                    self.last_error = 'Mount metadata could not be saved; preserve the cache directory.'
            finally:
                os.close(self._lock_fd)
                self._closed = True
