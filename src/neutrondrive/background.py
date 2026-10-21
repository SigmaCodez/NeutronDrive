"""Event-driven, conservative background synchronization for Linux.

The daemon performs one reconciliation when a job starts, then only in response
to inotify, real remote events, explicit commands, or retries of failed work.
There is no periodic tree scan. Configuration and IPC are private to this user;
the existing SyncEngine retains conflict/deletion and version-backup safeguards.
"""
from __future__ import annotations

from contextlib import contextmanager
import ctypes
from dataclasses import asdict, dataclass, field, replace
import fcntl
import json
import os
from pathlib import Path
import random
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Callable, Iterator
import uuid

from .backend import Backend, BackendError, Cancelled
from . import rust_core
from .sync import SyncEngine, SyncError, _open_absolute_directory


class BackgroundError(BackendError):
    """A safe, user-readable daemon/configuration failure."""


class _Blocked(BackgroundError):
    """Requires a deliberate job/configuration change, not timed retries."""


class RemoteTreeCache:
    """Directory metadata only; identity/revision checks always reach Proton.

    Initial traversal populates memory. Events invalidate affected old/new
    parents; writes invalidate their destination. No TTL, background traversal,
    persisted account metadata, or cached ``info`` calls are used.
    """

    def __init__(self, backend: Any):
        self.backend = backend
        self._lock = threading.RLock()
        self._listings: dict[str, tuple[Any, ...]] = {}
        self._uid_paths: dict[str, str] = {}
        self._parents: dict[str, set[str]] = {}
        self._children: dict[str, set[str]] = {}
        self._aliases: dict[str, str] = {}
        self._generation = 0

    def __getattr__(self, name):
        return getattr(self.backend, name)

    def _path(self, path: str) -> str:
        return self._aliases.get(path, path)

    def info(self, path: str):
        node = self.backend.info(path)
        with self._lock:
            self._uid_paths[node.uid] = node.path
            self._aliases[path] = node.path
            if node.raw.get("isTrashed"):
                self.invalidate_tree(node.path)
                for parent in self._parents.get(node.uid, ()):
                    self.invalidate(parent)
                return node
            for parent in self._parents.get(node.uid, ()):
                listing = self._listings.get(parent)
                if listing is not None:
                    self._listings[parent] = tuple(node if entry.uid == node.uid else entry for entry in listing)
        return node

    def list_nodes(self, path: str):
        with self._lock:
            cached = self._listings.get(self._path(path))
            if cached is not None:
                return list(cached)
        return self.list_nodes_fresh(path)

    def list_nodes_fresh(self, path: str):
        with self._lock:
            path = self._path(path)
            generation = self._generation
        nodes = self.backend.list_nodes(path)
        with self._lock:
            if generation == self._generation:
                self._listings[path] = tuple(nodes)
                self._children[path] = {node.path for node in nodes if node.type == "folder"}
                for node in nodes:
                    self._uid_paths[node.uid] = node.path
                    self._parents.setdefault(node.uid, set()).add(path)
        return list(nodes)

    def invalidate(self, path: str) -> None:
        with self._lock:
            self._generation += 1
            self._listings.pop(self._path(path), None)

    def _invalidate_plan(self, roots, *, node_uid=None, parent_uid=None, reset=True):
        # Caller holds the lock throughout observation, decision and application.
        # A failed core call must not leave stale metadata available for a retry.
        self._generation += 1
        try:
            paths = rust_core.call("cache_invalidate", snapshot={
                "roots": roots, "children": self._children, "aliases": self._aliases,
                "parents": self._parents if not reset else {},
                "uid_paths": self._uid_paths if not reset else {},
                "node_uid": node_uid, "parent_uid": parent_uid, "reset": reset,
            })
        except rust_core.CoreError:
            self._listings.clear()
            raise
        for path in paths:
            self._listings.pop(path, None)

    def invalidate_tree(self, path: str) -> None:
        with self._lock:
            self._invalidate_plan([path])

    def event(self, node_uid: str | None, parent_uid: str | None, roots: list[str], *, reset=False) -> None:
        with self._lock:
            self._invalidate_plan(roots, node_uid=node_uid, parent_uid=parent_uid, reset=reset)

    def upload(self, paths, parent, **options):
        try:
            return self.backend.upload(paths, parent, **options)
        finally:
            self.invalidate(parent)

    def mkdir(self, parent, name):
        try:
            return self.backend.mkdir(parent, name)
        finally:
            self.invalidate(parent)

    def trash(self, paths):
        # Keep old parent relationships before the mutation. They also cover
        # ambiguous outcomes: an exception may follow a successful server write.
        with self._lock:
            targets = {self._path(path) for path in paths}
            known_targets = {path for uid, path in self._uid_paths.items()
                             if path in targets and self._parents.get(uid)}
            parents = {parent for uid, path in self._uid_paths.items() if path in targets
                       for parent in self._parents.get(uid, ())}
        try:
            return self.backend.trash(paths)
        finally:
            with self._lock:
                for target in targets:
                    self.invalidate_tree(target)
                if targets <= known_targets:
                    for parent in parents:
                        self.invalidate(parent)
                else:
                    # An uncached UID cannot be assigned a safe old parent.
                    # Drop metadata only; the next event/job reads it afresh.
                    self._generation += 1
                    self._listings.clear()


def _private_directory(path: Path) -> Path:
    path = Path(os.path.abspath(Path(path).expanduser()))
    try:
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        with _open_absolute_directory(path) as fd:
            info = os.fstat(fd)
            if info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise BackgroundError("Das Dienstverzeichnis muss Ihnen gehören und Zugriffsmodus 0700 haben.")
    except (OSError, SyncError) as error:
        raise BackgroundError("Dienstverzeichnis ist nicht sicher zugänglich; Symlinks sind nicht erlaubt.") from error
    return path


