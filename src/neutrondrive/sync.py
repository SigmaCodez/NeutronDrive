"""Explicit, conservative two-way sync using Proton's official CLI adapter.

There is deliberately no watcher or background remote tree polling. Deletions,
ambiguous names, native Proton documents and simultaneous edits need a human.
Uploads use version history; the CLI does not expose conditional revision writes,
so another client can still race the final remote check. Local replacements keep
the previous inode as a visible, excluded ``.neutrondrive-backup-*`` sibling.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tempfile
import threading
from typing import Any, Callable, Iterator
import uuid

from .backend import Cancelled


class SyncError(RuntimeError):
    """A scan or transfer could not be completed safely."""


@dataclass(frozen=True)
class SyncAction:
    kind: str
    relative_path: str
    detail: str = ""

    def __str__(self) -> str:
        return f"{self.kind}: {self.relative_path}" + (f" — {self.detail}" if self.detail else "")


@dataclass(frozen=True)
class SyncConflict:
    relative_path: str
    reason: str

    def __str__(self) -> str:
        return f"{self.relative_path}: {self.reason}"


@dataclass(frozen=True)
class _Entry:
    kind: str
    sha1: str = ""
    size: int = 0
    uid: str = ""
    revision: str = ""
    remote_path: str = ""
    mtime_ns: int = 0
    inode: int = 0
    device: int = 0


@dataclass
class SyncPlan:
    local_root: Path
    remote_root: str
    remote_root_uid: str
    actions: list[SyncAction] = field(default_factory=list)
    conflicts: list[SyncConflict] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)
    _local: dict[str, _Entry] = field(default_factory=dict, repr=False)
    _remote: dict[str, _Entry] = field(default_factory=dict, repr=False)
    _baseline: dict[str, dict[str, str]] = field(default_factory=dict, repr=False)
    _state_digest: str = field(default="", repr=False)
    _state_key: str = field(default="", repr=False)
    _root_identity: tuple[int, int] = field(default=(0, 0), repr=False)
    _blocked: set[str] = field(default_factory=set, repr=False)
    _owner: str = field(default="", repr=False)
    _executed: bool = field(default=False, repr=False)

    def summary(self) -> str:
        counts: dict[str, int] = {}
        for action in self.actions:
            counts[action.kind] = counts.get(action.kind, 0) + 1
        changes = ", ".join(f"{kind}: {count}" for kind, count in sorted(counts.items()))
        return f"{changes or 'No transfers'}; conflicts: {len(self.conflicts)}; skipped: {len(self.skipped)}. No deletions."


@dataclass
class SyncResult:
    completed: list[str] = field(default_factory=list)
    conflicts: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    backups: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors and not self.conflicts

    def summary(self) -> str:
        return (f"Completed: {len(self.completed)}; conflicts: {len(self.conflicts)}; "
                f"errors: {len(self.errors)}; skipped: {len(self.skipped)}; local backups: {len(self.backups)}.")


_RESERVED = ".neutrondrive-"
_SHA1 = re.compile(r"^[0-9a-fA-F]{40}$")
_DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


def _name_error(name: str) -> str:
    if not name or name in {".", ".."} or "/" in name or "\\" in name:
        return "Unsafe or ambiguous path component"
    if any(ord(char) < 32 or ord(char) == 127 for char in name):
        return "Control characters in file name are unsupported"
    if any(char in name for char in "*?[{"):
        return "Name contains glob syntax interpreted by the Proton CLI"
    if name.startswith(_RESERVED):
        return "Reserved local backup/staging name; excluded from sync"
    return ""


def _parts(relative: str) -> tuple[str, ...]:
    path = PurePosixPath(relative)
    if path.is_absolute() or not relative or str(path) != relative:
        raise SyncError(f"Invalid relative path: {relative!r}")
    for part in path.parts:
        if _name_error(part):
            raise SyncError(f"Unsafe relative path: {relative!r}")
    return path.parts


def _under(path: str, blocked: set[str]) -> bool:
    return any(path == item or path.startswith(item + "/") for item in blocked)


@contextmanager
def _open_absolute_directory(path: Path) -> Iterator[int]:
    """Open every component without following links, including ancestors."""
    if not path.is_absolute():
        raise SyncError("An absolute local directory is required")
    fd = os.open("/", _DIR_FLAGS)
    try:
        for part in path.parts[1:]:
            if part in {".", ".."}:
                raise SyncError("Dot components are not allowed in local roots")
            next_fd = os.open(part, _DIR_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        yield fd
    finally:
        os.close(fd)


@contextmanager
def _parent(root_fd: int, relative: str) -> Iterator[tuple[int, str]]:
    components = _parts(relative)
    fd = os.dup(root_fd)
    try:
        for component in components[:-1]:
            next_fd = os.open(component, _DIR_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        yield fd, components[-1]
    finally:
        os.close(fd)


def _stat_identity(info: os.stat_result) -> tuple[int, int, int, int]:
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns


def _read_file(fd: int, output: Any = None, check_cancel: Callable[[], None] | None = None) -> tuple[str, os.stat_result]:
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode):
        raise SyncError("Only regular files can be synchronized")
    digest = hashlib.sha1()
    while True:
        if check_cancel is not None:
            check_cancel()
        chunk = os.read(fd, 1024 * 1024)
        if not chunk:
            break
        digest.update(chunk)
        if output is not None:
            output.write(chunk)
    after = os.fstat(fd)
    if _stat_identity(before) != _stat_identity(after) or before.st_ctime_ns != after.st_ctime_ns:
        raise SyncError("Local file changed while being read")
    return digest.hexdigest(), after


def _local_entry(parent_fd: int, name: str, check_cancel: Callable[[], None] | None = None) -> _Entry | None:
    try:
        info = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError:
        return None
    if stat.S_ISDIR(info.st_mode):
        return _Entry("folder", inode=info.st_ino, device=info.st_dev)
    if not stat.S_ISREG(info.st_mode):
        raise SyncError("Symbolic links and non-regular files are excluded")
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=parent_fd)
    try:
        sha1, after = _read_file(fd, check_cancel=check_cancel)
        if (info.st_dev, info.st_ino) != (after.st_dev, after.st_ino):
            raise SyncError("Local file was replaced while being scanned")
        return _Entry("file", sha1, after.st_size, mtime_ns=after.st_mtime_ns,
                      inode=after.st_ino, device=after.st_dev)
    finally:
        os.close(fd)


def _kind(node: Any) -> str:
    value = str(node.type).lower()
    return "folder" if value in {"folder", "directory"} else value


def _remote_entry(node: Any) -> _Entry:
    if not getattr(node, "safe", True):
        raise SyncError("Remote metadata or decrypted name is not trustworthy")
    media_type = str(getattr(node, "media_type", "") or "").lower()
    if "proton" in media_type and any(word in media_type for word in ("doc", "sheet")):
        raise SyncError("Native Proton Docs/Sheets cannot be exported by the CLI")
    kind = _kind(node)
    if kind not in {"folder", "file"}:
        raise SyncError(f"Unsupported remote node type: {kind}")
    digest = str(getattr(node, "sha1", "") or "").lower()
    if kind == "file" and not _SHA1.fullmatch(digest):
        raise SyncError("Remote file has no usable SHA-1 digest (draft/native document/metadata unavailable)")
    if not node.uid or not node.path:
        raise SyncError("Remote node has no stable identifier or address")
    return _Entry(kind, digest, int(getattr(node, "size", 0) or 0), str(node.uid),
                  str(getattr(node, "revision", "") or ""), str(node.path))


def _same_remote(a: _Entry | None, b: _Entry | None) -> bool:
    if a is None or b is None:
        return a is b
    return (a.kind, a.uid, a.revision, a.sha1, a.size) == (b.kind, b.uid, b.revision, b.sha1, b.size)


class SyncEngine:
    """Plan first, then execute once after explicit confirmation in the UI."""

    def __init__(self, backend: Any, state_dir: Path):
        self.backend = backend
        self.state_dir = Path(os.path.abspath(state_dir.expanduser()))
        self._lock = threading.RLock()
        self._owner = uuid.uuid4().hex

    def _check_cancel(self) -> None:
        event = getattr(self.backend, "cancel_event", None)
        if event is not None and event.is_set():
            raise Cancelled("Synchronisierung abgebrochen.")

    def _prepare_state(self) -> None:
        self.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        with _open_absolute_directory(self.state_dir) as fd:
            info = os.fstat(fd)
            if info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise SyncError("Sync state directory must be owned by you with mode 0700")

    def _root(self, remote_path: str) -> Any:
        node = self.backend.info(remote_path)
        if _remote_entry(node).kind != "folder":
            raise SyncError("The remote sync root must be a folder")
        return node

    def _scan_local(self, root_fd: int) -> tuple[dict[str, _Entry], dict[str, str]]:
        entries: dict[str, _Entry] = {}
        blocked: dict[str, str] = {}

        def visit(fd: int, prefix: str, ancestors: set[tuple[int, int]]) -> None:
            self._check_cancel()
            with os.scandir(fd) as children:
                names = sorted(item.name for item in children)
            for name in names:
                self._check_cancel()
                relative = f"{prefix}/{name}" if prefix else name
                reason = _name_error(name)
                if reason:
                    blocked[relative] = reason
                    continue
                try:
                    entry = _local_entry(fd, name, self._check_cancel)
                    if entry is None:
                        raise SyncError("File disappeared during scan")
                    entries[relative] = entry
                    if entry.kind == "folder":
                        identity = entry.device, entry.inode
                        if identity in ancestors:
                            raise SyncError("Directory cycle or duplicate ancestor")
                        child_fd = os.open(name, _DIR_FLAGS, dir_fd=fd)
                        try:
                            child_stat = os.fstat(child_fd)
                            if identity != (child_stat.st_dev, child_stat.st_ino):
                                raise SyncError("Directory was replaced during scan")
                            visit(child_fd, relative, ancestors | {identity})
                        finally:
                            os.close(child_fd)
                except (OSError, SyncError) as error:
                    blocked[relative] = str(error)

        root_stat = os.fstat(root_fd)
        visit(root_fd, "", {(root_stat.st_dev, root_stat.st_ino)})
        return entries, blocked

    def _node_entry(self, node: Any) -> _Entry:
        if _kind(node) == "file" and not getattr(node, "sha1", ""):
            node = self.backend.info(node.path)
        return _remote_entry(node)

    def _scan_remote(self, root: Any) -> tuple[dict[str, _Entry], dict[str, str]]:
        entries: dict[str, _Entry] = {}
        blocked: dict[str, str] = {}
        visited = {str(root.uid)}

        def visit(path: str, prefix: str) -> None:
            self._check_cancel()
            grouped: dict[str, list[Any]] = {}
            for node in self.backend.list_nodes(path):
                grouped.setdefault(node.name, []).append(node)
            for name, nodes in sorted(grouped.items()):
                self._check_cancel()
                relative = f"{prefix}/{name}" if prefix else name
                reason = _name_error(name)
                if reason or len(nodes) != 1:
                    blocked[relative] = reason or "Multiple remote nodes have the same name"
                    continue
                node = nodes[0]
                try:
                    entry = self._node_entry(node)
                    if entry.uid in visited:
                        raise SyncError("Remote tree contains a repeated node identifier")
                    visited.add(entry.uid)
                    entries[relative] = entry
                    if entry.kind == "folder":
                        visit(entry.remote_path, relative)
                except SyncError as error:
                    blocked[relative] = str(error)

        visit(str(root.path), "")
        return entries, blocked

    def _load(self, key: str, root: Path, uid: str) -> tuple[dict[str, dict[str, str]], str]:
        path = self.state_dir / (key + ".json")
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except FileNotFoundError:
            return {}, ""
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_size > 32 * 1024 * 1024:
                raise SyncError("Invalid or oversized sync baseline")
            with os.fdopen(fd, "rb", closefd=False) as stream:
                raw = stream.read()
            value = json.loads(raw)
            if (value.get("version") != 1 or value.get("local_root") != str(root)
                    or value.get("remote_root_uid") != uid or not isinstance(value.get("entries"), dict)):
                raise SyncError("Sync baseline identity/schema does not match")
            entries = value["entries"]
            for relative, entry in entries.items():
                _parts(relative)
                if (not isinstance(entry, dict) or entry.get("kind") not in {"folder", "file"}
                        or (entry["kind"] == "file" and not _SHA1.fullmatch(entry.get("sha1", "")))):
                    raise SyncError("Invalid sync baseline entry")
            return entries, hashlib.sha256(raw).hexdigest()
        except (ValueError, TypeError, AttributeError) as error:
            raise SyncError(f"Corrupt sync baseline; preserved without changes: {error}") from error
        finally:
            os.close(fd)

    def plan(self, local_root: Path, remote_root: str) -> SyncPlan:
        """Scan both complete trees only as a direct user-requested operation."""
        with self._lock:
            self._check_cancel()
            root = Path(os.path.abspath(Path(local_root).expanduser()))
            if self.state_dir == root or root in self.state_dir.parents:
                raise SyncError("The private sync state directory must be outside the synchronized tree")
            self._prepare_state()
            remote = self._root(remote_root)
            key = hashlib.sha256((str(root) + "\0" + str(remote.uid)).encode()).hexdigest()
            baseline, digest = self._load(key, root, str(remote.uid))
            with _open_absolute_directory(root) as fd:
                info = os.fstat(fd)
                local, local_blocked = self._scan_local(fd)
            cloud, cloud_blocked = self._scan_remote(remote)
            blocked = {**local_blocked, **cloud_blocked}
            plan = SyncPlan(root, str(remote.path), str(remote.uid), _local=local, _remote=cloud,
                            _baseline=baseline, _state_digest=digest, _state_key=key,
                            _root_identity=(info.st_dev, info.st_ino), _blocked=set(blocked), _owner=self._owner)
            plan.skipped.extend(f"{path}: {reason}" for path, reason in sorted(blocked.items()))
            for relative in sorted(local.keys() | cloud.keys() | baseline.keys(), key=lambda x: (x.count("/"), x)):
                if _under(relative, plan._blocked):
                    continue
                left, right, previous = local.get(relative), cloud.get(relative), baseline.get(relative)
                reason = ""
                if left is None and right is None:
                    continue
                if left is not None and right is not None:
                    if left.kind != right.kind:
                        reason = "File/folder type conflict"
                    elif left.kind == "folder" or left.sha1 == right.sha1:
                        continue
                    elif not previous or previous.get("kind") != "file":
                        reason = "Both sides already exist with different contents; no common baseline"
                    elif left.sha1 == previous["sha1"]:
                        plan.actions.append(SyncAction("download", relative, "Remote changed; retain a local backup"))
                    elif right.sha1 == previous["sha1"]:
                        plan.actions.append(SyncAction("upload", relative, "Local changed; create a remote revision"))
                    else:
                        reason = "Both sides changed since the last successful synchronization"
                elif previous:
                    reason = "Deleted on one side; deletions and moves require manual resolution"
                elif left is not None:
                    plan.actions.append(SyncAction("mkdir_remote" if left.kind == "folder" else "upload", relative))
                elif right is not None:
                    plan.actions.append(SyncAction("mkdir_local" if right.kind == "folder" else "download", relative))
                if reason:
                    plan.conflicts.append(SyncConflict(relative, reason))
                    plan._blocked.add(relative)
            return plan

    def _lookup_remote(self, root: Any, relative: str) -> _Entry | None:
        current = str(root.path)
        components = _parts(relative)
        for index, name in enumerate(components):
            self._check_cancel()
            matches = [item for item in self.backend.list_nodes(current) if item.name == name]
            if not matches:
                return None
            if len(matches) != 1:
                raise SyncError("Ambiguous remote name appeared after preview")
            entry = self._node_entry(matches[0])
            if index < len(components) - 1 and entry.kind != "folder":
                raise SyncError("Remote parent is no longer a folder")
            current = entry.remote_path
        return entry

    def _remote_parent(self, root: Any, relative: str, expected: dict[str, _Entry]) -> str:
        parent = str(PurePosixPath(relative).parent)
        if parent == ".":
            return str(root.path)
        entry = self._lookup_remote(root, parent)
        if entry is None or entry.kind != "folder" or not _same_remote(expected.get(parent), entry):
            raise SyncError("Remote parent changed after preview")
        return entry.remote_path

    def _stage_upload(self, root_fd: int, relative: str, expected: _Entry, folder: Path) -> Path:
        with _parent(root_fd, relative) as (fd, name):
            source = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
            try:
                target = folder / name
                with target.open("xb") as output:
                    digest, info = _read_file(source, output, self._check_cancel)
                    output.flush()
                    os.fsync(output.fileno())
                actual = _Entry("file", digest, info.st_size, mtime_ns=info.st_mtime_ns,
                                inode=info.st_ino, device=info.st_dev)
                if actual != expected:
                    raise SyncError("Local upload source changed after preview")
                os.utime(target, ns=(info.st_atime_ns, expected.mtime_ns), follow_symlinks=False)
                return target
            finally:
                os.close(source)

    def _install_download(self, root_fd: int, relative: str, source: Path,
                          expected: _Entry | None, result: SyncResult, root: Path) -> None:
        with _parent(root_fd, relative) as (parent_fd, name):
            temp_name = _RESERVED + "stage-" + uuid.uuid4().hex
            backup = ""
            fd = os.open(temp_name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                         0o600, dir_fd=parent_fd)
            try:
                with os.fdopen(fd, "wb") as output, source.open("rb") as downloaded:
                    while chunk := downloaded.read(1024 * 1024):
                        self._check_cancel()
                        output.write(chunk)
                    output.flush()
                    os.fsync(output.fileno())
                if _local_entry(parent_fd, name, self._check_cancel) != expected:
                    raise SyncError("Local destination changed while downloading")
                if expected is not None:
                    backup = _RESERVED + "backup-" + uuid.uuid4().hex
                    os.rename(name, backup, src_dir_fd=parent_fd, dst_dir_fd=parent_fd)
                    result.backups.append(str(root / PurePosixPath(relative).parent / backup))
                    if _local_entry(parent_fd, backup, self._check_cancel) != expected:
                        raise SyncError("Local destination changed during backup; original retained")
                # link() is an atomic no-replace installation, even if another
                # process creates the destination after our last check.
                os.link(temp_name, name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd, follow_symlinks=False)
                os.fsync(parent_fd)
            except BaseException:
                if backup:
                    try:
                        os.link(backup, name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd, follow_symlinks=False)
                    except FileExistsError:
                        pass  # Never overwrite a concurrently created destination.
                raise
            finally:
                try:
                    os.unlink(temp_name, dir_fd=parent_fd)
                except FileNotFoundError:
                    pass

    def _save(self, plan: SyncPlan, entries: dict[str, dict[str, str]]) -> None:
        value = {"version": 1, "local_root": str(plan.local_root), "remote_root_uid": plan.remote_root_uid,
                 "remote_root": plan.remote_root, "entries": entries}
        fd, temporary = tempfile.mkstemp(prefix="baseline-", suffix=".tmp", dir=self.state_dir)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as stream:
                json.dump(value, stream, ensure_ascii=True, indent=2, sort_keys=True)
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self.state_dir / (plan._state_key + ".json"))
            with _open_absolute_directory(self.state_dir) as state_fd:
                os.fsync(state_fd)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def execute(self, plan: SyncPlan) -> SyncResult:
        """Execute an explicit preview once; every transfer is rechecked."""
        with self._lock:
            self._check_cancel()
            self._prepare_state()
            fd = os.open(self.state_dir / "sync.lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
            try:
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError as error:
                    raise SyncError("Another synchronization is already running") from error
                return self._execute_locked(plan)
            finally:
                os.close(fd)

    def _execute_locked(self, plan: SyncPlan) -> SyncResult:
        with self._lock:
            if plan._owner != self._owner or plan._executed:
                raise SyncError("Use a fresh preview from this sync engine")
            plan._executed = True
            self._prepare_state()
            root = self._root(plan.remote_root)
            if str(root.uid) != plan.remote_root_uid:
                raise SyncError("Remote root/account changed; create a new preview")
            _, digest = self._load(plan._state_key, plan.local_root, plan.remote_root_uid)
            if digest != plan._state_digest:
                raise SyncError("Another sync changed the baseline; create a new preview")
            result = SyncResult(conflicts=[str(item) for item in plan.conflicts], skipped=list(plan.skipped))
            expected_remote = dict(plan._remote)
            failed = set(plan._blocked)
            with _open_absolute_directory(plan.local_root) as root_fd:
                info = os.fstat(root_fd)
                if (info.st_dev, info.st_ino) != plan._root_identity:
                    raise SyncError("Local sync root was replaced after preview")
                for action in plan.actions:
                    self._check_cancel()
                    relative = action.relative_path
                    if _under(relative, failed):
                        result.conflicts.append(f"{relative}: parent operation was blocked")
                        continue
                    try:
                        expected_local = plan._local.get(relative)
                        with _parent(root_fd, relative) as (parent_fd, name):
                            if _local_entry(parent_fd, name, self._check_cancel) != expected_local:
                                raise SyncError("Local path changed after preview")
                        before_remote = expected_remote.get(relative)
                        if not _same_remote(before_remote, self._lookup_remote(root, relative)):
                            raise SyncError("Remote path changed after preview")
                        remote_parent = self._remote_parent(root, relative, expected_remote)
                        if action.kind == "mkdir_local":
                            with _parent(root_fd, relative) as (parent_fd, name):
                                os.mkdir(name, mode=0o700, dir_fd=parent_fd)
                        elif action.kind == "mkdir_remote":
                            node = self.backend.mkdir(remote_parent, PurePosixPath(relative).name)
                            entry = self._node_entry(node)
                            if entry.kind != "folder":
                                raise SyncError("Remote mkdir did not return a folder")
                            expected_remote[relative] = entry
                        elif action.kind == "upload":
                            if expected_local is None or expected_local.kind != "file":
                                raise SyncError("Invalid upload action")
                            with tempfile.TemporaryDirectory(prefix="upload-", dir=self.state_dir) as stage:
                                source = self._stage_upload(root_fd, relative, expected_local, Path(stage))
                                if not _same_remote(before_remote, self._lookup_remote(root, relative)):
                                    raise SyncError("Remote destination changed while preparing upload")
                                self.backend.upload([source], remote_parent,
                                                    file_strategy="create-new-revision" if before_remote else "skip",
                                                    folder_strategy="skip")
                            after = self._lookup_remote(root, relative)
                            if (after is None or after.kind != "file" or after.sha1 != expected_local.sha1
                                    or (before_remote is not None and after.uid != before_remote.uid)):
                                raise SyncError("Upload could not be verified; remote content was retained")
                            expected_remote[relative] = after
                        elif action.kind == "download":
                            if before_remote is None or before_remote.kind != "file":
                                raise SyncError("Invalid download action")
                            with tempfile.TemporaryDirectory(prefix="download-", dir=self.state_dir) as stage:
                                folder = Path(stage)
                                self.backend.download([before_remote.remote_path], folder,
                                                      file_strategy="skip", folder_strategy="skip")
                                with _open_absolute_directory(folder) as stage_fd:
                                    downloaded = _local_entry(stage_fd, PurePosixPath(relative).name, self._check_cancel)
                                if (downloaded is None or downloaded.kind != "file"
                                        or downloaded.sha1 != before_remote.sha1):
                                    raise SyncError("Downloaded content is missing or failed SHA-1 verification")
                                if not _same_remote(before_remote, self._lookup_remote(root, relative)):
                                    raise SyncError("Remote source changed while downloading")
                                self._install_download(root_fd, relative, folder / PurePosixPath(relative).name,
                                                       expected_local, result, plan.local_root)
                        else:
                            raise SyncError(f"Unknown sync action: {action.kind}")
                        result.completed.append(str(action))
                    except Cancelled:
                        raise
                    except SyncError as error:
                        failed.add(relative)
                        result.conflicts.append(f"{relative}: {error}")
                    except Exception as error:
                        failed.add(relative)
                        result.errors.append(f"{relative}: {error}")
                # A fresh end-of-operation snapshot proves which paths actually
                # converged. Failed/conflicting paths keep their old baseline.
                try:
                    root_now = self._root(plan.remote_root)
                    if str(root_now.uid) != plan.remote_root_uid:
                        raise SyncError("Remote root/account changed during sync")
                    local_now, local_blocked = self._scan_local(root_fd)
                    remote_now, remote_blocked = self._scan_remote(root_now)
                    blocked = failed | set(local_blocked) | set(remote_blocked)
                    for path, reason in {**local_blocked, **remote_blocked}.items():
                        message = f"{path}: {reason}"
                        if message not in result.skipped:
                            result.skipped.append(message)
                    for action in plan.actions:
                        relative = action.relative_path
                        if _under(relative, failed):
                            continue
                        left, right = local_now.get(relative), remote_now.get(relative)
                        if (_under(relative, blocked) or left is None or right is None
                                or left.kind != right.kind or (left.kind == "file" and left.sha1 != right.sha1)):
                            result.conflicts.append(f"{relative}: content changed during sync; convergence was not confirmed")
                            blocked.add(relative)
                    baseline = dict(plan._baseline)
                    for relative in sorted(local_now.keys() | remote_now.keys() | baseline.keys()):
                        if _under(relative, blocked):
                            continue
                        left, right = local_now.get(relative), remote_now.get(relative)
                        if left is None and right is None:
                            baseline.pop(relative, None)
                        elif left is not None and right is not None and left.kind == right.kind:
                            if left.kind == "folder" or left.sha1 == right.sha1:
                                baseline[relative] = {"kind": left.kind, "sha1": left.sha1,
                                                      "remote_uid": right.uid, "remote_revision": right.revision}
                    self._save(plan, baseline)
                except Cancelled:
                    raise
                except Exception as error:
                    result.errors.append(f"Baseline was not updated: {error}")
            return result
