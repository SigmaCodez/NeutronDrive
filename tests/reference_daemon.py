"""Test-only orchestration harness for injected cloud, watcher and engine scenarios.

Not shipped with the application. Production daemon ownership and thread
coordination live in core/src/daemon.rs; runtime.rs and NativeDaemonEntryTests
exercise that implementation directly. This harness retains the existing
fault-injection scenarios around the native sync/cache/control boundaries.
"""
from __future__ import annotations
from dataclasses import asdict, dataclass, field
from pathlib import Path
import os
import random
import threading
import time
import uuid
from typing import Any, Callable
from core_contract import rust_core
from core_contract.backend import Backend, BackendError, Cancelled
from core_contract.sync import SyncEngine, SyncError
from core_contract.background import (Job, JobStore, RemoteTreeCache, BackgroundError,
    _Blocked, _validate_local, _write_json)

class InotifyWatcher:
    """Thread/callback binding; Rust owns registration, polling and descriptor."""

    def __init__(self, root: Path, callback: Callable[[str | None, str], None]):
        self.root, self.callback = Path(root), callback
        self._stop = threading.Event()
        self._thread = None
        self._handle = rust_core.backend_value("watcher_create", root=list(os.fsencode(self.root)))

    def start(self) -> None:
        if self._thread is not None:
            raise BackgroundError("Dateiüberwachung wurde bereits gestartet.")
        self._thread = threading.Thread(target=self._run, name="neutrondrive-inotify", daemon=True)
        self._thread.start()

    def _event(self, event):
        relative = event["relative"]
        self.callback(None if relative is None else os.fsdecode(bytes(relative)), event["reason"])

    def _run(self) -> None:
        try:
            rust_core.backend_value("watcher_run", id=self._handle, cancel=self._stop, on_event=self._event)
        except (OSError, ValueError, BackendError):
            if not self._stop.is_set(): self.callback(None, "watch-error")
        finally:
            self.stop()

    def stop(self) -> None:
        self._stop.set()
        rust_core.backend_value("watcher_stop", id=self._handle)

    def join(self) -> None:
        if self._thread and self._thread is not threading.current_thread():
            self._thread.join(timeout=1.0)

    def __del__(self):
        if hasattr(self, "_handle"):
            try: self.stop()
            except Exception: pass


def _matches_snapshot(root: Path, relative: str, snapshot: dict[str, Any]) -> bool:
    """Native stat-only proof; uncertainty always schedules reconciliation."""
    return rust_core.backend_value("echo_matches", root=str(root), relative=relative,
                                   snapshot=snapshot)


