"""Qt/CLI bindings for the native Rust background service and job store.

Rust owns daemon state, threads, watches, events, IPC and sync execution.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import signal
import threading
from typing import Any

from .backend import Backend, BackendError, Cancelled
from . import rust_core


class BackgroundError(BackendError):
    """A safe, user-readable daemon/configuration failure."""


class _Blocked(BackgroundError):
    """Requires a deliberate job/configuration change, not timed retries."""


class RemoteTreeCache:
    """Binding for Rust-owned event-invalidated directory metadata."""
    def __init__(self, backend):
        self.backend = backend
        self._handle = rust_core.backend_value('metadata_cache', operation=dict(action='create'))

    def __getattr__(self, name):
        return getattr(self.backend, name)

    def __del__(self):
        handle = self.__dict__.get('_handle')
        if handle is not None:
            try:
                rust_core.backend_value('metadata_cache', operation=dict(action='release', id=handle))
            except Exception:
                pass

    def _call(self, action, **arguments):
        from .backend import service_call
        return service_call(self.backend, 'metadata_cache', dict(action=action, id=self._handle, **arguments))

    def info(self, path):
        from .backend import Node
        return Node(**self._call('info', path=path))

    def list_nodes(self, path):
        from .backend import Node
        return [Node(**node) for node in self._call('list', path=path, fresh=False)]

    def list_nodes_fresh(self, path):
        from .backend import Node
        return [Node(**node) for node in self._call('list', path=path, fresh=True)]

    def invalidate(self, path):
        self._call('invalidate', path=path)

    def invalidate_tree(self, path):
        self.event(None, None, [path], reset=True)

    def event(self, node_uid, parent_uid, roots, *, reset=False):
        self._call('event', roots=roots, node_uid=node_uid, parent_uid=parent_uid, reset=reset)

    def upload(self, paths, parent, **options):
        return self._call('upload', paths=[str(path) for path in paths], parent=parent,
            file_strategy=options.get('file_strategy', 'rename'), folder_strategy=options.get('folder_strategy', 'merge'))

    def mkdir(self, parent, name):
        from .backend import Node
        return Node(**self._call('mkdir', parent=parent, name=name))

    def trash(self, paths):
        return self._call('trash', paths=paths)


def _private_directory(path: Path) -> Path:
    path = Path(os.path.abspath(Path(path).expanduser()))
    try:
        return Path(rust_core.backend_value("private_directory", path=str(path)))
    except BackendError as error:
        raise BackgroundError(str(error)) from error


def _read_json(path: Path, default: Any) -> Any:
    try:
        return rust_core.backend_value("store_read", path=str(path), default=default,
                                       limit=8 * 1024 * 1024, private=False)
    except BackendError as error:
        raise BackgroundError(str(error)) from error


def _write_json(path: Path, value: Any) -> None:
    try:
        rust_core.backend_value("store_write", path=str(path), value=value)
    except BackendError as error:
        raise BackgroundError(str(error)) from error


def _mount_paths() -> list[tuple[Path, str]]:
    try:
        return [(Path(path), kind) for path, kind in rust_core.backend_value("mount_paths")]
    except BackendError as error:
        raise _Blocked(str(error)) from error


def _validate_local(root: Path, state_dir: Path, mount_paths: list[Path] = ()) -> tuple[int, int]:
    try:
        return tuple(rust_core.backend_value("validate_local", root=str(root), state=str(state_dir),
                     planned=[str(p) for p in mount_paths], mounted=[(str(p), kind) for p, kind in _mount_paths()]))
    except BackendError as error:
        raise _Blocked(str(error)) from error


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
    """Compatibility adapter; Rust owns validation, locks and transactions."""

    def __init__(self, state_dir: Path, *, mount_paths: list[Path] = ()):
        self.state_dir = _private_directory(Path(state_dir))
        self.mount_paths = tuple(Path(path) for path in mount_paths)
        self.path = self.state_dir / "jobs.json"

    def _invoke(self, action, **arguments):
        try:
            return rust_core.backend_value("jobs", state=str(self.state_dir),
                planned=[str(path) for path in self.mount_paths], operation=dict(action=action, **arguments))
        except BackendError as error:
            raise BackgroundError(str(error)) from error

    def list(self) -> list[Job]:
        return [Job(**row) for row in self._invoke("list")]

    def add(self, local_root: Path, remote_root: str, name: str = "", *, paused: bool = False) -> Job:
        root = Path(os.path.abspath(Path(local_root).expanduser()))
        _validate_local(root, self.state_dir, self.mount_paths)
        return Job(**self._invoke("add", local_root=str(root), remote_root=remote_root, name=name, paused=paused))

    def update(self, job_id: str, *, paused: bool | None = None, name: str | None = None) -> Job:
        return Job(**self._invoke("update", id=job_id, name=name, paused=paused))

    def remove(self, job_id: str) -> None:
        self._invoke("remove", id=job_id)

    def _pin(self, job_id: str, uid: str, account_uid: str, remote_path: str) -> Job:
        try:
            return Job(**self._invoke("pin", id=job_id, uid=uid, account_uid=account_uid, remote_path=remote_path))
        except BackendError as error:
            raise _Blocked(str(error)) from error


class Controller:
    """Small authenticated local-IPC client used by the GUI and CLI."""

    def __init__(self, state_dir: Path, executable: str | None = None):
        self.state_dir = _private_directory(Path(state_dir))
        self.executable = executable
        self.socket_path = self.state_dir / "control.sock"

    def command(self, action: str, job_id: str | None = None, *, timeout: float = 5.0) -> dict[str, Any]:
        try:
            return rust_core.backend_value("control_command", path=str(self.socket_path),
                                           action=action, job_id=job_id, timeout=timeout)
        except BackendError as error:
            raise BackgroundError(str(error)) from error

    def _call(self, action: str, **arguments) -> dict[str, Any]:
        try:
            return rust_core.backend_value("service_controller", state=str(self.state_dir),
                                           operation={"action": action, **arguments})
        except BackendError as error:
            raise BackgroundError(str(error)) from error

    def status(self) -> dict[str, Any]:
        return self._call("status")

    def start(self) -> dict[str, Any]:
        command = [str(Path(__file__).resolve().parents[2] / "bin/neutrondrive"), "--daemon", "--state-dir", str(self.state_dir)]
        if self.executable:
            command += ["--cli", self.executable]
        return self._call("start", argv=command, environment={})

    def stop(self) -> dict[str, Any]:
        return self._call("stop")


def run_daemon(state_dir: Path, backend: Any = None) -> int:
    """Thin foreground binding; Rust owns the daemon and every worker thread."""
    backend = backend or Backend()
    stopped = threading.Event()
    old_handlers = {}
    if threading.current_thread() is threading.main_thread():
        for sig in (signal.SIGTERM, signal.SIGINT):
            old_handlers[sig] = signal.signal(sig, lambda *_: stopped.set())
    try:
        rust_core.backend_value("daemon", config={"state": str(Path(state_dir).absolute()),
            "executable": backend.executable}, cancel=stopped)
        return 0
    finally:
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)
