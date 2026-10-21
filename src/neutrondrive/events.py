"""Acknowledged Drive events through the source-built SDK bridge.

Only the SDK event endpoint is polled, at its foreground cadence. There are no
periodic remote directory scans here. A consumer must durably queue an event
before its callback returns; only then is the batch cursor committed locally.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import random
import stat
import tempfile
import threading
import time
from typing import Callable

from .backend import Backend, BackendError, Cancelled, validate_remote
from . import rust_core


@dataclass(frozen=True)
class RemoteEvent:
    root_uid: str | None = None
    node_uid: str | None = None
    parent_uid: str | None = None
    kind: str = "unknown"
    reset: bool = False
    scope_id: str = ""
    event_id: str = ""
    message: str = ""
    is_trashed: bool = False


def validate_batch(value, paths: tuple[str, ...]) -> dict:
    return rust_core.call("validate_events", value=value, paths=paths)


class EventSource:
    def __init__(self, backend, state_dir: Path, roots=("/my-files",), interval=30):
        self.roots = tuple(roots)
        if not self.roots or len(self.roots) > 64 or len(set(self.roots)) != len(self.roots):
            raise BackendError("Ein bis 64 eindeutige Ereignis-Wurzeln erforderlich.")
        for root in self.roots:
            validate_remote(root)
            if root != "/my-files" and not root.startswith("/my-files/"):
                raise BackendError("Automatische Drive-Ereignisse unterstützen derzeit Meine Dateien.")
        self.interval = max(30, float(interval))
        self.state_dir = Path(state_dir)
        key = hashlib.sha256(json.dumps(self.roots).encode()).hexdigest()[:24]
        self.state_file = self.state_dir / ("events-" + key + ".json")
        # Own cancellation, same interprocess backend lock: stopping the event
        # source must never cancel a user's upload on the GUI adapter.
        self.backend = Backend(backend.executable) if isinstance(backend, Backend) else backend
        if isinstance(backend, Backend):
            self.backend._mutex = backend._mutex
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._callback: Callable[[RemoteEvent], None] | None = None
        self._mutex = threading.RLock()
        self._status = {"state": "stopped", "error": "", "last_success": None, "retry_seconds": 0}
        self._cursors: dict[str, str] = {}
        self._known_roots: list[dict] = []
        self._announced = False

    @property
    def status(self) -> dict:
        with self._mutex:
            return dict(self._status)

    @property
    def error(self) -> str:
        return self.status["error"]

    @property
    def running(self) -> bool:
        return bool(self._thread and self._thread.is_alive())

    def _set_status(self, **fields):
        with self._mutex:
            self._status.update(fields)

    def _prepare(self):
        self.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        info = self.state_dir.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise BackendError("Ereigniszustand benötigt ein privates Verzeichnis mit Modus 0700.")

    def _load(self):
        self._prepare()
        self._cursors = {}
        self._known_roots = []
        try:
            fd = os.open(self.state_file, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except FileNotFoundError:
            return
        try:
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_size > 1024 * 1024
                    or info.st_uid != os.getuid() or info.st_mode & 0o077):
                raise BackendError("Unsichere Ereignis-Cursordatei.")
            with os.fdopen(fd, "r", closefd=False) as stream:
                value = json.load(stream)
            if not isinstance(value, dict) or not isinstance(value.get("removed_scopes", []), list):
                raise BackendError("Ungültige Ereignis-Cursordatei.")
            removed = value.get("removed_scopes", [])
            batch = validate_batch({**value, "type": "batch", "events": [
                {"type": "tree_remove", "treeEventScopeId": scope, "eventId": "none"} for scope in removed
            ], "resets": removed}, self.roots)
            self._cursors = batch["cursors"]
            self._known_roots = batch["roots"]
        except (ValueError, TypeError) as exc:
            raise BackendError("Ereignis-Cursordatei ist beschädigt; sie wurde nicht zurückgesetzt.") from exc
        finally:
            os.close(fd)

    def _save(self, batch):
        self._prepare()
        fd, name = tempfile.mkstemp(prefix="event-cursor-", suffix=".tmp", dir=self.state_dir)
        try:
            with os.fdopen(fd, "w") as stream:
                json.dump({"protocol": 1, "roots": batch["roots"], "cursors": batch["cursors"],
                           "removed_scopes": sorted({event["treeEventScopeId"] for event in batch["events"]
                                                     if event["type"] == "tree_remove"})}, stream)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(name, self.state_file)
            directory = os.open(self.state_dir, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(name):
                os.unlink(name)

    def poll_once(self, callback: Callable[[RemoteEvent], None] | None = None):
        """One bounded SDK event query. Callbacks must durably accept before return."""
        callback = callback or self._callback
        if callback is None:
            raise BackendError("Ereignisquelle benötigt einen Empfänger.")
        batch = validate_batch(self.backend._run([
            "events", "poll", "--json", "--cursor-json", json.dumps(self._cursors), "--", *self.roots,
        ], timeout=120), self.roots)
        if self._stop.is_set():
            raise Cancelled("Ereignisquelle angehalten.")
        deliveries = rust_core.call("event_notifications", batch=batch, known_roots=self._known_roots,
                                    announced=self._announced, ready=self.status["state"] == "ready")
        for delivery in deliveries:
            if self._stop.is_set():
                raise Cancelled("Ereignisquelle angehalten.")
            callback(RemoteEvent(**delivery))
        # Callback failure, cancellation or persistence failure leaves the old
        # cursor in memory. Repeated delivery is intentional and idempotent.
        if self._stop.is_set():
            raise Cancelled("Ereignisquelle angehalten.")
        self._save(batch)
        self._cursors = dict(batch["cursors"])
        self._known_roots = list(batch["roots"])
        self._announced = True
        self._set_status(state="ready", error="", last_success=time.time(), retry_seconds=self.interval)
        return batch

    def start(self, callback: Callable[[RemoteEvent], None]):
        with self._mutex:
            if self.running:
                return
            self._load()
            self._callback = callback
            self._announced = False
            self._stop.clear()
            self.backend.reset_cancel()
            self._set_status(state="starting", error="")
            self._thread = threading.Thread(target=self._run, name="neutrondrive-events", daemon=True)
            self._thread.start()

    def _run(self):
        failures = 0
        while not self._stop.is_set():
            delay = self.interval
            try:
                self.poll_once()
                failures = 0
            except Cancelled:
                if self._stop.is_set():
                    break
                self._set_status(state="waiting", error="")
            except Exception as exc:
                if self._stop.is_set():
                    break
                if isinstance(exc, BackendError) and any(message in str(exc) for message in (
                        "andere NeutronDrive-Instanz", "Backend ist noch beschäftigt")):
                    self._set_status(state="busy", error="", retry_seconds=delay)
                else:
                    retry = rust_core.call("event_retry", interval=self.interval, failures=failures, jitter=random.random())
                    delay, failures = retry["delay"], retry["failures"]
                    message = "Drive-Ereignisse sind vorübergehend nicht verfügbar; ausstehende Änderungen bleiben erhalten."
                    self._set_status(state="error", error=message, retry_seconds=delay)
                    try:
                        if self._callback:
                            self._callback(RemoteEvent(kind="error", message=message))
                    except Exception:
                        pass  # Never turn a failed durable queue into a cursor ACK.
            self._stop.wait(delay)
        self._set_status(state="stopped", retry_seconds=0)

    def stop(self, timeout=5):
        self._stop.set()
        self.backend.cancel()
        thread = self._thread
        if thread and thread is not threading.current_thread():
            thread.join(timeout)
        if thread and thread.is_alive():
            self._set_status(state="stopping")
        else:
            self._set_status(state="stopped", retry_seconds=0)