def _echo_snapshot(plan: Any, result: Any) -> dict[str, Any]:
    """Suppress only settled writes, retaining edits that arrived during a run."""
    from core_contract.sync import _Entry
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
    snapshot_ready: bool = False


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
        self._server: int | None = None
        self._server_thread: threading.Thread | None = None
        self._event_status: dict[str, Any] = {"state": "starting", "error": ""}
        self.socket_path = self.state_dir / "control.sock"
        if len(os.fsencode(self.socket_path)) > 106:
            raise BackgroundError("Dienstpfad zu lang für den lokalen Steuerungssocket.")

    def _status(self) -> dict[str, Any]:
        now = time.monotonic()
        return {"version": 1, "running": not self._stopping and self._server is not None,
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
            if relative is not None and runtime.state != "syncing" and runtime.snapshot_ready:
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
                jobs = [dict(id=runtime.job.id, paused=runtime.job.paused,
                    remote_uid=runtime.job.remote_uid, account_root_uid=runtime.job.account_root_uid,
                    known_uids=runtime.known_uids) for runtime in active]
            from core_contract.backend import service_call
            decision = service_call(self.backend, 'event_ancestry', dict(
                event={key: event.get(key) for key in ('root_uid', 'node_uid', 'parent_uid')}, jobs=jobs))
            if decision['cancelled'] and self._stopping:
                return
            affected = [runtime for runtime in active if runtime.job.id in decision['affected']]
            ancestor = decision['ancestor']
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

    def _control_call(self, action: str, job_id=None, *, reload_only=False):
        def request(value):
            if value.get("kind") != "host_request" or value.get("method") != "daemon_state":
                raise BackgroundError("Unexpected native daemon request")
            args = value["arguments"]
            operation, identity = args["action"], args.get("id")
            runtime = self._jobs.get(identity)
            result = None
            try:
                if operation == "jobs": result = self.store.list()
                elif operation == "snapshot":
                    result = {"current": self._current, "jobs": {key: {
                        "job": item.job, "watcher": item.watcher is not None,
                    } for key, item in self._jobs.items()}}
                elif operation == "status": result = self._status()
                elif operation == "persist": self._persist()
                elif operation == "notify": self._condition.notify_all()
                elif operation == "stop": self._stopping = True
                elif operation == "cancel": self.backend.cancel()
                elif operation == "remove": self._jobs.pop(identity)
                elif operation == "update_job": self.store.update(identity, paused=args["paused"])
                elif operation == "upsert":
                    job = Job(**args["job"])
                    self._jobs.setdefault(job.id, _Runtime(job)).job = job
                elif operation == "patch":
                    for key, item in args["fields"].items(): setattr(runtime, key, item)
                elif operation == "stop_watcher":
                    if runtime.watcher: runtime.watcher.stop()
                    runtime.watcher = None
                elif operation == "watch": self._watch(runtime)
                elif operation == "invalidate": self.remote_cache.invalidate_tree(args["root"])
                elif operation == "schedule": self._schedule(runtime, args["reason"], immediate=True)
                else: raise BackgroundError("Unknown native daemon operation")
            except Exception as error:
                rust_core.backend_value("host_failure", id=value["id"], message=str(error), cancelled=isinstance(error, Cancelled))
            else:
                rust_core.backend_value("host_reply", id=value["id"], value=result)
        try:
            return rust_core.backend_value("daemon_control", action=action, job_id=job_id,
                reload_only=reload_only, on_event=request)
        except BackendError as error:
            raise BackgroundError(str(error)) from error

    def _reload(self) -> None:
        self._control_call("reload", reload_only=True)

    def command(self, action: str, job_id: str | None = None) -> dict[str, Any]:
        with self._condition:
            return self._control_call(action, job_id)

    def _resolve(self, runtime: _Runtime) -> Job:
        from core_contract.backend import service_call
        resolved = service_call(self.backend, "job_roots", {
            "jobs": [asdict(job) for job in self.store.list()], "selected": runtime.job.id,
        })
        if resolved.get("blocked"):
            raise _Blocked(resolved["blocked"])
        if resolved.get("cancelled"):
            raise Cancelled("Sync-Job wurde pausiert oder entfernt.")
        pinned = [Job(**entry) for entry in resolved["jobs"]]
        job = next(entry for entry in pinned if entry.id == runtime.job.id)
        # Validate the entire set before canonicalizing persisted paths: aliases
        # must never leave a half-updated, overlapping configuration behind.
        pinned = [self.store._pin(entry.id, entry.remote_uid, entry.account_root_uid, entry.remote_root) for entry in pinned]
        with self._condition:
            for entry in pinned:
                if entry.id in self._jobs:
                    self._jobs[entry.id].job = entry
        return job

    def _transition(self, runtime: _Runtime, action: str, **arguments) -> None:
        fields = ("state", "pending", "due", "first_dirty", "paths", "reasons", "known_uids",
                  "failures", "last_sync", "error", "conflicts", "skipped", "completed",
                  "rebuild_watch", "local_snapshot", "snapshot_ready")
        value = rust_core.backend_value("daemon_transition",
            runtime={key: getattr(runtime, key) for key in fields},
            outcome={"action": action, **arguments}, context={
                "paused": runtime.job.paused, "stopping": self._stopping,
                "root": runtime.job.local_root, "now": time.monotonic(), "wall": time.time(),
                "debounce": self.debounce, "retry_base": self.retry_base,
                "retry_max": self.retry_max, "jitter": random.uniform(0.9, 1.1),
            })
        from core_contract.sync import _Entry
        for key, item in value.items():
            if key in {"paths", "reasons", "known_uids"}:
                item = set(item)
            elif key == "local_snapshot":
                item = {path: _Entry(**entry) for path, entry in item.items()}
            setattr(runtime, key, item)

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
                self._transition(runtime, "complete", previous=plan._local, result=result)
                if runtime.rebuild_watch and not runtime.job.paused:
                    self._watch(runtime)
        except Cancelled:
            with self._condition:
                self._transition(runtime, "cancelled")
        except _Blocked as error:
            with self._condition:
                self._transition(runtime, "blocked", error=str(error))
        except (BackendError, SyncError, OSError) as error:
            with self._condition:
                self._retry(runtime, str(error))
        except Exception:
            with self._condition:
                self._transition(runtime, "blocked", error="Unerwarteter Dienstfehler; Job bleibt zum Schutz der Daten angehalten.")
        finally:
            with self._condition:
                self._transition(runtime, "finalize")
                self._current = None
                self._persist()
                self._condition.notify_all()

    def _retry(self, runtime: _Runtime, error: str) -> None:
        self._transition(runtime, "retry", error=error)

    def _serve(self) -> None:
        def event(value):
            if value.get("kind") != "host_request" or value.get("method") != "daemon_command":
                raise BackgroundError("Unexpected native service request")
            try:
                request = value["arguments"]
                result = self.command(request["action"], request.get("job_id"))
            except Exception as error:
                rust_core.backend_value("host_failure", id=value["id"],
                    message=str(error) if isinstance(error, BackendError) else "Ungültige Steuerungsanfrage.", cancelled=False)
            else:
                rust_core.backend_value("host_reply", id=value["id"], value=result)
        try:
            rust_core.backend_value("control_server", operation={"action": "serve", "id": self._server}, on_event=event)
        except BackendError:
            pass

    def run(self) -> int:
        try:
            self._server = rust_core.backend_value("control_server", operation={"action": "create", "state": str(self.state_dir)})
        except BackendError as error:
            raise BackgroundError(str(error)) from error
        try:
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
                        self._transition(next_runtime, "begin")
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
                if self._server is not None:
                    rust_core.backend_value("control_server", operation={"action": "stop", "id": self._server})
                if self._server_thread and self._server_thread is not threading.current_thread():
                    self._server_thread.join(timeout=5)
                try:
                    with self._condition:
                        self._persist()
                finally:
                    if self._server is not None:
                        rust_core.backend_value("control_server", operation={"action": "release", "id": self._server})
                        self._server = None
