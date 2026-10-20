"""Recoverable, same-filesystem trash with a private operation journal.

Desktop trash follows https://specifications.freedesktop.org/trash/latest/ .
No content is ever unlinked or copied then deleted. Linux renameat2(NOREPLACE)
provides atomic destination exclusion. Linux has no inode-conditional rename:
pre/post identity+content proofs detect a concurrent source replacement; rollback
is attempted without overwrite, otherwise the preserved object is recovery data.
Only this application's receipts are listed. Desktop trash may independently be
restored/emptied by the user; a missing object is reported, never reconstructed.
"""
from __future__ import annotations

from contextlib import contextmanager
import ctypes
from dataclasses import asdict, dataclass
from datetime import datetime
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import time
from typing import Mapping
from urllib.parse import quote_from_bytes
import uuid

from .backend import BackendError, Cancelled, data_dir

DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
MAX_JOURNAL = 64 * 1024 * 1024
MAX_ENTRIES = 100000


@dataclass(frozen=True)
class TrashReceipt:
    id: str
    original_path: str
    trash_path: str
    metadata_path: str
    deleted_at: str
    kind: str
    status: str
    backend: str
    error: str = ""
    restore_path: str = ""

    def to_dict(self):
        return asdict(self)


class LocalTrashError(BackendError):
    def __init__(self, message, receipt=None):
        super().__init__(message)
        self.receipt = receipt


def _absolute(path):
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise LocalTrashError('An absolute path without dot components is required')
    return path


def _parts(relative):
    if not isinstance(relative, str) or not relative or str(PurePosixPath(relative)) != relative:
        raise LocalTrashError('Invalid relative trash path')
    path = PurePosixPath(relative)
    if path.is_absolute() or any(p in {'.', '..'} or '\\' in p or '\x00' in p for p in path.parts):
        raise LocalTrashError('Unsafe relative trash path')
    return path.parts


def _identity(info):
    return info.st_dev, info.st_ino


