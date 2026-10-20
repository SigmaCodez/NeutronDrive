"""Typed, serialized adapter for the pinned official Proton Drive CLI 0.9.0.

Encryption, authentication, caching and HTTP backoff belong to Proton's SDK.
No shell is involved. Never log command output: it can contain account data.
"""
from __future__ import annotations

import collections
import contextlib
import dataclasses
import fcntl
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import stat
import subprocess
import threading
import time
from typing import Callable
from urllib.parse import urlsplit


class BackendError(RuntimeError):
    """A user-visible backend failure, without raw credential-bearing output."""


class Cancelled(BackendError):
    pass


UID = re.compile(r"^(?:[a-zA-Z0-9=_-]{88,108}|[a-zA-Z0-9_-]{22})~(?:[a-zA-Z0-9=_-]{88,108}|[a-zA-Z0-9_-]{22})$")
ROOTS = {"my-files", "trash", "shared-with-me", "devices"}
MAX_OUTPUT = 64 * 1024 * 1024


def data_dir() -> Path:
    base = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state")))
    folder = base / "neutrondrive"
    try:
        folder.mkdir(parents=True, exist_ok=True, mode=0o700)
        if folder.is_symlink() or folder.stat().st_uid != os.getuid():
            raise BackendError("Unsicheres NeutronDrive-Zustandsverzeichnis.")
        folder.chmod(0o700)
    except OSError as exc:
        raise BackendError("Das NeutronDrive-Zustandsverzeichnis ist nicht zugänglich. XDG_STATE_HOME und Schreibrechte prüfen.") from exc
    return folder


def validate_remote(path: str) -> str:
    if not isinstance(path, str) or not path.startswith("/") or "\x00" in path:
        raise BackendError("Ungültiger Cloud-Pfad.")
    if path.split("/", 2)[1] not in ROOTS:
        raise BackendError("Dieser Cloud-Bereich wird noch nicht unterstützt.")
    return path


def validate_name(name: str) -> str:
    if not name.strip() or name in {".", ".."} or any(ord(c) < 32 for c in name) or "/" in name or "\\" in name:
        raise BackendError("Der Name darf nicht leer sein oder Schrägstriche/Steuerzeichen enthalten.")
    return name


def join_remote(parent: str, name: str) -> str:
    validate_remote(parent)
    validate_name(name)
    return parent.rstrip("/") + "/" + name


def local_path(path: Path, *, directory: bool = False) -> Path:
    path = Path(os.path.abspath(Path(path).expanduser()))
    # Upstream expands globs even when subprocess argv is used. Fail closed.
    if any(c in str(path) for c in "*?[{"):
        raise BackendError("Die Proton-CLI behandelt *, ?, [ und { in lokalen Pfaden als Suchmuster. Bitte einen anderen Pfad verwenden.")
    for item in (path, *path.parents):
        if item.is_symlink():
            raise BackendError("Symbolische Links werden für Dateiübertragungen nicht unterstützt.")
    if not path.exists() or (directory and not path.is_dir()):
        raise BackendError("Der lokale Pfad existiert nicht oder ist kein Ordner.")
    if not (path.is_file() or path.is_dir()):
        raise BackendError("Nur reguläre Dateien und Ordner werden unterstützt.")
    return path


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
        if not isinstance(item, dict):
            raise BackendError("Unerwartetes Datenformat der Proton-CLI.")
        result = item.get("name", {})
        safe = isinstance(result, dict) and result.get("ok") is True
        name = result.get("value", "") if safe else "[Name nicht entschlüsselbar]"
        if not isinstance(name, str) or not name:
            name, safe = "[Ungültiger Name]", False
        uid = item.get("rootFolderUid", item.get("uid", ""))
        if not isinstance(uid, str):
            raise BackendError("Ungültige Datei-ID von Proton.")
        revision = item.get("activeRevision") or {}
        kind = "folder" if "rootFolderUid" in item else item.get("type", "unknown")
        safe = safe and not item.get("errors") and kind in {"file", "folder"}
        root = parent.split("/", 2)[1]
        # Drive nodes accept unambiguous UID segments, but virtual trash and
        # shared/device roots are name-only in the upstream CLI.
        if root == "my-files" and UID.fullmatch(uid):
            remote = "/my-files/" + uid
        elif root != "trash" and parent.count("/") > 1 and UID.fullmatch(uid):
            remote = parent + "/" + uid
        else:
            try:
                remote = join_remote(parent, name)
            except BackendError:
                remote, safe = "", False
        size = revision.get("claimedSize")
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            size = None
        digest = (revision.get("claimedDigests") or {}).get("sha1", "")
        return cls(uid, name, kind, size,
                   revision.get("claimedModificationTime") or item.get("modificationTime", ""),
                   revision.get("uid", ""), digest or "", remote, item, bool(safe), item.get("mediaType", ""))


