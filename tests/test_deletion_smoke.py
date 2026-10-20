"""Live-harness regression tests: fake time/backend, no account or network."""
import importlib.machinery
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from test_sync import FakeBackend


loader = importlib.machinery.SourceFileLoader(
    "deletion_smoke", str(Path(__file__).resolve().parents[1] / "scripts/smoke-deletions-live"))
spec = importlib.util.spec_from_loader(loader.name, loader)
smoke = importlib.util.module_from_spec(spec)
loader.exec_module(smoke)

BUSY = "Das Backend ist noch beschäftigt. Bitte erneut versuchen."


class Clock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class DeletionSmokeTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.clock_patch = patch.object(smoke, "time", self.clock)
        self.clock_patch.start()
        self.addCleanup(self.clock_patch.stop)
        self.status = Mock(return_value=({}, {"state": "idle", "pending": False, "last_sync": 20}))

    def test_fresh_complete_cycle_required_after_mutation(self):
        current = self.status()[1]
        self.assertTrue(smoke.completed_after(current, 19))
        for changes in ({"last_sync": 19}, {"last_sync": None}, {"state": "syncing"},
                        {"pending": True}, {"state": "conflicts"}, {"state": "offline"}):
            with self.subTest(changes=changes):
                self.assertFalse(smoke.completed_after(dict(current, **changes), 19))
        conflict = dict(current, state="conflicts")
        self.assertTrue(smoke.completed_after(conflict, 19, allow_conflicts=True))
        self.assertFalse(smoke.completed_after(conflict, 20, allow_conflicts=True))

    def test_busy_read_retries_within_original_deadline(self):
        read = Mock(side_effect=[smoke.BackendError(BUSY), False, "confirmed"])
        retried = Mock()
        self.assertEqual(smoke.wait_until(read, self.status, 5, "timeout", retry_busy=True,
                                         on_retry=retried), "confirmed")
        self.assertEqual(read.call_count, 3)
        retried.assert_called_once_with()
        self.assertEqual(self.clock.now, 2)

        read = Mock(side_effect=smoke.BackendError(BUSY))
        with self.assertRaisesRegex(smoke.CheckFailed, "fixed deadline"):
            smoke.wait_until(read, self.status, 5, "fixed deadline", interval=2, retry_busy=True)
        self.assertEqual(self.clock.now, 7)
        self.assertEqual(read.call_count, 3)

    def test_no_implicit_mutation_or_nonbusy_error_retry(self):
        for retry_busy, message in ((False, BUSY), (True, "Verbindung zu Proton fehlgeschlagen.")):
            with self.subTest(retry_busy=retry_busy):
                operation = Mock(side_effect=smoke.BackendError(message))
                with self.assertRaises(smoke.BackendError):
                    smoke.wait_until(operation, self.status, 5, "timeout", retry_busy=retry_busy)
                operation.assert_called_once_with()
                self.assertEqual(self.clock.now, 0)

    def test_blocking_read_cannot_extend_phase_deadline(self):
        def late_read():
            self.clock.now += 6
            return True
        with self.assertRaisesRegex(smoke.CheckFailed, "deadline exceeded"):
            smoke.wait_until(late_read, self.status, 5, "deadline exceeded", retry_busy=True)
        self.assertEqual(self.clock.now, 6)

    def test_unexpected_conflicts_stop_before_observer_read(self):
        self.status.return_value[1]["state"] = "conflicts"
        read = Mock(return_value=True)
        with self.assertRaisesRegex(smoke.CheckFailed, "unexpected deletion conflict"):
            smoke.wait_until(read, self.status, 5, "timeout", retry_busy=True)
        read.assert_not_called()
        self.assertTrue(smoke.wait_until(read, self.status, 5, "timeout", allow_conflicts=True))

    def test_diagnostics_record_only_safe_details_and_command_names(self):
        backend = smoke.BoundedBackend("/not/executed")
        arguments = ["filesystem", "list", "--json", "--", "/my-files/private-name"]
        with patch.object(smoke.Backend, "_run", return_value={"private": "not logged"}) as run:
            backend._run(arguments, timeout=240)
            run.assert_called_once_with(arguments, timeout=60)
        self.assertEqual(backend.last_call, {"operation": "filesystem list", "outcome": "success", "seconds": 0})
        with patch.object(smoke.Backend, "_run", side_effect=smoke.BackendError(BUSY)):
            with self.assertRaises(smoke.BackendError):
                backend._run(arguments)
        self.assertEqual(backend.last_call, {"operation": "filesystem list", "outcome": "error",
                                            "error_type": "BackendError", "seconds": 0})
        self.assertEqual(smoke.error_details(smoke.BackendError(BUSY)),
                         {"error_type": "BackendError", "error_detail": BUSY})
        self.assertEqual(smoke.error_details(OSError("potentially private exception text")),
                         {"error_type": "OSError"})

    def test_remote_folder_retention_check_rejects_partial_or_changed_trees(self):
        for change in ("none", "missing-empty", "extra-file", "edited-file"):
            with self.subTest(change=change):
                backend = FakeBackend()
                folder = backend.add("folder")
                backend.add("folder/nested")
                backend.add("folder/empty")
                backend.add("folder/nested/child.txt", b"synthetic")
                before = smoke.remote_folder_snapshot(backend, folder, b"synthetic")
                self.assertEqual(set(before), {"", "empty", "nested", "nested/child.txt"})
                if change == "none":
                    self.assertEqual(smoke.remote_folder_snapshot(backend, folder, b"synthetic"), before)
                    continue
                if change == "missing-empty":
                    backend.nodes.pop("folder/empty")
                elif change == "extra-file":
                    backend.add("folder/extra.txt", b"synthetic")
                else:
                    backend.add("folder/nested/child.txt", b"changed")
                with self.assertRaises(smoke.CheckFailed):
                    smoke.remote_folder_snapshot(backend, folder, b"synthetic")

    def test_local_folder_retention_check_keeps_identity_and_complete_contents(self):
        with tempfile.TemporaryDirectory(prefix="neutrondrive-smoke-test-") as temporary:
            folder = Path(temporary) / "folder"
            (folder / "nested").mkdir(parents=True)
            (folder / "empty").mkdir()
            (folder / "nested/child.txt").write_bytes(b"synthetic")
            before = smoke.local_folder_snapshot(folder, b"synthetic")
            self.assertEqual(smoke.local_folder_snapshot(folder, b"synthetic"), before)
            (folder / "empty").rmdir()
            with self.assertRaises(smoke.CheckFailed):
                smoke.local_folder_snapshot(folder, b"synthetic")
            (folder / "empty").mkdir()
            (folder / "nested/child.txt").write_bytes(b"changed")
            with self.assertRaises(smoke.CheckFailed):
                smoke.local_folder_snapshot(folder, b"synthetic")


if __name__ == "__main__":
    unittest.main()
