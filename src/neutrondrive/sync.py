"""Explicit, conservative two-way sync using Proton's official CLI adapter.

There is deliberately no watcher or background remote tree polling here.
Baseline-backed file deletions use recoverable trash. Folder deletions remain
conflicts because the public SDK cannot enumerate directly trashed folders.
Uncertain identities,
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

from .backend import Cancelled, data_dir


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
    ctime_ns: int = field(default=0, compare=False)


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
    _baseline: dict[str, dict[str, Any]] = field(default_factory=dict, repr=False)
    _pending_deletions: dict[str, dict[str, str]] = field(default_factory=dict, repr=False)
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
        return f"{changes or 'No changes'}; conflicts: {len(self.conflicts)}; skipped: {len(self.skipped)}. Deletions use trash."


@dataclass
class SyncResult:
    completed: list[str] = field(default_factory=list)
    conflicts: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    backups: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)
    trashed: list[str] = field(default_factory=list)
    # End-of-run observations let the event-driven daemon ignore its own
    # filesystem writes without an additional content scan or lost events.
    _local_snapshot: dict[str, _Entry] = field(default_factory=dict, repr=False)
    _remote_snapshot: dict[str, _Entry] = field(default_factory=dict, repr=False)

    @property
    def ok(self) -> bool:
        return not self.errors and not self.conflicts

    def summary(self) -> str:
        return (f"Completed: {len(self.completed)}; conflicts: {len(self.conflicts)}; "
                f"errors: {len(self.errors)}; skipped: {len(self.skipped)}; trashed: {len(self.trashed)}; local backups: {len(self.backups)}.")


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
        return _Entry("folder", mtime_ns=info.st_mtime_ns, inode=info.st_ino,
                      device=info.st_dev, ctime_ns=info.st_ctime_ns)
    if not stat.S_ISREG(info.st_mode):
        raise SyncError("Symbolic links and non-regular files are excluded")
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=parent_fd)
    try:
        sha1, after = _read_file(fd, check_cancel=check_cancel)
        if (info.st_dev, info.st_ino) != (after.st_dev, after.st_ino):
            raise SyncError("Local file was replaced while being scanned")
        return _Entry("file", sha1, after.st_size, mtime_ns=after.st_mtime_ns,
                      inode=after.st_ino, device=after.st_dev, ctime_ns=after.st_ctime_ns)
    finally:
        os.close(fd)


def _kind(node: Any) -> str:
    value = str(node.type).lower()
    return "folder" if value in {"folder", "directory"} else value


def _remote_entry(node: Any, *, allow_trashed: bool = False) -> _Entry:
    if not getattr(node, "safe", True):
        raise SyncError("Remote metadata or decrypted name is not trustworthy")
    if getattr(node, "raw", {}).get("isTrashed") is True and not allow_trashed:
        raise SyncError("Trashed remote nodes are not active sync entries")
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


def _baseline_matches(entry: _Entry, previous: dict[str, Any], side: str, *, ancestor: bool = False) -> bool:
    if previous.get("identity_version") != 2 or entry.kind != previous.get("kind"):
        return False
    if entry.kind == "file" and (entry.sha1 != previous.get("sha1") or entry.size != previous.get("size")):
        return False
    if side == "local":
        return ((entry.device, entry.inode) == (previous.get("local_device"), previous.get("local_inode"))
                and ((entry.kind == "folder" and ancestor) or (entry.mtime_ns, entry.ctime_ns) == (previous.get("local_mtime_ns"), previous.get("local_ctime_ns"))))
    return (entry.uid == previous.get("remote_uid") and entry.revision == previous.get("remote_revision")
            and (entry.kind == "folder" or bool(entry.revision)))


def _subtree(entries: dict[str, Any], relative: str) -> dict[str, Any]:
    return {path: entry for path, entry in entries.items() if path == relative or path.startswith(relative + "/")}


def _require_file_deletion(previous: dict[str, Any]) -> None:
    if previous.get("kind") == "folder":
        raise SyncError("Folder deletion is blocked: the public Proton SDK cannot verify all children of a trashed folder. "
                        "Review this folder's contents manually. Individual files in existing folders remain supported.")


def _deletion_proof(relative: str, target: dict[str, _Entry], baseline: dict[str, dict[str, Any]],
                    blocked: set[str], side: str) -> None:
    previous, current = baseline.get(relative, {}), target.get(relative)
    _require_file_deletion(previous)
    if current is None or not _baseline_matches(current, previous, side):
        raise SyncError("Deletion conflicts with an edit, replacement, or unverified old baseline")
    expected, actual = _subtree(baseline, relative), _subtree(target, relative)
    if current.kind == "folder":
        if not previous.get("subtree_complete"):
            raise SyncError("Folder deletion lacks a complete verified subtree baseline")
        if any(path == relative or path.startswith(relative + "/") for path in blocked):
            raise SyncError("Folder contains unsupported, ignored, or unreadable children")
        if set(expected) != set(actual):
            raise SyncError("Folder contents changed since the deletion baseline")
        if any(not _baseline_matches(actual[path], entry, side) for path, entry in expected.items()):
            raise SyncError("Folder deletion conflicts with changed or replaced descendants")


class SyncEngine:
    """Plan first, then execute once after explicit confirmation in the UI."""

    def __init__(self, backend: Any, state_dir: Path, *, local_trash: Any = None):
        self.backend = backend
        self.state_dir = Path(os.path.abspath(state_dir.expanduser()))
        self._lock = threading.RLock()
        self._owner = uuid.uuid4().hex
        self.local_trash = local_trash

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
        if remote_path == "/my-files" or remote_path.startswith("/my-files/"):
            account = node if remote_path == "/my-files" else self.backend.info("/my-files")
            if _remote_entry(account).kind != "folder":
                raise SyncError("The account root is not safely accessible")
            current, visited = node, set()
            for _ in range(128):
                if current.uid == account.uid:
                    break
                if current.uid in visited:
                    raise SyncError("Remote root has a cyclic ancestor chain")
                visited.add(current.uid)
                parent = current.raw.get("parentUid")
                if not isinstance(parent, str) or not parent:
                    raise SyncError("Remote root ancestry cannot be verified")
                if parent == account.uid:
                    break
                current = self.backend.info("/my-files/" + parent)
                if _remote_entry(current).kind != "folder" or current.uid != parent:
                    raise SyncError("Remote ancestor disappeared or was replaced")
            else:
                raise SyncError("Remote root ancestry exceeds the safe maximum depth")
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

    def _node_entry(self, node: Any, *, allow_trashed: bool = False) -> _Entry:
        if _kind(node) == "file" and not getattr(node, "sha1", ""):
            node = self.backend.info(node.path)
        return _remote_entry(node, allow_trashed=allow_trashed)

    def _scan_remote(self, root: Any, *, fresh: bool = False, allow_trashed: bool = False) -> tuple[dict[str, _Entry], dict[str, str]]:
        entries: dict[str, _Entry] = {}
        blocked: dict[str, str] = {}
        visited = {str(root.uid)}

        def visit(path: str, prefix: str) -> None:
            self._check_cancel()
            grouped: dict[str, list[Any]] = {}
            listing = getattr(self.backend, "list_nodes_fresh", self.backend.list_nodes) if fresh else self.backend.list_nodes
            for node in listing(path):
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
                    entry = self._node_entry(node, allow_trashed=allow_trashed)
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

    def _load(self, key: str, root: Path, uid: str) -> tuple[dict[str, dict[str, Any]], str, dict, tuple | None]:
        path = self.state_dir / (key + ".json")
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except FileNotFoundError:
            return {}, "", {}, None
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_size > 32 * 1024 * 1024:
                raise SyncError("Invalid or oversized sync baseline")
            with os.fdopen(fd, "rb", closefd=False) as stream:
                raw = stream.read()
            value = json.loads(raw)
            if (value.get("version") not in {1, 2} or value.get("local_root") != str(root)
                    or value.get("remote_root_uid") != uid or not isinstance(value.get("entries"), dict)):
                raise SyncError("Sync baseline identity/schema does not match")
            entries = value["entries"]
            identity = value.get("local_root_identity")
            pending = value.get("pending_deletions", {})
            if value["version"] == 2:
                if (not isinstance(identity, list) or len(identity) != 2
                        or any(type(part) is not int or part < 0 for part in identity)
                        or not isinstance(pending, dict)):
                    raise SyncError("Invalid v2 baseline root identity or deletion journal")
                identity = tuple(identity)
            else:
                identity, pending = None, {}
            for relative, entry in entries.items():
                _parts(relative)
                if (not isinstance(entry, dict) or entry.get("kind") not in {"folder", "file"}
                        or (entry["kind"] == "file" and not _SHA1.fullmatch(entry.get("sha1", "")))):
                    raise SyncError("Invalid sync baseline entry")
                if value["version"] == 1:
                    entry["identity_version"] = 1
                if entry.get("identity_version") == 2:
                    if (any(type(entry.get(key)) is not int or entry[key] < 0
                            for key in ("size", "local_device", "local_inode", "local_mtime_ns", "local_ctime_ns"))
                            or not isinstance(entry.get("remote_uid"), str) or not entry["remote_uid"]
                            or not isinstance(entry.get("remote_revision"), str)
                            or not isinstance(entry.get("remote_path"), str)
                            or not isinstance(entry.get("subtree_complete", False), bool)):
                        raise SyncError("Invalid v2 sync baseline identity")
                elif entry.get("identity_version") != 1:
                    raise SyncError("Unknown baseline entry identity version")
            for relative, record in pending.items():
                _parts(relative)
                if (relative not in entries or not isinstance(record, dict)
                        or record.get("direction") not in {"local", "remote"}
                        or not re.fullmatch(r"[0-9a-f]{32}", record.get("operation_id", ""))):
                    raise SyncError("Invalid pending deletion journal")
            return entries, hashlib.sha256(raw).hexdigest(), pending, identity
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
            baseline, digest, pending, saved_identity = self._load(key, root, str(remote.uid))
            with _open_absolute_directory(root) as fd:
                info = os.fstat(fd)
                if saved_identity is not None and saved_identity != (info.st_dev, info.st_ino):
                    raise SyncError("Local sync root identity changed since the saved baseline")
                local, local_blocked = self._scan_local(fd)
            cloud, cloud_blocked = self._scan_remote(remote)
            blocked = {**local_blocked, **cloud_blocked}
            plan = SyncPlan(root, str(remote.path), str(remote.uid), _local=local, _remote=cloud,
                            _baseline=baseline, _pending_deletions=pending, _state_digest=digest, _state_key=key,
                            _root_identity=(info.st_dev, info.st_ino), _blocked=set(blocked), _owner=self._owner)
            plan.skipped.extend(f"{path}: {reason}" for path, reason in sorted(blocked.items()))
            # A rename/move inside either scanned tree is not deletion proof.
            # Block both its old and new locations to avoid a trash+copy cascade.
            for old, previous in baseline.items():
                moved = []
                if old not in local and previous.get("identity_version") == 2:
                    moved.extend(path for path, entry in local.items()
                                 if (entry.device, entry.inode) == (previous.get("local_device"), previous.get("local_inode")))
                if old not in cloud and previous.get("remote_uid"):
                    moved.extend(path for path, entry in cloud.items() if entry.uid == previous["remote_uid"])
                if moved:
                    plan.conflicts.append(SyncConflict(old, "A baseline identity moved to another path; resolve the move first"))
                    plan._blocked.update([old, *moved])
            covered_deletions = set()
            for relative in sorted(local.keys() | cloud.keys() | baseline.keys(), key=lambda x: (x.count("/"), x)):
                if _under(relative, plan._blocked | covered_deletions):
                    continue
                left, right, previous = local.get(relative), cloud.get(relative), baseline.get(relative)
                reason = ""
                if left is None and right is None:
                    continue
                pending_root = next((path for path in pending if relative == path or relative.startswith(path + "/")), None)
                if pending_root and ((left is not None and right is not None)
                                     or pending[pending_root]["direction"] != ("local" if left is not None else "remote")):
                    reason = "Earlier trash operation has an uncertain outcome; retained deletion intent requires review"
                elif left is not None and right is not None:
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
                    try:
                        side = "local" if left is not None else "remote"
                        self._check_ancestor_baselines(relative, local, cloud, baseline)
                        _deletion_proof(relative, local if left is not None else cloud, baseline, set(blocked), side)
                        if left is not None:
                            self._prove_remote_deletion(plan, remote, relative)
                        plan.actions.append(SyncAction("trash_local" if left is not None else "trash_remote", relative,
                                                       "Propagate verified deletion to recoverable trash"))
                        covered_deletions.add(relative)
                    except SyncError as error:
                        reason = str(error)
                elif left is not None:
                    plan.actions.append(SyncAction("mkdir_remote" if left.kind == "folder" else "upload", relative))
                elif right is not None:
                    plan.actions.append(SyncAction("mkdir_local" if right.kind == "folder" else "download", relative))
                if reason:
                    plan.conflicts.append(SyncConflict(relative, reason))
                    plan._blocked.add(relative)
            return plan

    def _check_ancestor_baselines(self, relative: str, local: dict[str, _Entry], cloud: dict[str, _Entry],
                                  baseline: dict[str, dict[str, Any]]) -> None:
        parts = _parts(relative)
        for index in range(1, len(parts)):
            parent = "/".join(parts[:index])
            previous = baseline.get(parent, {})
            left, right = local.get(parent), cloud.get(parent)
            if (left is None or right is None or left.kind != "folder" or right.kind != "folder"
                    or not _baseline_matches(left, previous, "local", ancestor=True)
                    or not _baseline_matches(right, previous, "remote")):
                raise SyncError("Deletion ancestor disappeared, moved, or was replaced")

    def _assert_roots(self, plan: SyncPlan, root_fd: int) -> Any:
        with _open_absolute_directory(plan.local_root) as current_fd:
            current, opened = os.fstat(current_fd), os.fstat(root_fd)
            if ((current.st_dev, current.st_ino) != plan._root_identity
                    or (opened.st_dev, opened.st_ino) != plan._root_identity):
                raise SyncError("Local root was moved or replaced during synchronization")
        root = self._root(plan.remote_root)
        if str(root.uid) != plan.remote_root_uid:
            raise SyncError("Remote root/account changed during synchronization")
        return root

    def _prove_remote_deletion(self, plan: SyncPlan, root: Any, relative: str) -> None:
        _require_file_deletion(plan._baseline.get(relative, {}))
        self._assert_remote_ancestors(plan, root, relative)
        if self._lookup_remote(root, relative) is not None:
            raise SyncError("Remote deletion source reappeared")
        expected = _subtree(plan._baseline, relative)
        deleted_root = None
        for path, previous in expected.items():
            try:
                node = self.backend.info(previous["remote_path"])
            except Cancelled:
                raise
            except Exception as error:
                raise SyncError("Remote disappearance is not confirmed trash; it may be a move or unavailable metadata") from error
            entry = _remote_entry(node, allow_trashed=True)
            if not _baseline_matches(entry, previous, "remote"):
                raise SyncError("Remote deleted identity was edited or replaced")
            if path == relative:
                if node.raw.get("isTrashed") is not True:
                    raise SyncError("Remote UID is still active elsewhere; a move is not deletion proof")
                parent = str(PurePosixPath(path).parent)
                parent_uid = (plan.remote_root_uid if parent == "."
                              else plan._baseline.get(parent, {}).get("remote_uid"))
                if not parent_uid or node.raw.get("parentUid") != parent_uid:
                    raise SyncError("Remote deleted identity moved to a different parent before trash")
                deleted_root = node
            else:
                parent = str(PurePosixPath(path).parent)
                if node.raw.get("parentUid") != expected[parent]["remote_uid"]:
                    raise SyncError("A descendant of the deleted folder moved elsewhere")
        if deleted_root is not None and expected[relative]["kind"] == "folder":
            children, blocked = self._scan_remote(deleted_root, fresh=True, allow_trashed=True)
            actual = {relative + "/" + path: entry for path, entry in children.items()}
            if (blocked or set(actual) != set(expected) - {relative}
                    or any(not _baseline_matches(entry, expected[path], "remote") for path, entry in actual.items())):
                raise SyncError("Deleted remote folder changed or contains unverified children; inspect the recoverable trash")
        self._assert_remote_ancestors(plan, root, relative)

    def _assert_remote_ancestors(self, plan: SyncPlan, root: Any, relative: str) -> None:
        """A stable parent UID alone cannot prove its original location."""
        parts = _parts(relative)
        for index in range(1, len(parts)):
            parent = "/".join(parts[:index])
            current = self._lookup_remote(root, parent)
            if (current is None or current.kind != "folder"
                    or not _baseline_matches(current, plan._baseline.get(parent, {}), "remote")):
                raise SyncError("Remote deletion ancestor disappeared, moved, or was replaced")

    def _deletion_preflight(self, plan: SyncPlan, action: SyncAction, root_fd: int) -> dict[str, _Entry]:
        """Fresh identities, absence, ancestors and complete target subtree."""
        _require_file_deletion(plan._baseline.get(action.relative_path, {}))
        root = self._assert_roots(plan, root_fd)
        local, local_blocked = self._scan_local(root_fd)
        relative = action.relative_path
        if _under(relative, set(local_blocked)):
            raise SyncError("Local absence or target cannot be proved through an unreadable ancestor")
        cloud_ancestors = {}
        parts = _parts(relative)
        for index in range(1, len(parts)):
            parent = "/".join(parts[:index])
            entry = self._lookup_remote(root, parent)
            if entry is not None:
                cloud_ancestors[parent] = entry
        self._check_ancestor_baselines(relative, local, cloud_ancestors, plan._baseline)
        if action.kind == "trash_local":
            self._prove_remote_deletion(plan, root, relative)
            _deletion_proof(relative, local, plan._baseline, set(local_blocked), "local")
            self._assert_roots(plan, root_fd)
            self._assert_remote_ancestors(plan, root, relative)
            return _subtree(local, relative)
        if local.get(relative) is not None:
            raise SyncError("Locally deleted path reappeared")
        previous_tree = _subtree(plan._baseline, relative)
        previous_ids = {(entry["local_device"], entry["local_inode"]) for entry in previous_tree.values()}
        if any((entry.device, entry.inode) in previous_ids for entry in local.values()):
            raise SyncError("Locally deleted identity moved elsewhere in the sync tree")
        current = self._lookup_remote(root, relative)
        if current is None:
            raise SyncError("Remote trash target disappeared after preview")
        # UID info catches revision and parent changes after the fresh listing.
        # A moved UID can still resolve normally outside this synchronization.
        node = self.backend.info(current.remote_path)
        parent = str(PurePosixPath(relative).parent)
        parent_uid = str(root.uid) if parent == "." else cloud_ancestors[parent].uid
        if node.raw.get("parentUid") != parent_uid:
            raise SyncError("Remote trash target moved to a different parent after preview")
        current = self._node_entry(node)
        target, blocked = {relative: current}, set()
        if current.kind == "folder":
            children, problems = self._scan_remote(node, fresh=True)
            target.update({relative + "/" + path: entry for path, entry in children.items()})
            blocked = {relative + "/" + path for path in problems}
        _deletion_proof(relative, target, plan._baseline, blocked, "remote")
        self._assert_roots(plan, root_fd)
        self._assert_remote_ancestors(plan, root, relative)
        return target

    def _trash_store(self):
        if self.local_trash is None:
            from .local_trash import LocalTrash
            self.local_trash = LocalTrash(data_dir() / "local-trash")
        return self.local_trash

    def _lookup_remote(self, root: Any, relative: str) -> _Entry | None:
        current = str(root.path)
        components = _parts(relative)
        for index, name in enumerate(components):
            self._check_cancel()
            # Background metadata caches must never weaken transfer preflight
            # checks. A cached adapter explicitly supplies an uncached listing.
            listing = getattr(self.backend, "list_nodes_fresh", self.backend.list_nodes)
            matches = [item for item in listing(current) if item.name == name]
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

    def _save(self, plan: SyncPlan, entries: dict[str, dict[str, Any]], pending: dict | None = None) -> None:
        value = {"version": 2, "local_root": str(plan.local_root), "remote_root_uid": plan.remote_root_uid,
                 "remote_root": plan.remote_root, "local_root_identity": list(plan._root_identity),
                 "entries": entries, "pending_deletions": plan._pending_deletions if pending is None else pending}
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
            _, digest, _, _ = self._load(plan._state_key, plan.local_root, plan.remote_root_uid)
            if digest != plan._state_digest:
                raise SyncError("Another sync changed the baseline; create a new preview")
            result = SyncResult(conflicts=[str(item) for item in plan.conflicts], skipped=list(plan.skipped))
            expected_remote = dict(plan._remote)
            failed = set(plan._blocked)
            baseline = dict(plan._baseline)
            pending = dict(plan._pending_deletions)
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
                        if action.kind in {"trash_local", "trash_remote"}:
                            target = self._deletion_preflight(plan, action, root_fd)
                            operation = {"direction": "local" if action.kind == "trash_local" else "remote",
                                         "operation_id": uuid.uuid4().hex}
                            pending[relative] = operation
                            # Durable intent precedes either mutation. A crash
                            # cannot turn this deletion into a new upload.
                            self._save(plan, baseline, pending)
                            if action.kind == "trash_remote":
                                target = self._deletion_preflight(plan, action, root_fd)
                                self.backend.trash([target[relative].remote_path])
                            else:
                                receipt = self._trash_store().trash(
                                    root_fd, relative, target[relative], root_path=plan.local_root,
                                    expected_tree=target if target[relative].kind == "folder" else None,
                                    validate=lambda a=action: self._deletion_preflight(plan, a, root_fd),
                                    operation_id=operation["operation_id"], check_cancel=self._check_cancel)
                                if receipt.status != "trashed" or receipt.original_path != str(plan.local_root / relative):
                                    raise SyncError("Local trash did not return a verified receipt")
                            current_root = self._assert_roots(plan, root_fd)
                            local_after, blocked_after = self._scan_local(root_fd)
                            if relative in local_after or _under(relative, set(blocked_after)):
                                raise SyncError("Local path reappeared or became unreadable while deletion was propagated")
                            self._prove_remote_deletion(plan, current_root, relative)
                            result.trashed.append(relative)
                            result.completed.append(str(action))
                            continue
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
                    root_now = self._assert_roots(plan, root_fd)
                    local_now, local_blocked = self._scan_local(root_fd)
                    remote_now, remote_blocked = self._scan_remote(root_now)
                    result._local_snapshot = local_now
                    result._remote_snapshot = remote_now
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
                        if action.kind in {"trash_local", "trash_remote"} and left is None and right is None:
                            continue
                        if (_under(relative, blocked) or left is None or right is None
                                or left.kind != right.kind or (left.kind == "file" and left.sha1 != right.sha1)):
                            result.conflicts.append(f"{relative}: content changed during sync; convergence was not confirmed")
                            blocked.add(relative)
                    for relative, operation in list(pending.items()):
                        if _under(relative, blocked) or relative in local_now or relative in remote_now:
                            continue
                        try:
                            self._prove_remote_deletion(plan, root_now, relative)
                            if operation["direction"] == "local":
                                receipt = self._trash_store().get_receipt(operation["operation_id"])
                                if (receipt is None or receipt.status != "trashed"
                                        or receipt.original_path != str(plan.local_root / relative)):
                                    raise SyncError("Uncertain local trash receipt requires recovery review")
                            pending.pop(relative)
                            for path in list(_subtree(baseline, relative)):
                                baseline.pop(path)
                            if relative not in result.trashed:
                                result.trashed.append(relative)
                        except Cancelled:
                            raise
                        except Exception as error:
                            blocked.add(relative)
                            result.conflicts.append(f"{relative}: {error}")
                    for relative in sorted(local_now.keys() | remote_now.keys() | baseline.keys()):
                        if _under(relative, blocked | set(pending)):
                            continue
                        left, right = local_now.get(relative), remote_now.get(relative)
                        if left is None and right is None:
                            if self._lookup_remote(root_now, relative) is None:
                                baseline.pop(relative, None)
                        elif left is not None and right is not None and left.kind == right.kind:
                            if left.kind == "folder" or left.sha1 == right.sha1:
                                descendants = _subtree(local_now, relative)
                                if left.kind == "folder" and any(path == relative or path.startswith(relative + "/") for path in blocked | set(pending)):
                                    continue
                                complete = (set(descendants) == set(_subtree(remote_now, relative))
                                            and all(entry.kind == remote_now[path].kind
                                                    and (entry.kind == "folder" or entry.sha1 == remote_now[path].sha1)
                                                    for path, entry in descendants.items()))
                                baseline[relative] = {"identity_version": 2, "kind": left.kind, "sha1": left.sha1,
                                                      "size": left.size, "local_device": left.device, "local_inode": left.inode,
                                                      "local_mtime_ns": left.mtime_ns, "local_ctime_ns": left.ctime_ns,
                                                      "remote_uid": right.uid, "remote_revision": right.revision,
                                                      "remote_path": right.remote_path, "subtree_complete": complete}
                    self._save(plan, baseline, pending)
                except Cancelled:
                    raise
                except Exception as error:
                    result.errors.append(f"Baseline was not updated: {error}")
            return result