def _read_json(path: Path, default: Any) -> Any:
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except FileNotFoundError:
        return default
    except OSError as error:
        raise BackgroundError("Private Dienstdatei ist nicht sicher lesbar.") from error
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_size > 8 * 1024 * 1024:
            raise BackgroundError("Ungültige private Dienstdatei.")
        with os.fdopen(fd, "r", encoding="utf-8", closefd=False) as stream:
            return json.load(stream)
    except (ValueError, UnicodeError) as error:
        raise BackgroundError("Eine Dienstdatei ist beschädigt; sie wird nicht überschrieben.") from error
    finally:
        os.close(fd)


def _write_json(path: Path, value: Any) -> None:
    fd, temporary = tempfile.mkstemp(prefix=".state-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=True, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def _overlap(left: Path, right: Path) -> bool:
    return left == right or left in right.parents or right in left.parents


def _valid_remote(path: str) -> bool:
    return (isinstance(path, str) and (path == "/my-files" or path.startswith("/my-files/"))
            and "\x00" not in path and not any(part in {".", "..", ""} for part in path[1:].split("/")))


def _mount_paths() -> list[tuple[Path, str]]:
    """Read mount metadata only, never traverse a potentially lazy mount."""
    result = []
    try:
        lines = Path("/proc/self/mountinfo").read_text().splitlines()
    except OSError as error:
        raise _Blocked("Linux-Mountinformationen sind nicht verfügbar.") from error
    for line in lines:
        left, separator, right = line.partition(" - ")
        fields, tail = left.split(), right.split()
        if not separator or len(fields) < 5 or not tail:
            continue
        path = fields[4]
        for escaped, literal in (("\\040", " "), ("\\011", "\t"), ("\\012", "\n"), ("\\134", "\\")):
            path = path.replace(escaped, literal)
        result.append((Path(path), tail[0]))
    return result


def _validate_local(root: Path, state_dir: Path, mount_paths: list[Path] = ()) -> tuple[int, int]:
    if _overlap(root, state_dir):
        raise _Blocked("Sync-Ordner und privates Dienstverzeichnis dürfen sich nicht überschneiden.")
    for mounted in mount_paths:
        if _overlap(root, Path(mounted)):
            raise _Blocked("Ein On-demand-Mount darf kein Hintergrund-Sync-Ordner sein oder darin liegen.")
    for mounted, kind in _mount_paths():
        if kind == "autofs" or kind.startswith("fuse"):
            if _overlap(root, mounted):
                raise _Blocked("FUSE-/On-demand-/Autofs-Dateisysteme sind vom Hintergrund-Sync ausgeschlossen.")
        elif root in mounted.parents:
            raise _Blocked("Der Sync-Ordner enthält einen weiteren Mount; bitte einen einzelnen Ordner wählen.")
    try:
        with _open_absolute_directory(root) as fd:
            info = os.fstat(fd)
            return info.st_dev, info.st_ino
    except (OSError, SyncError) as error:
        raise _Blocked("Lokaler Sync-Ordner ist nicht sicher zugänglich; Symlinks sind nicht erlaubt.") from error


@dataclass(frozen=True)
class Job:
    id: str
    name: str
    local_root: str
    remote_root: str
    paused: bool = False
    remote_uid: str = ""
    account_root_uid: str = ""
    local_device: int = 0
    local_inode: int = 0


class JobStore:
    """Process-safe persisted job configuration; roots are immutable after add."""

    def __init__(self, state_dir: Path, *, mount_paths: list[Path] = ()):
        self.state_dir = _private_directory(Path(state_dir))
        self.mount_paths = tuple(Path(path) for path in mount_paths)
        self.path = self.state_dir / "jobs.json"
        self._lock = threading.RLock()

    @contextmanager
    def _locked(self) -> Iterator[None]:
        with self._lock:
            fd = os.open(self.state_dir / "jobs.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX)
                yield
            finally:
                os.close(fd)

    def _load(self) -> list[Job]:
        value = _read_json(self.path, {"version": 1, "jobs": []})
        if not isinstance(value, dict) or value.get("version") != 1 or not isinstance(value.get("jobs"), list):
            raise BackgroundError("Ungültige Version der Sync-Konfiguration.")
        jobs = []
        try:
            for row in value["jobs"]:
                job = Job(**row)
                if (len(job.id) != 32 or any(char not in "0123456789abcdef" for char in job.id)
                        or not isinstance(job.paused, bool) or not isinstance(job.name, str)
                        or not Path(job.local_root).is_absolute() or not _valid_remote(job.remote_root)
                        or not isinstance(job.local_device, int) or not isinstance(job.local_inode, int)
                        or not isinstance(job.remote_uid, str) or not isinstance(job.account_root_uid, str)
                        or any(part in {".", ".."} for part in job.local_root.split("/"))):
                    raise ValueError("invalid fields")
                jobs.append(job)
        except (TypeError, ValueError, AttributeError) as error:
            raise BackgroundError("Ungültige Sync-Jobkonfiguration; keine automatische Reparatur.") from error
        if len(jobs) > 64 or len({job.id for job in jobs}) != len(jobs):
            raise BackgroundError("Zu viele oder doppelte Sync-Jobs.")
        for index, job in enumerate(jobs):
            for other in jobs[index + 1:]:
                if _overlap(Path(job.local_root), Path(other.local_root)):
                    raise BackgroundError("Lokale Sync-Ordner dürfen sich nicht überschneiden.")
                a, b = job.remote_root, other.remote_root
                if a == b or a.startswith(b + "/") or b.startswith(a + "/"):
                    raise BackgroundError("Remote-Sync-Ordner dürfen sich nicht überschneiden.")
        return jobs

    def _save(self, jobs: list[Job]) -> None:
        _write_json(self.path, {"version": 1, "jobs": [asdict(job) for job in jobs]})

    def list(self) -> list[Job]:
        with self._locked():
            return self._load()

    def add(self, local_root: Path, remote_root: str, name: str = "", *, paused: bool = False) -> Job:
        root = Path(os.path.abspath(Path(local_root).expanduser()))
        if remote_root != "/my-files" and not remote_root.startswith("/my-files/"):
            raise BackgroundError("Automatischer Sync unterstützt derzeit nur Meine Dateien und dessen Unterordner.")
        if not _valid_remote(remote_root):
            raise BackgroundError("Ungültiger Remote-Sync-Pfad.")
        device, inode = _validate_local(root, self.state_dir, self.mount_paths)
        title = (name.strip() or root.name or "Sync")[:120]
        with self._locked():
            jobs = self._load()
            if len(jobs) >= 64:
                raise BackgroundError("Es sind höchstens 64 Sync-Jobs möglich.")
            for other in jobs:
                if _overlap(root, Path(other.local_root)):
                    raise BackgroundError("Lokale Sync-Ordner dürfen sich nicht überschneiden.")
                a, b = remote_root.rstrip("/"), other.remote_root.rstrip("/")
                if a == b or a.startswith(b + "/") or b.startswith(a + "/"):
                    raise BackgroundError("Remote-Sync-Ordner dürfen sich nicht überschneiden.")
            job = Job(uuid.uuid4().hex, title, str(root), remote_root, bool(paused),
                      local_device=device, local_inode=inode)
            self._save(jobs + [job])
            return job

    def update(self, job_id: str, *, paused: bool | None = None, name: str | None = None) -> Job:
        with self._locked():
            jobs = self._load()
            for index, job in enumerate(jobs):
                if job.id == job_id:
                    values = {}
                    if paused is not None:
                        if not isinstance(paused, bool):
                            raise BackgroundError("paused muss ein Wahrheitswert sein.")
                        values["paused"] = paused
                    if name is not None:
                        values["name"] = name.strip()[:120] or job.name
                    updated = replace(job, **values)
                    jobs[index] = updated
                    self._save(jobs)
                    return updated
        raise BackgroundError("Sync-Job wurde nicht gefunden.")

    def remove(self, job_id: str) -> None:
        with self._locked():
            jobs = self._load()
            remaining = [job for job in jobs if job.id != job_id]
            if len(remaining) == len(jobs):
                raise BackgroundError("Sync-Job wurde nicht gefunden.")
            self._save(remaining)

    def _pin(self, job_id: str, uid: str, account_uid: str, remote_path: str) -> Job:
        with self._locked():
            jobs = self._load()
            for index, job in enumerate(jobs):
                if job.id != job_id:
                    continue
                if (job.remote_uid and job.remote_uid != uid) or (job.account_root_uid and job.account_root_uid != account_uid):
                    raise _Blocked("Proton-Konto oder Remote-Wurzel wurde geändert; Job bewusst neu anlegen.")
                updated = replace(job, remote_uid=uid, account_root_uid=account_uid, remote_root=remote_path)
                if updated != job:
                    jobs[index] = updated
                    self._save(jobs)
                return updated
        raise _Blocked("Sync-Job wurde während der Vorbereitung entfernt.")


class InotifyWatcher:
    """Recursive Linux watcher; registering directories never hashes contents."""

    _MASK = 0x00000008 | 0x00000004 | 0x00000040 | 0x00000080 | 0x00000100 | 0x00000200 | 0x00000400 | 0x00000800
    _DIRECTORY = 0x40000000
    _OVERFLOW = 0x00004000
    _IGNORED = 0x00008000
    _NOFOLLOW = 0x02000000
    _ONLYDIR = 0x01000000

    def __init__(self, root: Path, callback: Callable[[str | None, str], None]):
        self.root = Path(root)
        self.callback = callback
        self._libc = ctypes.CDLL(None, use_errno=True)
        self._libc.inotify_init1.argtypes = [ctypes.c_int]
        self._libc.inotify_init1.restype = ctypes.c_int
        self._libc.inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
        self._libc.inotify_add_watch.restype = ctypes.c_int
        self._fd = self._libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
        if self._fd < 0:
            raise BackgroundError("inotify konnte nicht gestartet werden.")
        self._watches: dict[int, Path] = {}
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        try:
            self._add_tree(self.root)
        except BaseException:
            os.close(self._fd)
            raise

    def _add_tree(self, root: Path) -> None:
        mounts = _mount_paths()
        if any(_overlap(root, path) and (kind.startswith("fuse") or kind == "autofs")
               for path, kind in mounts):
            raise _Blocked("Ein FUSE-/On-demand-Mount wurde im überwachten Baum gefunden.")
        nested_mounts = {path for path, _ in mounts if self.root in path.parents}
        if root.is_symlink() or not root.is_dir():
            return
        if any(root == mounted or mounted in root.parents for mounted in nested_mounts):
            raise _Blocked("Ein neuer Mount liegt im überwachten Baum.")
        for directory, names, _ in os.walk(root, followlinks=False):
            names[:] = [name for name in names if not name.startswith(".neutrondrive-")
                        and Path(directory) / name not in nested_mounts
                        and not (Path(directory) / name).is_symlink()]
            watch = self._libc.inotify_add_watch(self._fd, os.fsencode(directory), self._MASK | self._NOFOLLOW | self._ONLYDIR)
            if watch < 0:
                raise BackgroundError("inotify-Überwachung fehlgeschlagen; Watch-Limit oder Ordnerrechte prüfen.")
            self._watches[watch] = Path(directory)

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, name="neutrondrive-inotify", daemon=True)
        self._thread.start()

    def _consume(self, data: bytes) -> None:
        for event in rust_core.call("inotify_decode", data=list(data)):
            if event["kind"] == "overflow":
                self.callback(None, "inotify-overflow")
                continue
            watch = event["watch"]
            parent = self._watches.get(watch)
            if parent is None:
                continue
            if event["kind"] == "ignored":
                self._watches.pop(watch, None)
                continue
            name = os.fsdecode(bytes(event["name"]))
            path = parent / name if name else parent
            if event["add_tree"]:
                try:
                    self._add_tree(path)
                except (OSError, BackgroundError):
                    self.callback(None, "watch-error")
            if event["rebuild"]:
                self.callback(None, "watch-rebuild")
            try:
                relative = path.relative_to(self.root).as_posix()
            except ValueError:
                self.callback(None, "watch-rebuild")
            else:
                self.callback(None if relative == "." else relative, "local")

    def _run(self) -> None:
        try:
            while not self._stop.is_set():
                if select.select([self._fd], [], [], 0.5)[0]:
                    try:
                        self._consume(os.read(self._fd, 1024 * 1024))
                    except BlockingIOError:
                        pass
        except (OSError, ValueError, rust_core.CoreError):
            if not self._stop.is_set():
                self.callback(None, "watch-error")

    def stop(self) -> None:
        self._stop.set()
        if self._fd >= 0:
            fd, self._fd = self._fd, -1
            os.close(fd)

    def join(self) -> None:
        if self._thread and self._thread is not threading.current_thread():
            self._thread.join(timeout=1.0)


def _matches_snapshot(root: Path, relative: str, snapshot: dict[str, Any]) -> bool:
    """Cheap stat-only check for an inotify echo; never follows symlinks."""
    path = root / relative
    try:
        with _open_absolute_directory(root):
            pass
    except (OSError, SyncError):
        return False
    for mounted, kind in _mount_paths():
        if (kind == "autofs" or kind.startswith("fuse")) and _overlap(path, mounted):
            return False
        if root in mounted.parents and _overlap(path, mounted):
            return False
    if path.is_absolute() and not path.is_relative_to(root):
        return False
    if any(part in {".", ".."} for part in relative.split("/")):
        return False
    if any(parent.is_symlink() for parent in reversed(path.parents) if parent != root and root in parent.parents):
        return False
    try:
        info = path.lstat()
    except FileNotFoundError:
        return relative not in snapshot
    except OSError:
        return False
    expected = snapshot.get(relative)
    if expected is None or stat.S_ISLNK(info.st_mode):
        return False
    if expected.kind == "folder":
        if not stat.S_ISDIR(info.st_mode) or (info.st_dev, info.st_ino) != (expected.device, expected.inode):
            return False
        # Directory arrivals can contain files written before watches attached.
        found = set()
        for directory, names, files in os.walk(path, followlinks=False):
            names[:] = [name for name in names if not name.startswith(".neutrondrive-")]
            for name in names + files:
                if name.startswith(".neutrondrive-"):
                    continue
                sub = (Path(directory) / name).relative_to(root).as_posix()
                found.add(sub)
                entry = snapshot.get(sub)
                try:
                    st = (root / sub).lstat()
                except OSError:
                    return False
                if entry is None or stat.S_ISLNK(st.st_mode):
                    return False
                if entry.kind == "folder" and (not stat.S_ISDIR(st.st_mode) or (st.st_dev, st.st_ino) != (entry.device, entry.inode)):
                    return False
                if entry.kind == "file" and (not stat.S_ISREG(st.st_mode) or (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns, st.st_ctime_ns) != (entry.device, entry.inode, entry.size, entry.mtime_ns, entry.ctime_ns)):
                    return False
        return found == {key for key in snapshot if key.startswith(relative + "/")}
    return stat.S_ISREG(info.st_mode) and (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns) == (expected.device, expected.inode, expected.size, expected.mtime_ns, expected.ctime_ns)


def _echo_snapshot(plan: Any, result: Any) -> dict[str, Any]:
    """Suppress only settled writes, retaining edits that arrived during a run."""
    from .sync import _Entry
    values = rust_core.call("echo", previous=plan._local, current=result._local_snapshot,
                            remote=result._remote_snapshot, trashed=getattr(result, "trashed", []))
    return {path: _Entry(**entry) for path, entry in values.items()}


@dataclass
class _Runtime:
    job: Job
    state: str = "pending"
    pending: bool = True
    due: float = 0.0
    first_dirty: float = 0.0
    paths: set[str | None] = field(default_factory=set)
    reasons: set[str] = field(default_factory=lambda: {"startup"})
    known_uids: set[str] = field(default_factory=set)
    watcher: Any = None
    watch_token: str = ""
    failures: int = 0
    last_sync: float | None = None
    error: str = ""
    conflicts: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)
    completed: int = 0
    rebuild_watch: bool = False
    local_snapshot: dict[str, Any] = field(default_factory=dict)