def _stamp(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def _mount_id(fd):
    for line in Path(f'/proc/self/fdinfo/{fd}').read_text().splitlines():
        if line.startswith('mnt_id:'):
            return int(line.split(':', 1)[1])
    raise LocalTrashError('Cannot verify the Linux mount identity')


@contextmanager
def _directory(path, *, create=False, private=False):
    path = _absolute(path)
    fd = os.open('/', DIR)
    try:
        for part in path.parts[1:]:
            if create:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                    os.fsync(fd)
                except FileExistsError:
                    pass
            child = os.open(part, DIR, dir_fd=fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        if private and (info.st_uid != os.getuid() or info.st_mode & 0o077):
            raise LocalTrashError(f'Trash/state directory must be owned by this user and private: {path}')
        yield fd
    finally:
        os.close(fd)


@contextmanager
def _parent(root_fd, parts):
    fd = os.dup(root_fd)
    chain = [_identity(os.fstat(fd))]
    try:
        for part in parts[:-1]:
            child = os.open(part, DIR, dir_fd=fd)
            os.close(fd)
            fd = child
            chain.append(_identity(os.fstat(fd)))
        yield fd, parts[-1], chain
    finally:
        os.close(fd)


def _check_chain(root_path, parts, chain):
    with _directory(root_path) as fd, _parent(fd, parts) as (_, __, actual):
        if [list(x) for x in actual] != [list(x) for x in chain]:
            raise LocalTrashError('The sync root or an ancestor directory changed')


def _check_directory(path, fd):
    with _directory(path, private=True) as current:
        if _identity(os.fstat(current)) != _identity(os.fstat(fd)):
            raise LocalTrashError('Trash directory was replaced during the operation')


def _rename_noreplace(source_fd, source, destination_fd, destination):
    library = ctypes.CDLL(None, use_errno=True)
    rename = getattr(library, 'renameat2', None)
    if rename is None:
        raise LocalTrashError('Atomic non-overwriting renameat2 is unavailable')
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(source_fd, os.fsencode(source), destination_fd, os.fsencode(destination), 1) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def _snapshot(parent_fd, name, check_cancel=None):
    """Complete nofollow proof; refuses links, hardlinks, mounts and special files."""
    result = {}
    initial = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    device = initial.st_dev
    mount_id = _mount_id(parent_fd)
    ancestors = set()

    def visit(fd, child_name, relative):
        if check_cancel:
            check_cancel()
        if len(result) >= MAX_ENTRIES:
            raise LocalTrashError('Trash tree exceeds the verification limit')
        before = os.stat(child_name, dir_fd=fd, follow_symlinks=False)
        if before.st_dev != device:
            raise LocalTrashError('Nested filesystems cannot be moved into this trash')
        if stat.S_ISDIR(before.st_mode):
            child = os.open(child_name, DIR, dir_fd=fd)
            try:
                opened = os.fstat(child)
                if _identity(opened) != _identity(before) or _mount_id(child) != mount_id or _identity(opened) in ancestors:
                    raise LocalTrashError('Directory replaced during trash verification')
                ancestors.add(_identity(opened))
                names = sorted(os.listdir(child))
                result[relative] = dict(kind='folder', device=opened.st_dev, inode=opened.st_ino,
                                        size=0, mtime_ns=opened.st_mtime_ns, ctime_ns=opened.st_ctime_ns, sha1='')
                for entry in names:
                    visit(child, entry, relative + '/' + entry if relative else entry)
                after = os.fstat(child)
                if _stamp(opened) != _stamp(after) or sorted(os.listdir(child)) != names:
                    raise LocalTrashError('Directory contents changed during trash verification')
                # File fsync does not commit newly created descendant names.
                # Sync every directory before the top-level atomic move.
                os.fsync(child)
                ancestors.remove(_identity(opened))
            finally:
                os.close(child)
        elif stat.S_ISREG(before.st_mode) and before.st_nlink == 1:
            child = os.open(child_name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=fd)
            try:
                opened = os.fstat(child)
                if not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1 or _stamp(before) != _stamp(opened) or _mount_id(child) != mount_id:
                    raise LocalTrashError('File replaced during trash verification')
                digest = hashlib.sha1()
                while data := os.read(child, 1024 * 1024):
                    if check_cancel:
                        check_cancel()
                    digest.update(data)
                after = os.fstat(child)
                if _stamp(opened) != _stamp(after):
                    raise LocalTrashError('File changed during trash verification')
                os.fsync(child)
                result[relative] = dict(kind='file', device=after.st_dev, inode=after.st_ino,
                                        size=after.st_size, mtime_ns=after.st_mtime_ns,
                                        ctime_ns=after.st_ctime_ns, sha1=digest.hexdigest())
            finally:
                os.close(child)
        else:
            raise LocalTrashError('Links and special files cannot be automatically trashed')
        final = os.stat(child_name, dir_fd=fd, follow_symlinks=False)
        if _stamp(before) != _stamp(final):
            raise LocalTrashError('Source changed during trash verification')
    visit(parent_fd, name, '')
    return result


def _prove_expected(snapshot, relative, expected, expected_tree):
    values = {relative: expected}
    if snapshot['']['kind'] == 'folder':
        if expected_tree is None:
            raise LocalTrashError('Trashing a folder requires a complete unchanged subtree proof')
        values = dict(expected_tree)
    wanted = {relative + '/' + name if name else relative for name in snapshot}
    if set(values) != wanted:
        raise LocalTrashError('The local subtree differs from the deletion plan')
    for suffix, actual in snapshot.items():
        item = values[relative + '/' + suffix if suffix else relative]
        get = (lambda key, default=None: item.get(key, default)) if isinstance(item, Mapping) else (lambda key, default=None: getattr(item, key, default))
        fields = ('kind', 'device', 'inode')
        if actual['kind'] == 'file':
            fields += ('size', 'sha1', 'mtime_ns')
            if get('ctime_ns'):
                fields += ('ctime_ns',)
        if any(get(key) != actual[key] for key in fields):
            raise LocalTrashError('The local object changed since the deletion plan')


def _same_snapshot(left, right, *, renamed=False):
    if left.keys() != right.keys():
        return False
    for name, first in left.items():
        second = dict(right[name])
        first = dict(first)
        if renamed and name == '':
            # rename changes only the moved object's ctime, not its contents.
            first.pop('ctime_ns', None)
            second.pop('ctime_ns', None)
        if first != second:
            return False
    return True


def _mount_top(path):
    path = _absolute(path)
    candidates = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        fields = line.split()
        if len(fields) < 6:
            continue
        mount = Path(re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), fields[4]))
        if path == mount or mount in path.parents:
            candidates.append(mount)
    if not candidates:
        raise LocalTrashError('Cannot determine filesystem mount point')
    return max(candidates, key=lambda p: len(p.parts))