def checked_json(output: str):
    if not output.strip():
        return None
    try:
        result = json.loads(output)
    except (ValueError, RecursionError) as exc:
        raise BackendError("Die Proton-CLI lieferte keine vollständige JSON-Antwort. Es wurde kein Erfolg angenommen.") from exc
    if isinstance(result, list):
        failed = [x for x in result if isinstance(x, dict) and x.get("ok") is False]
        if failed:
            raise BackendError(f"Proton meldet {len(failed)} fehlgeschlagene Dateioperation(en). Bitte Ansicht aktualisieren.")
    if isinstance(result, dict) and result.get("failedItems", 0):
        raise BackendError(f"{result['failedItems']} Übertragung(en) fehlgeschlagen. Erfolgreiche Teilübertragungen bleiben erhalten.")
    return result


def friendly_error(stderr: str) -> str:
    lower = stderr.lower()
    if any(x in lower for x in ("not authenticated", "not logged in", "authentication required", "auth login", "log in first", "login first")):
        return "Bitte zuerst über den Browser bei Proton anmelden."
    if any(x in lower for x in ("libsecret", "secret service", "keyring", "credentials store", "dbus")):
        return "Der Linux-Schlüsselbund ist nicht erreichbar. GNOME Keyring oder KWallet entsperren und Secret-Service-Zugriff prüfen."
    if any(x in lower for x in ("429", "rate limit", "too many requests")):
        return "Proton begrenzt gerade die Anfragen. Bitte später erneut versuchen."
    if any(x in lower for x in ("quota", "insufficient storage")):
        return "Der verfügbare Cloud-Speicher reicht für diese Übertragung nicht aus."
    if "not found" in lower:
        return "Die Datei oder der Ordner wurde nicht gefunden. Bitte aktualisieren."
    if any(x in lower for x in ("network", "fetch failed", "connection", "enotfound", "timeout", "timed out")):
        return "Verbindung zu Proton fehlgeschlagen. Bitte Netzwerk prüfen und erneut versuchen."
    return "Die Proton-CLI hat die Aktion abgebrochen. Bitte Anmeldung, Zugriffsrechte und Netzwerk prüfen. Rohdaten werden zum Schutz von Kontodaten nicht angezeigt."


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
        if self.cancel_event.is_set():
            raise Cancelled("Vorgang abgebrochen; bereits übertragene Dateien bleiben erhalten.")
        env = os.environ.copy()
        if env.get("PROTON_DRIVE_CREDENTIALS_STORE", "keychain") not in {"keychain", "pass"}:
            raise BackendError("NeutronDrive erlaubt nur einen verschlüsselten Schlüsselbund (keychain oder pass).")
        if env.get("PROTON_DRIVE_BASE_URL", "drive-api.proton.me") != "drive-api.proton.me":
            raise BackendError("NeutronDrive verwendet ausschließlich den offiziellen Proton-API-Endpunkt.")
        env.update({"NO_COLOR": "1", "PROTON_DRIVE_LOG_LEVEL": "WARNING", "PROTON_DRIVE_UNSAFE_CACHE": "false"})
        with self._mutex:
            if self.cancel_event.is_set():
                raise Cancelled("Vorgang vor dem Start abgebrochen.")
            lock_path = data_dir() / "backend.lock"
            lock_fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            with os.fdopen(lock_fd, "w") as lock:
                # The GUI, background worker and on-demand filesystem share a
                # credential/cache session. Serialize all short CLI calls, but
                # keep a waiting caller cancellable and bounded.
                lock_deadline = time.monotonic() + min(timeout, 120)
                while True:
                    if self.cancel_event.is_set():
                        raise Cancelled("Warten auf das Backend abgebrochen.")
                    try:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError as exc:
                        if time.monotonic() >= lock_deadline:
                            raise BackendError("Das Backend ist noch beschäftigt. Bitte erneut versuchen.") from exc
                        self.cancel_event.wait(0.05)
                try:
                    process = subprocess.Popen([self.executable, *args], stdin=subprocess.DEVNULL,
                                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                               env=env, start_new_session=True)
                except OSError as exc:
                    raise BackendError("Die Proton-CLI konnte nicht gestartet werden. Architektur und Bibliotheken prüfen.") from exc
                try:
                    return self._collect(process, timeout, on_login, parse)
                finally:
                    if process.poll() is None:
                        with contextlib.suppress(ProcessLookupError):
                            os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=2)
                        except subprocess.TimeoutExpired:
                            with contextlib.suppress(ProcessLookupError):
                                os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                    process.stdout.close()
                    process.stderr.close()

    def _collect(self, process, timeout, on_login, parse):
        buffers = {"out": bytearray(), "err": bytearray()}
        deadline = time.monotonic() + timeout
        login_reported = False
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, "out")
            selector.register(process.stderr, selectors.EVENT_READ, "err")
            while selector.get_map() or process.poll() is None:
                if self.cancel_event.is_set():
                    raise Cancelled("Vorgang abgebrochen; bereits übertragene Dateien bleiben erhalten.")
                if time.monotonic() > deadline:
                    raise BackendError("Zeitlimit erreicht. Der Vorgang wurde beendet; bitte Cloud-Status vor einem erneuten Versuch prüfen.")
                for key, _ in selector.select(0.1):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    buffers[key.data].extend(chunk)
                    if len(buffers[key.data]) > MAX_OUTPUT:
                        raise BackendError("Die Backend-Antwort überschreitet die Größenbegrenzung. Bitte kleinere Ordner verwenden.")
                    if on_login and not login_reported and key.data == "out" and b"\n" in buffers["out"]:
                        try:
                            url = json.loads(buffers["out"].split(b"\n", 1)[0])["signInUrl"]
                            parsed = urlsplit(url)
                            if parsed.scheme != "https" or parsed.hostname != "account.proton.me" or parsed.username is not None or parsed.password is not None or parsed.port not in (None, 443):
                                raise ValueError("Untrusted login URL")
                        except (ValueError, KeyError, TypeError) as exc:
                            raise BackendError("Die Proton-CLI lieferte keine vertrauenswürdige Anmelde-URL.") from exc
                        on_login(url)
                        login_reported = True
        returncode = process.wait()
        if returncode:
            raise BackendError(friendly_error(buffers["err"].decode("utf-8", "replace")))
        if on_login and not login_reported:
            raise BackendError("Die Proton-CLI hat keine Anmeldung bestätigt. Bitte erneut anmelden.")
        output = buffers["out"].decode("utf-8", "replace")
        return checked_json(output) if parse else output.strip()

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
        sources = [local_path(p) for p in paths]
        # The upstream uploader can traverse links. Do not unintentionally send
        # files outside a user-selected folder or read devices/FIFOs.
        for source in sources:
            if source.is_dir():
                for base, dirs, files in os.walk(source, followlinks=False):
                    if self.cancel_event.is_set():
                        raise Cancelled("Upload abgebrochen.")
                    for name in dirs + files:
                        child = Path(base) / name
                        mode = child.lstat().st_mode
                        if not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
                            raise BackendError("Der Upload-Ordner enthält symbolische Links oder Spezialdateien. Bitte diese zuerst ausschließen.")
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
