"""SDK event acknowledgement and recovery tests, using no network or credentials."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from core_contract.backend import Backend, BackendError, Cancelled
from core_contract.events import EventSource, RemoteEvent, validate_batch

ROOT = {"path": "/my-files", "uid": "root-uid", "scopeId": "scope"}
CHANGE = {"type": "node_updated", "treeEventScopeId": "scope", "eventId": "next",
          "nodeUid": "file-uid", "parentNodeUid": "folder-uid", "isTrashed": False}


def batch(*events, cursor="next", reset=False):
    return {"protocol": 1, "type": "batch", "roots": [dict(ROOT)], "events": list(events),
            "cursors": {"scope": cursor} if cursor else {}, "resets": ["scope"] if reset else []}


class FakeBackend:
    def __init__(self, response):
        self.response = response
        self.calls = []
        self.cancelled = threading.Event()
        self.entered = threading.Event()

    def _run(self, args, timeout):
        self.calls.append((args, timeout))
        self.entered.set()
        if isinstance(self.response, BaseException):
            raise self.response
        if callable(self.response):
            return self.response()
        return copy.deepcopy(self.response)

    def cancel(self):
        self.cancelled.set()

    def reset_cancel(self):
        self.cancelled.clear()


class EventTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="neutrondrive-events-test-")
        self.addCleanup(self.temp.cleanup)
        self.state = Path(self.temp.name) / "events"
        self.backend = FakeBackend(batch(CHANGE, reset=True))
        self.source = EventSource(self.backend, self.state)
        self.addCleanup(self.source.stop)

    def test_only_event_endpoint_is_used_and_cursor_is_saved_after_delivery(self):
        received = []
        def accept(event):
            self.assertFalse(self.source.state_file.exists())
            received.append(event)
        self.source.poll_once(accept)
        self.assertEqual([e.kind for e in received], ["ready", "node_updated"])
        self.assertTrue(received[0].reset)
        self.assertEqual(received[1].parent_uid, "folder-uid")
        args, timeout = self.backend.calls[0]
        self.assertEqual(args, ["events", "poll", "--json", "--cursor-json", "{}", "--", "/my-files"])
        self.assertEqual(timeout, 120)
        self.assertEqual(json.loads(self.source.state_file.read_text())["cursors"], {"scope": "next"})
        self.assertEqual(self.source.state_file.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.source.status["state"], "ready")

    def test_callback_failure_replays_old_cursor_and_keeps_durable_state(self):
        self.source.poll_once(lambda event: None)
        original = self.source.state_file.read_bytes()
        self.backend.response = batch({**CHANGE, "eventId": "newer"}, cursor="newer")
        def reject(event):
            raise OSError("queue disk full")
        with self.assertRaises(OSError):
            self.source.poll_once(reject)
        self.assertEqual(self.source.state_file.read_bytes(), original)
        self.source.poll_once(lambda event: None)
        self.assertEqual(json.loads(self.backend.calls[-1][0][4]), {"scope": "next"})
        self.assertEqual(self.source._cursors, {"scope": "newer"})

    def test_persistence_failure_does_not_acknowledge_batch_in_memory(self):
        # A real non-file destination makes the native atomic commit fail.
        self.source._prepare()
        self.source.state_file.mkdir(mode=0o700)
        received = []
        with self.assertRaises(BackendError):
            self.source.poll_once(received.append)
        self.assertTrue(received)
        self.assertEqual(self.source._cursors, {})
        self.assertTrue(self.source.state_file.is_dir())

    def test_restart_uses_cursor_without_reset_and_account_root_change_resets(self):
        self.source.poll_once(lambda event: None)
        again = EventSource(self.backend, self.state)
        again._load()
        self.backend.response = batch(cursor="next")
        events = []
        again.poll_once(events.append)
        self.assertEqual(events, [RemoteEvent(root_uid="root-uid", kind="ready", scope_id="scope")])
        self.assertEqual(json.loads(self.backend.calls[-1][0][4]), {"scope": "next"})
        changed = batch(cursor="new-account", reset=True)
        changed["roots"][0]["uid"] = "new-root"
        self.backend.response = changed
        again.poll_once(events.append)
        self.assertTrue(events[-1].reset)
        self.assertEqual(events[-1].root_uid, "new-root")

    def test_tree_removal_has_no_cursor_and_state_is_restartable(self):
        self.backend.response = batch({"type": "tree_remove", "treeEventScopeId": "scope", "eventId": "none"}, cursor=None, reset=True)
        events = []
        self.source.poll_once(events.append)
        self.assertEqual(events[-1].kind, "tree_remove")
        self.assertTrue(events[-1].reset)
        again = EventSource(self.backend, self.state)
        again._load()
        self.assertEqual(again._cursors, {})

    # Reconnection notification after a native error-state transition is covered
    # in Rust, without a production API that can overwrite scheduler status.

    def test_corrupt_insecure_and_symlink_state_fail_closed(self):
        self.source._prepare()
        self.source.state_file.write_text("invalid json")
        self.source.state_file.chmod(0o600)
        with self.assertRaises(BackendError):
            self.source._load()
        self.assertEqual(self.source.state_file.read_text(), "invalid json")
        self.source.state_file.chmod(0o644)
        with self.assertRaises(BackendError):
            self.source._load()
        self.source.state_file.unlink()
        target = self.state / "other"
        target.write_text("unchanged")
        self.source.state_file.symlink_to(target)
        # Native I/O exposes a sanitized BackendError across the C ABI.
        with self.assertRaises(BackendError):
            self.source._load()
        self.assertEqual(target.read_text(), "unchanged")

    def test_json_null_cursor_is_corruption_not_a_missing_file(self):
        self.source._prepare()
        self.source.state_file.write_text("null")
        self.source.state_file.chmod(0o600)
        with self.assertRaises(BackendError): self.source._load()
        self.assertEqual(self.source.state_file.read_text(), "null")

    def test_cancellation_during_callback_never_advances_cursor(self):
        def accept(event):
            self.source.stop()
        with self.assertRaises(Cancelled):
            self.source.poll_once(accept)
        self.assertEqual(self.source._cursors, {})
        self.assertFalse(self.source.state_file.exists())

    def test_background_error_is_redacted_and_stop_interrupts_wait(self):
        self.backend.response = BackendError("secret-access-token")
        received = []
        ready = threading.Event()
        def accept(event):
            received.append(event)
            ready.set()
        self.source.start(accept)
        self.assertTrue(ready.wait(2))
        self.assertEqual(received[-1].kind, "error")
        self.assertFalse(received[-1].reset)
        self.assertNotIn("secret-access-token", str(self.source.status) + str(received))
        self.assertGreaterEqual(self.source.status["retry_seconds"], 30)
        before = time.monotonic()
        self.source.stop()
        self.source.stop()
        self.assertLess(time.monotonic() - before, 1)
        self.assertFalse(self.source.running)
        self.assertFalse(self.source.state_file.exists())

    def test_busy_backend_does_not_report_remote_failure(self):
        self.backend.response = BackendError("Das Backend ist noch beschäftigt. Bitte erneut versuchen.")
        received = []
        self.source.start(received.append)
        deadline = time.monotonic() + 2
        while self.source.status["state"] != "busy" and time.monotonic() < deadline:
            time.sleep(0.005)
        self.assertEqual(self.source.status["state"], "busy")
        self.assertEqual(received, [])

    def test_native_poll_uses_sdk_transport_and_commits_after_delivery(self):
        executable = Path(self.temp.name) / "synthetic-sdk"
        calls = Path(self.temp.name) / "sdk-arguments.json"
        executable.write_text(f"#!{sys.executable}\nimport json, sys\n"
                              f"from pathlib import Path\nPath({str(calls)!r}).write_text(json.dumps(sys.argv[1:]))\n"
                              f"print({json.dumps(batch(CHANGE))!r})\n")
        executable.chmod(0o700)
        source = EventSource(Backend(str(executable)), self.state)
        self.addCleanup(source.stop)
        received = []
        def accept(event):
            self.assertFalse(source.backend._mutex.locked())
            self.assertFalse(source.state_file.exists())
            received.append(event)
        with mock.patch.dict(os.environ, {"XDG_STATE_HOME": str(Path(self.temp.name) / "runtime")}):
            source.poll_once(accept)
        self.assertEqual([event.kind for event in received], ["ready", "node_updated"])
        self.assertEqual(json.loads(calls.read_text()),
            ["events", "poll", "--json", "--cursor-json", "{}", "--", "/my-files"])
        self.assertEqual(json.loads(source.state_file.read_text())["cursors"], {"scope": "next"})

    def test_native_stop_cancels_and_reaps_inflight_sdk(self):
        import shlex
        executable = Path(self.temp.name) / 'slow-sdk'
        pid_file = Path(self.temp.name) / 'sdk.pid'
        executable.write_text('#!/bin/sh\nprintf "%s" "$$" > ' + shlex.quote(str(pid_file)) + '\nexec /bin/sleep 30\n')
        executable.chmod(0o700)
        source = EventSource(Backend(str(executable)), self.state)
        self.addCleanup(source.stop)
        with mock.patch.dict(os.environ, XDG_STATE_HOME=str(Path(self.temp.name) / 'isolated-runtime')):
            source.start(lambda event: None)
            deadline = time.monotonic() + 3
            while (not pid_file.exists() or not pid_file.read_text()) and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(pid_file.exists())
            pid = int(pid_file.read_text())
            source.stop(timeout=2)
            self.assertFalse(source.running)
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
        self.assertEqual(source._cursors, {})
        self.assertFalse(source.state_file.exists())

    def test_failed_thread_start_can_be_retried(self):
        with mock.patch('core_contract.events.threading.Thread.start', side_effect=RuntimeError('start failure')):
            with self.assertRaisesRegex(RuntimeError, 'start failure'):
                self.source.start(lambda event: None)
        self.assertFalse(self.source.running)
        received = threading.Event()
        self.source.start(lambda event: received.set())
        self.assertTrue(received.wait(2))
        self.source.stop()

    def test_real_backend_has_independent_cancellation(self):
        backend = Backend("/bin/true")
        source = EventSource(backend, self.state)
        self.assertIsNot(source.backend, backend)
        self.assertIs(source.backend._mutex, backend._mutex)
        source.stop()
        self.assertFalse(backend.cancel_event.is_set())

    def test_scope_and_batch_validation_rejects_unknown_or_partial_data(self):
        invalid = []
        for key, value in [("protocol", 2), ("events", None), ("cursors", {}), ("resets", ["foreign"])]:
            item = batch(CHANGE); item[key] = value; invalid.append(item)
        item = batch({**CHANGE, "type": "future_event"}); invalid.append(item)
        item = batch({**CHANGE, "treeEventScopeId": "foreign"}); invalid.append(item)
        item = batch({**CHANGE, "isTrashed": "false"}); invalid.append(item)
        item = batch({k: v for k, v in CHANGE.items() if k != "nodeUid"}); invalid.append(item)
        item = batch(CHANGE); item["roots"][0]["path"] = "/trash"; invalid.append(item)
        for value in invalid:
            with self.subTest(value=value):
                with self.assertRaises(BackendError):
                    validate_batch(value, ("/my-files",))
        with self.assertRaises(BackendError):
            EventSource(self.backend, self.state, roots=("/trash",))
        with self.assertRaises(BackendError):
            EventSource(self.backend, self.state, roots=("/my-files", "/my-files"))


if __name__ == "__main__":
    unittest.main()

class ComputerEventTests(unittest.TestCase):
    def test_dynamic_roots_use_separate_cursor_state(self):
        roots=['/my-files']
        with tempfile.TemporaryDirectory() as folder:
            backend=FakeBackend(batch(CHANGE))
            source=EventSource(backend,Path(folder),roots_provider=lambda:tuple(roots))
            source.poll_once(lambda event:None)
            old=source.state_file
            roots.append('/devices/Linux')
            value=batch(CHANGE)
            value['roots'].append({'path':'/devices/Linux','uid':'computer','scopeId':'device-scope'})
            value['cursors']['device-scope']='device-cursor'
            backend.response=value
            received=[]
            source.poll_once(received.append)
            self.assertNotEqual(old,source.state_file)
            self.assertTrue(old.exists())
            self.assertIn('/devices/Linux',backend.calls[-1][0])
            self.assertEqual(source._cursors['device-scope'],'device-cursor')
            self.assertTrue(any(event.reset for event in received))
