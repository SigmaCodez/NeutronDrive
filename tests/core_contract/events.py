"""Acknowledged Drive events through the source-built SDK bridge.

Only the SDK event endpoint is polled, at its foreground cadence. There are no
periodic remote directory scans here. A consumer must durably queue an event
before its callback returns; only then is the batch cursor committed locally.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import threading
from typing import Callable

from .backend import Backend, BackendError, Cancelled
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
    """Scoped callback/DTO binding; Rust owns cursors, status and scheduling."""
    def __init__(self, backend, state_dir: Path, roots=("/my-files",), interval=30, roots_provider=None):
        self.backend = Backend(backend.executable) if isinstance(backend, Backend) else backend
        if isinstance(backend, Backend):
            self.backend._mutex = backend._mutex
        self.roots_provider = roots_provider
        self._callback = None
        self._thread = None
        self._id = rust_core.backend_value('event_source', operation={
            'action': 'create', 'state_dir': str(state_dir), 'roots': list(roots),
            'interval': float(interval), 'dynamic_roots': roots_provider is not None})

    def _call(self, action, callback=None, **arguments):
        errors = []
        receiver = callback or self._callback
        def event(value):
            if value['kind'] != 'host_request':
                raise BackendError('Unexpected native event-source notification')
            try:
                method, args = value['method'], value['arguments']
                if method == 'event_roots':
                    result = list(self.roots_provider())
                elif method == 'event_accept':
                    if receiver is None:
                        raise BackendError('Ereignisquelle benötigt einen Empfänger.')
                    receiver(RemoteEvent(**args))
                    result = None
                elif method == 'events_query':
                    result = self.backend._run(['events', 'poll', '--json', '--cursor-json',
                                               json.dumps(args['cursors']), '--', *args['paths']], timeout=120)
                else:
                    raise BackendError('Unsupported event-source host service')
            except BaseException as error:
                if action != 'run':
                    errors.append(error)
                rust_core.backend_value('host_failure', id=value['id'], message=str(error),
                                        cancelled=isinstance(error, Cancelled))
            else:
                rust_core.backend_value('host_reply', id=value['id'], value=result)
        try:
            return rust_core.backend_value('event_source', operation={
                'action': action, 'id': self._id, **arguments},
                executable=self.backend.executable if isinstance(self.backend, Backend) else None,
                host_delivery=True, on_event=event)
        except BaseException:
            if errors:
                raise errors[0]
            raise

    def _snapshot(self): return self._call('snapshot')

    @property
    def status(self): return self._snapshot()['status']

    @property
    def error(self): return self.status['error']

    @property
    def running(self): return self._snapshot()['running']

    @property
    def roots(self): return tuple(self._snapshot()['roots'])

    @property
    def interval(self): return self._snapshot()['interval']

    @property
    def state_file(self): return Path(self._snapshot()['file'])

    @property
    def _cursors(self): return self._snapshot()['cursors']

    def _prepare(self): return self._call('prepare')

    def _load(self): return self._call('load')

    def poll_once(self, callback=None):
        if callback is None and self._callback is None:
            raise BackendError('Ereignisquelle benötigt einen Empfänger.')
        return self._call('poll', callback=callback)

    def start(self, callback):
        if not self._call('start'):
            return
        self._callback = callback
        try:
            self.backend.reset_cancel()
            self._thread = threading.Thread(target=self._run, name='neutrondrive-events', daemon=True)
            self._thread.start()
        except BaseException:
            self._call('abort_start')
            raise

    def _run(self):
        self._call('run')

    def stop(self, timeout=5):
        self._call('stop', timeout=0.0)
        self.backend.cancel()
        self._call('stop', timeout=float(timeout))

    def __del__(self):
        if hasattr(self, '_id'):
            try:
                rust_core.backend_value('event_source', operation={'action': 'forget', 'id': self._id})
            except Exception:
                pass