def _outside(path, root):
    return path != root and root not in path.parents


def _write_exclusive(fd, name, contents):
    file_fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=fd)
    try:
        with os.fdopen(file_fd, 'wb', closefd=False) as stream:
            stream.write(contents)
            stream.flush()
            os.fsync(file_fd)
    finally:
        os.close(file_fd)
    os.fsync(fd)


def _read_json(fd, name):
    file_fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
    try:
        info = os.fstat(file_fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1 or info.st_mode & 0o077 or info.st_size > MAX_JOURNAL:
            raise LocalTrashError('Unsafe trash journal')
        with os.fdopen(file_fd, closefd=False) as stream:
            return json.load(stream)
    finally:
        os.close(file_fd)


class LocalTrash:
    def __init__(self, state_dir=None, *, data_home=None, prefer_desktop=True):
        self.state_dir = _absolute(state_dir if state_dir is not None else data_dir() / 'local-trash')
        self.data_home = _absolute(data_home if data_home is not None else os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        self.prefer_desktop = prefer_desktop
        self.entries_dir = self.state_dir / 'entries'
        with _directory(self.state_dir, create=True, private=True), _directory(self.entries_dir, create=True, private=True):
            pass

    @contextmanager
    def _locked(self, check_cancel=None, timeout=5):
        with _directory(self.state_dir, private=True) as state_fd, _directory(self.entries_dir, private=True) as entries_fd:
            fd = os.open('lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=state_fd)
            try:
                info = os.fstat(fd)
                if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1 or info.st_mode & 0o077:
                    raise LocalTrashError('Unsafe trash lock')
                deadline = time.monotonic() + timeout
                while True:
                    if check_cancel:
                        check_cancel()
                    try:
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError:
                        if time.monotonic() >= deadline:
                            raise LocalTrashError('Another local trash operation is busy; retry later')
                        time.sleep(0.05)
                yield entries_fd
            finally:
                os.close(fd)

    @staticmethod
    def _receipt(record):
        return TrashReceipt(**{key: record.get(key, '') for key in TrashReceipt.__dataclass_fields__})

    def _save(self, fd, record, *, initial=False):
        content = json.dumps(record, ensure_ascii=True, sort_keys=True).encode()
        if len(content) > MAX_JOURNAL:
            raise LocalTrashError('Trash journal is too large')
        name = record['id'] + '.json'
        if initial:
            _write_exclusive(fd, name, content)
        else:
            temporary = '.' + str(uuid.uuid4()) + '.tmp'
            _write_exclusive(fd, temporary, content)
            os.replace(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
            os.fsync(fd)

    def _candidate(self, path, root, device, backend, top=None):
        if not _outside(path, root):
            raise LocalTrashError('Trash must be outside the synchronized folder')
        with _directory(path, create=True, private=True) as fd:
            if os.fstat(fd).st_dev != device:
                raise LocalTrashError('Trash is on a different filesystem; copy-delete is disabled')
        for child in ('files', 'info'):
            with _directory(path / child, create=True, private=True) as fd:
                if os.fstat(fd).st_dev != device:
                    raise LocalTrashError('Trash subdirectory is on a different filesystem')
        return path, backend, top

    def _choose(self, root, device):
        if self.prefer_desktop:
            home = self.data_home / 'Trash'
            top = _mount_top(root)
            home_top = _mount_top(self.data_home)
            try:
                if top == home_top:
                    return self._candidate(home, root, device, 'desktop')
            except (OSError, LocalTrashError):
                pass
            if top != home_top and _outside(top / f'.Trash-{os.getuid()}', root):
                try:
                    with _directory(top / '.Trash') as fd:
                        if not os.fstat(fd).st_mode & stat.S_ISVTX:
                            raise LocalTrashError('Shared trash is not sticky')
                    return self._candidate(top / '.Trash' / str(os.getuid()), root, device, 'desktop', top)
                except (OSError, LocalTrashError):
                    pass
                try:
                    return self._candidate(top / f'.Trash-{os.getuid()}', root, device, 'desktop', top)
                except (OSError, LocalTrashError):
                    pass
        # Explicit managed recovery is not advertised as a Desktop Trash folder.
        # It is never auto-cleaned and is only used outside the synchronized root.
        return self._candidate(root.parent / f'.neutrondrive-trash-{os.getuid()}', root, device, 'managed')

    def trash(self, root_fd, relative, expected_entry, *, root_path, expected_tree=None, validate=None, operation_id=None, check_cancel=None):
        record = None
        registered = False
        try:
            root = _absolute(root_path)
            parts = _parts(relative)
            if not _outside(self.state_dir, root):
                raise LocalTrashError('Trash journal must be outside the synchronized folder')
            identifier = str(uuid.UUID(operation_id)) if operation_id else str(uuid.uuid4())
            with self._locked(check_cancel) as entries_fd, _parent(root_fd, parts) as (parent_fd, name, chain):
                _check_chain(root, parts, chain)
                snapshot = _snapshot(parent_fd, name, check_cancel)
                _prove_expected(snapshot, relative, expected_entry, expected_tree)
                trash, backend, top = self._choose(root, snapshot['']['device'])
                # A readable prefix helps manual recovery; UUID avoids collisions.
                trash_name = os.fsdecode(os.fsencode(name)[:80]) + '-' + identifier
                deleted_at = datetime.now().astimezone().strftime('%Y-%m-%dT%H:%M:%S')
                record = dict(id=identifier, original_path=str(root.joinpath(*parts)),
                              trash_path=str(trash / 'files' / trash_name), metadata_path=str(trash / 'info' / (trash_name + '.trashinfo')),
                              deleted_at=deleted_at, kind=snapshot['']['kind'], status='prepared', backend=backend, error='',
                              root_path=str(root), relative=relative, parent_chain=chain, snapshot=snapshot)
                try:
                    self._save(entries_fd, record, initial=True)
                except FileExistsError:
                    existing = _read_json(entries_fd, identifier + '.json')
                    record = None
                    raise LocalTrashError('This trash operation already has a receipt; inspect it before retrying', self._receipt(existing))
                registered = True
                with _directory(trash / 'files', private=True) as files_fd, _directory(trash / 'info', private=True) as info_fd:
                    original = Path(record['original_path'])
                    stored_path = str(original.relative_to(top)) if top else str(original)
                    info = '[Trash Info]\nPath=' + quote_from_bytes(os.fsencode(stored_path), safe='/') + '\nDeletionDate=' + deleted_at + '\n'
                    _write_exclusive(info_fd, trash_name + '.trashinfo', info.encode())
                    _check_chain(root, parts, chain)
                    before = _snapshot(parent_fd, name, check_cancel)
                    if not _same_snapshot(snapshot, before):
                        raise LocalTrashError('The source changed before it could be moved to trash')
                    # The last cloud/account proof belongs after expensive local
                    # hashing. Only cheap path/identity checks follow it.
                    if validate:
                        validate()
                    if check_cancel:
                        check_cancel()
                    _check_chain(root, parts, chain)
                    final = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
                    expected = before['']
                    if (_identity(final) != (expected['device'], expected['inode']) or
                            final.st_mtime_ns != expected['mtime_ns'] or final.st_ctime_ns != expected['ctime_ns'] or
                            (expected['kind'] == 'file' and final.st_size != expected['size'])):
                        raise LocalTrashError('Source changed during final deletion validation')
                    _check_directory(trash / 'files', files_fd)
                    _check_directory(trash / 'info', info_fd)
                    _rename_noreplace(parent_fd, name, files_fd, trash_name)
                    try:
                        os.fsync(parent_fd)
                        os.fsync(files_fd)
                        after = _snapshot(files_fd, trash_name, check_cancel)
                        if not _same_snapshot(snapshot, after, renamed=True):
                            raise LocalTrashError('Source was replaced or modified during the move')
                        _check_chain(root, parts, chain)
                        _check_directory(trash / 'files', files_fd)
                    except Exception:
                        # Roll back only into the original still-identical parent.
                        # If anything prevents this, leave content in recovery.
                        try:
                            _check_chain(root, parts, chain)
                            _rename_noreplace(files_fd, trash_name, parent_fd, name)
                            os.fsync(parent_fd)
                            os.fsync(files_fd)
                            record['status'] = 'aborted'
                        except Exception:
                            record['status'] = 'recovery'
                            record['trash_path'] = str(Path(os.readlink(f'/proc/self/fd/{files_fd}')) / trash_name)
                            record['metadata_path'] = str(Path(os.readlink(f'/proc/self/fd/{info_fd}')) / (trash_name + '.trashinfo'))
                        raise
                    record['snapshot'] = after
                    record['status'] = 'trashed'
                    self._save(entries_fd, record)
                    return self._receipt(record)
        except Exception as error:
            if record and registered:
                if record['status'] not in {'recovery', 'aborted'}:
                    record['status'] = 'recovery'
                record['error'] = str(error)
                try:
                    with self._locked() as fd:
                        self._save(fd, record)
                except Exception:
                    pass  # Previously fsynced prepared receipt is still useful.
            if isinstance(error, (LocalTrashError, Cancelled)):
                if record:
                    error.receipt = self._receipt(record)
                raise
            raise LocalTrashError(str(error), self._receipt(record) if record else None) from error

    def list_entries(self):
        entries = []
        with self._locked() as fd:
            for name in sorted(os.listdir(fd)):
                if not re.fullmatch(r'[0-9a-f-]{36}\.json', name):
                    continue
                record = _read_json(fd, name)
                if record.get('id') + '.json' != name:
                    raise LocalTrashError('Trash journal identity mismatch')
                if record['status'] not in {'restored', 'aborted'}:
                    try:
                        path = Path(record['trash_path'])
                        with _directory(path.parent, private=True) as parent:
                            info = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
                            if _identity(info) != (record['snapshot']['']['device'], record['snapshot']['']['inode']):
                                record = dict(record, status='recovery', error='Trash object identity changed; automatic restore is blocked')
                    except (FileNotFoundError, NotADirectoryError):
                        if record.get('restore_path') and record['status'] in {'restoring', 'recovery'}:
                            record = dict(record, status='recovery', error='Interrupted restore; inspect or retry the recorded destination: ' + record['restore_path'])
                        else:
                            record = dict(record, status='missing', error='Trash object is unavailable or was handled by the desktop')
                    except (OSError, LocalTrashError) as error:
                        record = dict(record, status='recovery', error=str(error))
                entries.append(self._receipt(record))
        return entries

    def get_receipt(self, identifier):
        identifier = str(uuid.UUID(identifier))
        return next((entry for entry in self.list_entries() if entry.id == identifier), None)

    def restore(self, identifier, destination=None, *, check_cancel=None):
        identifier = str(uuid.UUID(identifier))
        record = None
        try:
            with self._locked(check_cancel) as entries_fd:
                record = _read_json(entries_fd, identifier + '.json')
                if record.get('id') != identifier or record['status'] in {'restored', 'aborted'}:
                    raise LocalTrashError('This receipt cannot be restored again')
                original = destination is None
                target = _absolute(record['original_path'] if original else destination)
                if original:
                    _check_chain(Path(record['root_path']), _parts(record['relative']), record['parent_chain'])
                payload = _absolute(record['trash_path'])
                # A crash after rename but before journal commit must not hide
                # the restored object or cause a second destructive operation.
                if record.get('restore_path') and record['status'] in {'restoring', 'recovery'}:
                    with _directory(payload.parent, private=True) as fd:
                        try:
                            os.stat(payload.name, dir_fd=fd, follow_symlinks=False)
                        except FileNotFoundError:
                            previous = _absolute(record['restore_path'])
                            if destination is not None and target != previous:
                                raise LocalTrashError('An interrupted restore has a different recorded destination')
                            with _directory(previous.parent) as restored_fd:
                                if list(_identity(os.fstat(restored_fd))) != record.get('restore_parent_identity'):
                                    raise LocalTrashError('Interrupted restore parent changed; inspect ' + str(previous))
                                actual = _snapshot(restored_fd, previous.name, check_cancel)
                                if not _same_snapshot(record['snapshot'], actual, renamed=True):
                                    raise LocalTrashError('Interrupted restore contents changed; inspect ' + str(previous))
                                os.fsync(restored_fd)
                            record['status'], record['error'] = 'restored', ''
                            self._save(entries_fd, record)
                            return self._receipt(record)
                with _directory(payload.parent, private=True) as files_fd, _directory(target.parent) as destination_fd:
                    before = _snapshot(files_fd, payload.name, check_cancel)
                    if not _same_snapshot(record['snapshot'], before, renamed=record['status'] in {'prepared', 'recovery'}):
                        raise LocalTrashError('Trash contents changed; inspect the recovery copy before restoring')
                    if os.fstat(destination_fd).st_dev != before['']['device']:
                        raise LocalTrashError('Restore must remain on the same filesystem')
                    record['status'] = 'restoring'
                    record['restore_path'] = str(target)
                    record['restore_parent_identity'] = list(_identity(os.fstat(destination_fd)))
                    self._save(entries_fd, record)
                    _check_directory(payload.parent, files_fd)
                    with _directory(target.parent) as check_fd:
                        if _identity(os.fstat(check_fd)) != _identity(os.fstat(destination_fd)):
                            raise LocalTrashError('Restore destination parent changed')
                    if original:
                        _check_chain(Path(record['root_path']), _parts(record['relative']), record['parent_chain'])
                    _rename_noreplace(files_fd, payload.name, destination_fd, target.name)
                    os.fsync(destination_fd)
                    os.fsync(files_fd)
                    after = _snapshot(destination_fd, target.name, check_cancel)
                    if not _same_snapshot(before, after, renamed=True):
                        raise LocalTrashError('Restored object changed; preserved at the restore destination')
                    try:
                        with _directory(target.parent) as check_fd:
                            if _identity(os.fstat(check_fd)) != _identity(os.fstat(destination_fd)):
                                raise LocalTrashError('Restore destination parent changed')
                    except (OSError, LocalTrashError) as error:
                        # Also handle a parent moved away without a replacement.
                        record['restore_path'] = str(Path(os.readlink(f'/proc/self/fd/{destination_fd}')) / target.name)
                        raise LocalTrashError('Restore parent moved concurrently; content remains at ' + record['restore_path']) from error
                    if original:
                        _check_chain(Path(record['root_path']), _parts(record['relative']), record['parent_chain'])
                    record['status'] = 'restored'
                    record['error'] = ''
                    self._save(entries_fd, record)
                    # Leave the small .trashinfo record: removing it could race a
                    # desktop replacing that entry; it contains no file content.
                    return self._receipt(record)
        except Exception as error:
            if record and record.get('status') == 'restoring':
                record['status'], record['error'] = 'recovery', str(error)
                try:
                    with self._locked() as fd:
                        self._save(fd, record)
                except Exception:
                    pass
            if isinstance(error, (LocalTrashError, Cancelled)):
                error.receipt = self._receipt(record) if record else None
                raise
            raise LocalTrashError(str(error), self._receipt(record) if record else None) from error
