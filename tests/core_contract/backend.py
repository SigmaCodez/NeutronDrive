"""Typed, serialized adapter for the pinned official Proton Drive CLI 0.9.0.

Encryption, authentication, caching and HTTP backoff belong to Proton's SDK.
No shell is involved. Never log command output: it can contain account data.
"""
from __future__ import annotations

import dataclasses
import os
from pathlib import Path
import shutil
import threading
from typing import Callable


class BackendError(RuntimeError):
    """A user-visible backend failure, without raw credential-bearing output."""


class Cancelled(BackendError):
    pass


MAX_OUTPUT = 64 * 1024 * 1024


def _native(operation, **arguments):
    from .rust_core import backend_value
    return backend_value(operation, **arguments)


def data_dir() -> Path:
    return Path(_native("data_dir"))


def validate_remote(path: str) -> str:
    return _native("validate_remote", path=path)


def validate_name(name: str) -> str:
    return _native("validate_name", name=name)


def join_remote(parent: str, name: str) -> str:
    return _native("join_remote", parent=parent, name=name)


def local_path(path: Path, *, directory: bool = False) -> Path:
    return Path(_native("local_path", path=str(Path(path).expanduser()), directory=directory, recursive=False))


@dataclasses.dataclass(frozen=True)
class Node:
    uid: str
    name: str
    type: str
    size: int | None = None
    modified: str = ""
    revision: str = ""
    sha1: str = ""
    path: str = ""
    raw: dict = dataclasses.field(default_factory=dict, compare=False, repr=False)
    safe: bool = True
    media_type: str = ""

    @classmethod
    def parse(cls, item: dict, parent: str) -> Node:
        return cls(**_native("node", item=item, parent=parent))


def checked_json(output: str):
    return _native("checked_json", output=output)


def friendly_error(stderr: str) -> str:
    return _native("friendly_error", stderr=stderr)


class Backend:
    def __init__(self, executable: str | None = None):
        self.executable = executable or os.environ.get("NEUTRONDRIVE_CLI") or shutil.which("proton-drive") or ""
        self.cancel_event = threading.Event()
        self._mutex = threading.Lock()

    @property
    def available(self) -> bool:
        return bool(self.executable and Path(self.executable).is_file() and os.access(self.executable, os.X_OK))

    def cancel(self):
        self.cancel_event.set()

    def reset_cancel(self):
        self.cancel_event.clear()

    def _run(self, args: list[str], *, timeout=3600, on_login: Callable[[str], None] | None = None, parse=True):
        if not self.available:
            raise BackendError("Die offizielle Proton Drive CLI fehlt. Bitte ein NeutronDrive-Paket mit Backend installieren oder NEUTRONDRIVE_CLI setzen.")
        from .rust_core import run_backend
        with self._mutex:
            return run_backend(self.executable, args, timeout=timeout, parse=parse,
                               on_login=on_login, cancel=self.cancel_event, max_output=MAX_OUTPUT)

    def _drive(self, action, *, on_login=None, **arguments):
        if not self.available:
            raise BackendError("Die offizielle Proton Drive CLI fehlt. Bitte ein NeutronDrive-Paket mit Backend installieren oder NEUTRONDRIVE_CLI setzen.")
        from .rust_core import drive_operation
        with self._mutex:
            return drive_operation(self.executable, dict(action=action, **arguments),
                                   on_login=on_login, cancel=self.cancel_event)

    def version(self) -> str:
        return self._drive("version")

    def login(self, on_url):
        return self._drive("login", on_login=on_url)

    def logout(self):
        return self._drive("logout")

    def list_nodes(self, path: str) -> list[Node]:
        nodes = [Node(**value) for value in self._drive("list", path=path)]
        return sorted(nodes, key=lambda node: (node.type != "folder", node.name.casefold()))

    def info(self, path: str) -> Node:
        return Node(**self._drive("info", path=path))

    def mkdir(self, parent: str, name: str) -> Node:
        return Node(**self._drive("mkdir", parent=parent, name=name))

    def rename(self, path: str, name: str):
        return self._drive("rename", path=path, name=name)

    def move(self, paths: list[str], destination: str):
        return self._drive("move", paths=paths, destination=destination)

    def trash(self, paths: list[str]):
        return self._drive("trash", paths=paths)

    def restore(self, paths: list[str]):
        return self._drive("restore", paths=paths)

    def upload(self, paths: list[Path], parent: str, file_strategy="rename", folder_strategy="merge"):
        return self._drive("upload", paths=[str(p) for p in paths], parent=parent,
                           file_strategy=file_strategy, folder_strategy=folder_strategy)

    def download(self, paths: list[str], destination: Path, file_strategy="rename", folder_strategy="merge"):
        return self._drive("download", paths=paths, destination=str(destination),
                           file_strategy=file_strategy, folder_strategy=folder_strategy)

    def sharing(self, path: str):
        return self._drive("sharing", path=path)

    def create_link(self, path: str):
        return self._drive("create_link", path=path)

    def remove_link(self, path: str):
        return self._drive("remove_link", path=path)


def service_call(backend, operation_name, operation, *, on_poll=None, on_started=None, on_status=None):
    """Synchronous bindings for Rust-owned SDK orchestration."""
    from . import rust_core
    def event(value):
        if value['kind'] == 'fuse_status' and on_status is not None:
            on_status(value['status'])
            return
        if value['kind'] == 'sync_started' and on_started is not None:
            on_started()
            return
        if value['kind'] != 'host_request':
            raise BackendError('Unexpected native sync event')
        method, arguments = value['method'], value['arguments']
        if method not in {'info', 'try_info', 'list_nodes', 'list_nodes_fresh', 'mkdir', 'trash', 'upload', 'download', 'rename', 'move'}:
            raise BackendError('Unsupported native sync service')
        try:
            function = getattr(backend, 'info' if method == 'try_info' else method, backend.list_nodes)
            if method in {'info', 'try_info', 'list_nodes', 'list_nodes_fresh'}:
                result = function(arguments['path'])
            elif method == 'mkdir':
                result = function(arguments['parent'], arguments['name'])
            elif method == 'rename':
                result = function(arguments['path'], arguments['name'])
            elif method == 'move':
                result = function(arguments['paths'], arguments['destination'])
            elif method == 'trash':
                result = function(arguments['paths'])
            else:
                arguments = dict(arguments)
                paths = arguments.pop('paths')
                if method == 'upload':
                    result = function([Path(path) for path in paths], arguments.pop('parent'), **arguments)
                else:
                    result = function(paths, Path(arguments.pop('destination')), **arguments)
            if method == 'try_info':
                result = {'node': result}
        except Exception as error:
            if method == 'try_info' and not isinstance(error, Cancelled):
                result = {'unavailable': True}
            else:
                rust_core.backend_value('host_failure', id=value['id'], message=str(error), cancelled=isinstance(error, Cancelled))
                return
        rust_core.backend_value('host_reply', id=value['id'], value=result)
    arguments = operation if operation_name in {'sync_plan', 'sync_execute', 'event_ancestry', 'job_roots'} else dict(operation=operation)
    # Production FUSE calls the SDK directly from Rust. Injected backends keep
    # the synchronous test boundary; callbacks never escape their calling thread.
    native_mount = operation_name in {'ondemand_fs', 'fuse_session', 'mount_manager'} and type(backend) is Backend
    if native_mount:
        arguments['executable'] = backend.executable
    return rust_core.backend_value(operation_name, **arguments, on_poll=on_poll, on_event=event,
                                   cancel=backend.cancel_event if native_mount else None)
