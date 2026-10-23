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

from dataclasses import asdict, dataclass, field
import os
from pathlib import Path
import threading
from typing import Any
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


class SyncEngine:
    """Qt/daemon adapter for the native Rust sync runtime."""

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


    def _service(self, operation_name, operation, on_started=None):
        from .backend import service_call
        try:
            return service_call(self.backend, operation_name, operation,
                on_poll=self._check_cancel, on_started=on_started)
        except Cancelled:
            raise
        except RuntimeError as error:
            raise SyncError(str(error)) from error

    def plan(self, local_root: Path, remote_root: str) -> SyncPlan:
        with self._lock:
            root = Path(os.path.abspath(Path(local_root).expanduser()))
            value = self._service('sync_plan', dict(state=str(self.state_dir), root=str(root), remote_path=remote_root))
            value['local_root'] = Path(value['local_root'])
            value['actions'] = [SyncAction(**item) for item in value['actions']]
            value['conflicts'] = [SyncConflict(**item) for item in value['conflicts']]
            for key in ('_local', '_remote'):
                value[key] = {path: _Entry(**entry) for path, entry in value[key].items()}
            value['_root_identity'] = tuple(value['_root_identity'])
            value['_blocked'] = set(value['_blocked'])
            return SyncPlan(**value, _owner=self._owner)


    def _trash_store(self):
        if self.local_trash is None:
            from .local_trash import LocalTrash
            self.local_trash = LocalTrash(data_dir() / "local-trash")
        return self.local_trash


    def execute(self, plan: SyncPlan) -> SyncResult:
        with self._lock:
            if plan._owner != self._owner or plan._executed:
                raise SyncError('Use a fresh preview from this sync engine')
            self._check_cancel()
            value = asdict(plan)
            value['local_root'] = str(plan.local_root)
            store = self._trash_store()
            result = self._service('sync_execute', dict(state=str(self.state_dir), plan=value,
                trash_state=str(store.state_dir), trash_data=str(store.data_home), trash_desktop=store.prefer_desktop), on_started=lambda: setattr(plan, "_executed", True))
            for key in ('_local_snapshot', '_remote_snapshot'):
                result[key] = {path: _Entry(**entry) for path, entry in result[key].items()}
            return SyncResult(**result)