class BackgroundDaemon:
    def __init__(self, state_dir: Path, backend: Any, event_source: Any, *,
                 debounce: float = 1.5, watcher_factory=InotifyWatcher,
                 engine_factory=SyncEngine, retry_base: float = 30.0, retry_max: float = 900.0,
                 mount_paths: list[Path] = ()):
        self._core_info = rust_core.call("info")  # Fail before starting a daemon without its native policy core.
        self.store = JobStore(state_dir, mount_paths=mount_paths)
        self.state_dir = self.store.state_dir
        self.backend, self.event_source = backend, event_source
        self.remote_cache = RemoteTreeCache(backend)
        self.debounce, self.retry_base, self.retry_max = debounce, retry_base, retry_max
        self.watcher_factory, self.engine_factory = watcher_factory, engine_factory
        self._condition = threading.Condition(threading.RLock())
        self._jobs: dict[str, _Runtime] = {}
        self._unresolved_events: list[dict[str, Any]] = []
        self._stopping = False
        self._current: str | None = None
        self._server: socket.socket | None = None
        self._server_thread: threading.Thread | None = None
        self._lock_fd: int | None = None
        self._event_status: dict[str, Any] = {"state": "starting", "error": ""}
        self.socket_path = self.state_dir / "control.sock"
        if len(os.fsencode(self.socket_path)) > 106:
            raise BackgroundError("Dienstpfad zu lang für den lokalen Steuerungssocket.")

    def _status(self) -> dict[str, Any]:
        now = time.monotonic()
        return {"version": 1, "running": not self._stopping and self._lock_fd is not None,
                "pid": os.getpid(), "updated_at": time.time(), "event_source": dict(self._event_status),
                "rust_core": dict(self._core_info),
                "unresolved_events": list(self._unresolved_events),
                "jobs": [{**asdict(runtime.job), "state": runtime.state, "pending": runtime.pending,
                          "last_sync": runtime.last_sync, "error": runtime.error,
                          "conflicts": runtime.conflicts[:1000], "skipped": runtime.skipped[:1000],
                          "completed": runtime.completed, "reasons": sorted(runtime.reasons),
                          "retry_at": time.time() + max(0, runtime.due - now) if runtime.state == "offline" else None}
                         for runtime in self._jobs.values()]}

    def _persist(self) -> None:
        _write_json(self.state_dir / "status.json", self._status())

    def _schedule(self, runtime: _Runtime, reason: str, relative: str | None = None, *, immediate=False) -> None:
        state = {key: getattr(runtime, key) for key in
                 ("state", "pending", "due", "first_dirty", "paths", "reasons", "rebuild_watch")}
        state["paused"] = runtime.job.paused
        updated = rust_core.call("schedule", state=state, reason=reason, relative=relative,
                                 immediate=immediate, now=time.monotonic(), debounce=self.debounce)
        for key, value in updated.items():
            if key != "paused":
                setattr(runtime, key, set(value) if key in {"paths", "reasons"} else value)
        self._condition.notify_all()

    def _local_event(self, job_id: str, relative: str | None, reason: str, token: str | None = None) -> None:
        with self._condition:
            runtime = self._jobs.get(job_id)
            if runtime is None or self._stopping or (token is not None and token != runtime.watch_token):
                return
            if relative is not None and runtime.state != "syncing" and runtime.local_snapshot:
                if _matches_snapshot(Path(runtime.job.local_root), relative, runtime.local_snapshot):
                    return
            self._schedule(runtime, reason, relative)

    def remote_event(self, event: Any) -> None:
        """Persist accepted work before returning: this is the event cursor ACK."""
        def get(name, default=None):
            return event.get(name, default) if isinstance(event, dict) else getattr(event, name, default)
        with self._condition:
            if self._stopping:
                raise BackgroundError("Dienst wird beendet; Event-Cursor nicht bestätigen.")
            identity = {name: get(name) for name in ("node_uid", "parent_uid", "root_uid")}
            reset = bool(get("reset", False))
            decision = rust_core.call("route_event", event={
                "kind": get("kind", "change"), "reset": reset, "identity": identity,
            }, jobs=[{
                "id": runtime.job.id, "remote_uid": runtime.job.remote_uid,
                "account_root_uid": runtime.job.account_root_uid,
                "known_uids": runtime.known_uids, "paused": runtime.job.paused,
            } for runtime in self._jobs.values()], pending=self._unresolved_events)
            if decision["offline"]:
                self._event_status = {"state": "offline", "error": "Remote-Eventverbindung unterbrochen; Wiederverbindung läuft."}
            else:
                self._event_status = {"state": "connected", "error": ""}
                affected_roots = []
                for job_id in decision["affected"]:
                    runtime = self._jobs[job_id]
                    affected_roots.append(runtime.job.remote_root)
                    self._schedule(runtime, decision["reason"])
                if affected_roots:
                    self.remote_cache.event(identity["node_uid"], identity["parent_uid"], affected_roots, reset=reset)
                if decision["append"] is not None:
                    self._unresolved_events.append(decision["append"])
                for job_id in decision["overflow"]:
                    runtime = self._jobs[job_id]
                    self.remote_cache.invalidate_tree(runtime.job.remote_root)
                    self._schedule(runtime, "remote-overflow")
                if decision["resolve_unknown"]:
                    self._condition.notify_all()
            self._persist()

    def _resolve_events(self, batch: list[dict[str, Any]]) -> None:
        """Resolve unknown parents using metadata, outside the durable ACK path."""
        for event in batch:
            with self._condition:
                active = [runtime for runtime in self._jobs.values() if not runtime.job.paused]
                account_ids = {runtime.job.account_root_uid for runtime in active} - {""}
                known = {uid: runtime for runtime in active
                         for uid in runtime.known_uids | {runtime.job.remote_uid} if uid}
            affected, ancestor = [], None
            try:
                if event["root_uid"] and account_ids and event["root_uid"] not in account_ids:
                    raise _Blocked("Account identity changed in event stream")
                current = event["parent_uid"]
                if not current:
                    node = self.backend.info("/my-files/" + event["node_uid"])
                    if not node.safe or node.uid != event["node_uid"]:
                        raise _Blocked("Unsafe event node")
                    current = node.raw.get("parentUid")
                visited = set()
                for _ in range(128):
                    if current in known:
                        affected, ancestor = [known[current]], current
                        break
                    if current in account_ids:
                        break  # Proven outside every configured job subtree.
                    if not current or current in visited:
                        raise _Blocked("Unknown event ancestry")
                    visited.add(current)
                    node = self.backend.info("/my-files/" + current)
                    if not node.safe or node.uid != current:
                        raise _Blocked("Unsafe event ancestry")
                    current = node.raw.get("parentUid")
                else:
                    raise _Blocked("Event ancestry exceeds maximum depth")
            except Cancelled:
                if self._stopping:
                    return
                affected = active
            except Exception:
                # A deleted unknown ancestor cannot be queried anymore. One
                # event-triggered reconciliation is the conservative fallback;
                # network failures then use the ordinary bounded job backoff.
                affected = active
            with self._condition:
                roots = [runtime.job.remote_root for runtime in affected]
                if roots:
                    self.remote_cache.event(event["node_uid"], ancestor, roots, reset=ancestor is None)
                    for runtime in affected:
                        self._schedule(runtime, "remote-ancestry")
                if event in self._unresolved_events:
                    self._unresolved_events.remove(event)
                self._persist()
                self._condition.notify_all()

    def _watch(self, runtime: _Runtime) -> None:
        runtime.watch_token = uuid.uuid4().hex
        if runtime.watcher:
            runtime.watcher.stop()
        root = Path(runtime.job.local_root)
        identity = _validate_local(root, self.state_dir, self.store.mount_paths)
        if identity != (runtime.job.local_device, runtime.job.local_inode):
            raise _Blocked("Der lokale Sync-Ordner wurde ersetzt; Job bewusst neu anlegen.")
        runtime.watcher = self.watcher_factory(root, lambda path, reason, jid=runtime.job.id, token=runtime.watch_token: self._local_event(jid, path, reason, token))
        runtime.watcher.start()
        runtime.rebuild_watch = False

    def _reload(self) -> None:
        configurations = {job.id: job for job in self.store.list()}
        for job_id in list(self._jobs):
            if job_id not in configurations:
                runtime = self._jobs.pop(job_id)
                if runtime.watcher:
                    runtime.watcher.stop()
                if self._current == job_id:
                    self.backend.cancel()
        for job in configurations.values():
            runtime = self._jobs.get(job.id)
            was_paused = runtime.job.paused if runtime else True
            if runtime is None:
                runtime = _Runtime(job)
                self._jobs[job.id] = runtime
            runtime.job = job
            if job.paused:
                runtime.state, runtime.pending = "paused", False
                runtime.watch_token = ""
                if runtime.watcher:
                    runtime.watcher.stop()
                    runtime.watcher = None
                if self._current == job.id:
                    self.backend.cancel()
            elif was_paused or runtime.watcher is None:
                try:
                    self._watch(runtime)
                    self.remote_cache.invalidate_tree(job.remote_root)
                    runtime.state, runtime.error = "pending", ""
                    self._schedule(runtime, "startup" if was_paused else "resume", immediate=True)
                except (OSError, BackgroundError, SyncError) as error:
                    runtime.state, runtime.pending, runtime.error = "blocked", False, str(error)
        self._condition.notify_all()

    def command(self, action: str, job_id: str | None = None) -> dict[str, Any]:
        with self._condition:
            if action == "status":
                return self._status()
            if action == "stop":
                self._stopping = True
                self.backend.cancel()
                self._condition.notify_all()
            elif action == "reload":
                self._reload()
            elif action in {"pause", "resume", "sync"}:
                selected = [job for job in self.store.list() if job_id is None or job.id == job_id]
                if not selected:
                    raise BackgroundError("Sync-Job wurde nicht gefunden.")
                if action != "sync":
                    for job in selected:
                        self.store.update(job.id, paused=action == "pause")
                    # Explicit resume may retry a blocked job, but never erases
                    # pinned account/root identities.
                    for job in selected:
                        runtime = self._jobs.get(job.id)
                        if runtime and action == "resume":
                            runtime.state = "pending"
                            runtime.watch_token = ""
                            if runtime.watcher:
                                runtime.watcher.stop()
                                runtime.watcher = None
                    self._reload()
                else:
                    for job in selected:
                        runtime = self._jobs.get(job.id)
                        if runtime and not job.paused:
                            self.remote_cache.invalidate_tree(job.remote_root)
                            self._schedule(runtime, "manual", immediate=True)
            else:
                raise BackgroundError("Unbekannter Dienstbefehl.")
            self._persist()
            return self._status()

    def _resolve(self, runtime: _Runtime) -> Job:
        account = self.backend.info("/my-files")
        if not account.safe or account.type != "folder" or not account.uid or account.raw.get("isTrashed"):
            raise _Blocked("Proton-Kontowurzel konnte nicht sicher bestätigt werden.")
        active = [job for job in self.store.list() if not job.paused]
        pinned = []
        nodes = {}
        for job in active:
            if job.account_root_uid and job.account_root_uid != account.uid:
                raise _Blocked("Proton-Konto wurde gewechselt; betroffene Jobs bewusst neu anlegen.")
            node = self.backend.info(job.remote_root)
            if not node.safe or node.type != "folder" or not node.uid or node.raw.get("isTrashed"):
                raise _Blocked("Ein Remote-Sync-Ordner ist nicht sicher verfügbar.")
            if job.remote_uid and job.remote_uid != node.uid:
                raise _Blocked("Remote-Wurzel wurde geändert; Job bewusst neu anlegen.")
            pinned.append(replace(job, remote_uid=node.uid, account_root_uid=account.uid, remote_root=node.path))
            nodes[job.id] = node
        job = next((entry for entry in pinned if entry.id == runtime.job.id), None)
        if job is None:
            raise Cancelled("Sync-Job wurde pausiert oder entfernt.")
        other_ids = {entry.remote_uid for entry in pinned if entry.id != job.id}
        if job.remote_uid in other_ids:
            raise _Blocked("Mehrere Sync-Jobs verweisen auf denselben Remote-Ordner.")
        # Follow only root ancestor metadata, not tree listings. Check every
        # pair before writing either side, including an ancestor job added last.
        all_ids = {entry.remote_uid for entry in pinned}
        if len(all_ids) != len(pinned):
            raise _Blocked("Mehrere Sync-Jobs verweisen auf denselben Remote-Ordner.")
        # Also check ancestry for a single job: a selected subfolder inside a
        # trashed ancestor must not be interpreted as many individual deletions.
        for entry in pinned:
            current, visited = nodes[entry.id], {entry.remote_uid}
            for _ in range(128):
                if current.uid == account.uid:
                    break
                parent_uid = current.raw.get("parentUid")
                if not parent_uid:
                    raise _Blocked("Remote-Ordnerabgrenzung nicht nachweisbar; Jobs überlappen möglicherweise.")
                if parent_uid in all_ids:
                    raise _Blocked("Remote-Sync-Jobs dürfen keine gemeinsamen Unterbäume haben.")
                if parent_uid == account.uid:
                    break
                if parent_uid in visited:
                    raise _Blocked("Zyklische Remote-Ordnerstruktur.")
                visited.add(parent_uid)
                current = self.backend.info("/my-files/" + parent_uid)
                if (not current.safe or current.type != "folder" or current.uid != parent_uid
                        or current.raw.get("isTrashed")):
                    raise _Blocked("Remote-Ordnerabgrenzung konnte nicht sicher bestätigt werden.")
            else:
                raise _Blocked("Remote-Ordnerstruktur überschreitet die sichere maximale Tiefe.")
        # Validate the entire set before canonicalizing persisted paths: aliases
        # must never leave a half-updated, overlapping configuration behind.
        pinned = [self.store._pin(entry.id, entry.remote_uid, entry.account_root_uid, entry.remote_root) for entry in pinned]
        with self._condition:
            for entry in pinned:
                if entry.id in self._jobs:
                    self._jobs[entry.id].job = entry
        return job

    def _run_job(self, runtime: _Runtime) -> None:
        try:
            identity = _validate_local(Path(runtime.job.local_root), self.state_dir, self.store.mount_paths)
            if identity != (runtime.job.local_device, runtime.job.local_inode):
                raise _Blocked("Lokaler Sync-Ordner wurde ersetzt; Job bewusst neu anlegen.")
            job = self._resolve(runtime)
            engine = self.engine_factory(self.remote_cache, self.state_dir / "sync")
            plan = engine.plan(Path(job.local_root), job.remote_root)
            if plan.remote_root_uid != job.remote_uid:
                raise _Blocked("Remote-Wurzel wurde während der Vorbereitung geändert.")
            with self._condition:
                current = next((item for item in self.store.list() if item.id == job.id), None)
                if self._stopping or current is None or current.paused:
                    raise Cancelled("Sync-Job wurde angehalten.")
                runtime.known_uids = {entry.uid for entry in plan._remote.values()} | {job.remote_uid}
            result = engine.execute(plan)
            with self._condition:
                if not result.errors:
                    runtime.failures = 0
                runtime.last_sync = time.time()
                runtime.completed = len(result.completed)
                runtime.conflicts = list(result.conflicts)
                runtime.skipped = list(result.skipped)
                runtime.local_snapshot = _echo_snapshot(plan, result)
                runtime.known_uids |= {entry.uid for entry in result._remote_snapshot.values()}
                runtime.error = "\n".join(result.errors[:10])
                runtime.state = "conflicts" if result.conflicts else "idle"
                if result.errors:
                    self._retry(runtime, runtime.error)
                elif runtime.paths:
                    dirty = {path for path in runtime.paths if path is None or not _matches_snapshot(Path(job.local_root), path, runtime.local_snapshot)}
                    runtime.paths = dirty
                    if not dirty and runtime.reasons <= {"local", "startup", "resume"}:
                        runtime.pending = False
                        runtime.reasons.clear()
                if runtime.rebuild_watch and not runtime.job.paused:
                    self._watch(runtime)
        except Cancelled:
            with self._condition:
                runtime.state = "paused" if runtime.job.paused else "stopped" if self._stopping else "pending"
                runtime.pending = not runtime.job.paused and not self._stopping
                runtime.due = time.monotonic() + self.debounce
        except _Blocked as error:
            with self._condition:
                runtime.state, runtime.pending, runtime.error = "blocked", False, str(error)
        except (BackendError, SyncError, OSError) as error:
            with self._condition:
                self._retry(runtime, str(error))
        except Exception:
            with self._condition:
                runtime.state, runtime.pending = "blocked", False
                runtime.error = "Unerwarteter Dienstfehler; Job bleibt zum Schutz der Daten angehalten."
        finally:
            with self._condition:
                if runtime.job.paused:
                    runtime.state, runtime.pending = "paused", False
                self._current = None
                self._persist()
                self._condition.notify_all()

    def _retry(self, runtime: _Runtime, error: str) -> None:
        update = rust_core.call("retry", failures=runtime.failures, now=time.monotonic(),
                                base=self.retry_base, maximum=self.retry_max, jitter=random.uniform(0.9, 1.1))
        runtime.failures, runtime.due = update["failures"], update["due"]
        runtime.state, runtime.pending, runtime.error = "offline", True, error
        runtime.reasons.add("retry")

    def _serve(self) -> None:
        while not self._stopping:
            try:
                client, _ = self._server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            with client:
                client.settimeout(2.0)
                try:
                    _pid, uid, _gid = struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                    if uid != os.getuid():
                        continue
                    payload = bytearray()
                    while b"\n" not in payload:
                        block = client.recv(4096)
                        if not block or len(payload) + len(block) > 32768:
                            raise BackgroundError("Ungültige IPC-Anfrage.")
                        payload.extend(block)
                    request = json.loads(bytes(payload).split(b"\n", 1)[0])
                    response = {"ok": True, "result": self.command(request["action"], request.get("job_id"))}
                except (OSError, ValueError, KeyError, TypeError, BackgroundError) as error:
                    response = {"ok": False, "error": str(error) if isinstance(error, BackgroundError) else "Ungültige Steuerungsanfrage."}
                try:
                    client.sendall(json.dumps(response).encode() + b"\n")
                except OSError:
                    pass

    def run(self) -> int:
        owns_socket = owns_pid = False
        self._lock_fd = os.open(self.state_dir / "daemon.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            fcntl.flock(self._lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            os.close(self._lock_fd)
            self._lock_fd = None
            raise BackgroundError("Der Hintergrunddienst läuft bereits.") from error
        try:
            if self.socket_path.exists() or self.socket_path.is_symlink():
                info = self.socket_path.lstat()
                if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
                    raise BackgroundError("Unsicherer Steuerungssocket; Start abgebrochen.")
                self.socket_path.unlink()
            self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self._server.bind(str(self.socket_path))
            owns_socket = True
            self.socket_path.chmod(0o600)
            self._server.listen(8)
            self._server.settimeout(0.5)
            _write_json(self.state_dir / "daemon.pid", {"pid": os.getpid()})
            owns_pid = True
            with self._condition:
                self._reload()
                self._persist()
            self._server_thread = threading.Thread(target=self._serve, name="neutrondrive-control", daemon=True)
            self._server_thread.start()
            self.event_source.start(self.remote_event)
            while True:
                event_batch = []
                with self._condition:
                    if self._stopping:
                        break
                    if self._unresolved_events:
                        event_batch = list(self._unresolved_events[:64])
                        self.backend.reset_cancel()
                    if event_batch:
                        next_runtime = None
                    else:
                        decision = rust_core.call("select", now=time.monotonic(), jobs=[
                            {"id": runtime.job.id, "pending": runtime.pending, "paused": runtime.job.paused,
                             "state": runtime.state, "due": runtime.due} for runtime in self._jobs.values()])
                        next_runtime = self._jobs[decision["id"]] if decision is not None else None
                    if event_batch:
                        pass
                    elif next_runtime is None:
                        self._condition.wait()
                        continue
                    else:
                        delay = next_runtime.due - time.monotonic()
                        if delay > 0:
                            self._condition.wait(delay)
                            continue
                        self._current = next_runtime.job.id
                        next_runtime.state, next_runtime.pending = "syncing", False
                        next_runtime.paths.clear()
                        next_runtime.reasons.clear()
                        next_runtime.first_dirty = 0
                        self.backend.reset_cancel()
                        self._persist()
                if event_batch:
                    self._resolve_events(event_batch)
                else:
                    self._run_job(next_runtime)
            return 0
        finally:
            self._stopping = True
            self.backend.cancel()
            try:
                self.event_source.stop()
            finally:
                for runtime in self._jobs.values():
                    if runtime.watcher:
                        runtime.watcher.stop()
                    runtime.state = "paused" if runtime.job.paused else "stopped"
                for runtime in self._jobs.values():
                    join = getattr(runtime.watcher, "join", None)
                    if join:
                        join()
                if self._server:
                    self._server.close()
                if self._server_thread and self._server_thread is not threading.current_thread():
                    self._server_thread.join(timeout=3)
                for path in ([self.socket_path] if owns_socket else []) + ([self.state_dir / "daemon.pid"] if owns_pid else []):
                    try:
                        path.unlink()
                    except FileNotFoundError:
                        pass
                try:
                    with self._condition:
                        self._persist()
                finally:
                    if self._lock_fd is not None:
                        os.close(self._lock_fd)
                        self._lock_fd = None


class Controller:
    """Small authenticated local-IPC client used by the GUI and CLI."""

    def __init__(self, state_dir: Path, executable: str | None = None):
        self.state_dir = _private_directory(Path(state_dir))
        self.executable = executable
        self.socket_path = self.state_dir / "control.sock"

    def command(self, action: str, job_id: str | None = None, *, timeout: float = 5.0) -> dict[str, Any]:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(timeout)
            try:
                connection.connect(str(self.socket_path))
                _pid, uid, _gid = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                if uid != os.getuid():
                    raise BackgroundError("Dienstsocket gehört einem anderen Benutzer.")
                connection.sendall(json.dumps({"action": action, "job_id": job_id}).encode() + b"\n")
                data = bytearray()
                while b"\n" not in data:
                    chunk = connection.recv(16384)
                    if not chunk or len(data) + len(chunk) > 8 * 1024 * 1024:
                        raise BackgroundError("Unvollständige Dienstantwort.")
                    data.extend(chunk)
                response = json.loads(bytes(data).split(b"\n", 1)[0])
            except (OSError, ValueError) as error:
                raise BackgroundError("Hintergrunddienst ist nicht erreichbar.") from error
            if not response.get("ok"):
                raise BackgroundError(response.get("error", "Dienstbefehl fehlgeschlagen."))
            return response["result"]

    def status(self) -> dict[str, Any]:
        try:
            return self.command("status")
        except BackgroundError:
            value = _read_json(self.state_dir / "status.json", {"jobs": [], "event_source": {"state": "stopped"}})
            if not isinstance(value, dict) or not isinstance(value.get("jobs"), list):
                raise BackgroundError("Gespeicherter Dienststatus ist beschädigt.")
            value["running"] = False
            return value

    def start(self) -> dict[str, Any]:
        if self.status().get("running"):
            return self.status()
        command = [sys.executable, "-m", "neutrondrive", "--daemon", "--state-dir", str(self.state_dir)]
        if self.executable:
            command += ["--cli", self.executable]
        fd = os.open(self.state_dir / "daemon.log", os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
        try:
            environment = os.environ.copy()
            package_root = str(Path(__file__).resolve().parent.parent)
            environment["PYTHONPATH"] = os.pathsep.join(filter(None, [package_root, environment.get("PYTHONPATH")]))
            process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=fd, stderr=fd,
                                       start_new_session=True, close_fds=True, env=environment)
        finally:
            os.close(fd)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            status = self.status()
            if status.get("running"):
                return status
            if process.poll() is not None:
                raise BackgroundError("Hintergrunddienst konnte nicht starten; privates Dienstprotokoll prüfen.")
            time.sleep(0.05)
        # This process belongs to this exact start attempt. Do not leave a
        # delayed, unacknowledged daemon that could begin transfers afterwards.
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise BackgroundError("Dienststart hat das Zeitlimit überschritten; Beendigung noch nicht bestätigt.") from error
        raise BackgroundError("Dienststart hat das Zeitlimit überschritten; der gestartete Prozess wurde beendet.")

    def stop(self) -> dict[str, Any]:
        if not self.status().get("running"):
            return self.status()
        self.command("stop")
        deadline = time.monotonic() + 10
        while self.socket_path.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        if self.socket_path.exists():
            raise BackgroundError("Dienstbeendigung ist noch nicht bestätigt; laufende Übertragung wird abgebrochen.")
        return self.status()


def run_daemon(state_dir: Path, backend: Any = None, event_source: Any = None) -> int:
    """Foreground entry point; systemd/Controller own process lifetime."""
    backend = backend or Backend()
    if event_source is None:
        try:
            from .events import EventSource
        except ImportError as error:
            raise BackgroundError("Die Bridge für echte Proton-Ereignisse fehlt; Hintergrundsync ist nicht verfügbar.") from error
        event_source = EventSource(backend, Path(state_dir) / "events", roots=("/my-files",))
    daemon = BackgroundDaemon(state_dir, backend, event_source)
    old_handlers = {}
    if threading.current_thread() is threading.main_thread():
        def stop(_signum, _frame):
            daemon.command("stop")
        for sig in (signal.SIGTERM, signal.SIGINT):
            old_handlers[sig] = signal.signal(sig, stop)
    try:
        return daemon.run()
    finally:
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)
