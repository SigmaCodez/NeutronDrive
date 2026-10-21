"""Typed, serialized adapter for the pinned official Proton Drive CLI 0.9.0.

Encryption, authentication, caching and HTTP backoff belong to Proton's SDK.
No shell is involved. Never log command output: it can contain account data.
"""
from __future__ import annotations

import collections
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

    def version(self) -> str:
        return self._run(["version"], timeout=30, parse=False)

    def login(self, on_url: Callable[[str], None]):
        return self._run(["auth", "login", "--json"], timeout=600, on_login=on_url)

    def logout(self):
        return self._run(["auth", "logout", "--json"], timeout=60)

    def list_nodes(self, path: str) -> list[Node]:
        result = self._run(["filesystem", "list", "--json", "--", validate_remote(path)])
        if not isinstance(result, list):
            raise BackendError("Die Proton-CLI lieferte keine Dateiliste.")
        nodes = [Node.parse(item, path) for item in result]
        counts = collections.Counter(node.name for node in nodes)
        if path in {"/trash", "/shared-with-me", "/devices"}:
            nodes = [dataclasses.replace(n, safe=False) if counts[n.name] > 1 else n for n in nodes]
        return sorted(nodes, key=lambda node: (node.type != "folder", node.name.casefold()))

    def info(self, path: str) -> Node:
        result = self._run(["filesystem", "info", "--json", "--", validate_remote(path)])
        return Node.parse(result, path.rsplit("/", 1)[0] or path)

    def mkdir(self, parent: str, name: str) -> Node:
        result = self._run(["filesystem", "create-folder", "--json", "--", validate_remote(parent), validate_name(name)])
        return Node.parse(result, parent)

    def rename(self, path: str, name: str):
        return self._run(["filesystem", "rename", "--json", "--", validate_remote(path), validate_name(name)])

    def move(self, paths: list[str], destination: str):
        if not paths:
            raise BackendError("Keine Dateien ausgewählt.")
        return self._run(["filesystem", "move", "--json", "--", *map(validate_remote, paths), validate_remote(destination)])

    def trash(self, paths: list[str]):
        if not paths:
            raise BackendError("Keine Dateien ausgewählt.")
        return self._run(["filesystem", "trash", "--json", "--", *map(validate_remote, paths)])

    def restore(self, paths: list[str]):
        if not paths:
            raise BackendError("Keine Dateien ausgewählt.")
        # Recheck names just before restoring: trash cannot be addressed by UID.
        current = self.list_nodes("/trash")
        for path in paths:
            matches = [n for n in current if n.path == path]
            if len(matches) != 1 or not matches[0].safe:
                raise BackendError("Mehrdeutiger Papierkorb-Eintrag. Bitte in der Proton-Web-App wiederherstellen.")
        return self._run(["filesystem", "restore", "--json", "--", *map(validate_remote, paths)])

    def upload(self, paths: list[Path], parent: str, file_strategy="rename", folder_strategy="merge"):
        if not paths or file_strategy not in {"rename", "create-new-revision", "skip"} or folder_strategy not in {"merge", "rename", "skip"}:
            raise BackendError("Ungültige Upload-Einstellungen.")
        sources = [_native("local_path", path=str(Path(p).expanduser()), directory=False,
                           recursive=True, cancel=self.cancel_event) for p in paths]
        return self._run(["filesystem", "upload", "--json", "--file-conflict-strategy", file_strategy,
                          "--folder-conflict-strategy", folder_strategy, "--", *map(str, sources), validate_remote(parent)])

    def download(self, paths: list[str], destination: Path, file_strategy="rename", folder_strategy="merge"):
        if not paths or file_strategy not in {"rename", "skip"} or folder_strategy not in {"merge", "rename", "skip"}:
            raise BackendError("Ungültige Download-Einstellungen.")
        dest = local_path(destination, directory=True)
        return self._run(["filesystem", "download", "--json", "--file-conflict-strategy", file_strategy,
                          "--folder-conflict-strategy", folder_strategy, "--", *map(validate_remote, paths), str(dest)])

    def sharing(self, path: str):
        return self._optional_sharing("status", path)

    def create_link(self, path: str):
        return self._run(["sharing", "set-url", "--json", "--role", "viewer", "--", validate_remote(path)])

    def remove_link(self, path: str):
        return self._optional_sharing("remove-url", path)

    def _optional_sharing(self, command: str, path: str):
        # Upstream printObject(undefined, true) prints the literal `undefined`
        # for unshared nodes. Only these optional ShareResult commands allow it.
        output = self._run(["sharing", command, "--json", "--", validate_remote(path)], parse=False)
        if output == "undefined":
            return None
        value = checked_json(output)
        if not isinstance(value, dict):
            raise BackendError("Die Proton-CLI lieferte keinen gültigen Freigabestatus.")
        return value
