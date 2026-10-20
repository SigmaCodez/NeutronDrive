"""Daemon tests use local sockets/files and a fake cloud; never a Proton session."""
from dataclasses import asdict, replace
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from neutrondrive.backend import BackendError, Cancelled
from neutrondrive.background import (BackgroundDaemon, BackgroundError, Controller,
                                    InotifyWatcher, JobStore, RemoteTreeCache, _echo_snapshot)
from neutrondrive.sync import SyncEngine
from test_sync import FakeBackend


def eventually(predicate, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.01)
    raise AssertionError("Expected daemon state was not reached")


class Cloud(FakeBackend):
    def __init__(self):
        super().__init__()
        self.plans = 0
        self.executions = 0
        self.fail_info = False
        self.after_execute = None
        self.after_plan = None
        self.listings = []

    def reset_cancel(self):
        self.cancel_event.clear()

    def cancel(self):
        self.cancel_event.set()

    def info(self, path):
        if self.fail_info:
            raise BackendError("Offline test")
        return super().info(path)

    def list_nodes(self, path):
        self.listings.append(path)
        return super().list_nodes(path)

    def add(self, relative, data=None, **overrides):
        parent = str(Path(relative).parent)
        parent = "" if parent == "." else parent
        overrides.setdefault("raw", {"parentUid": self.nodes[parent].uid})
        return super().add(relative, data, **overrides)


class ObservedEngine(SyncEngine):
    def plan(self, *args):
        getattr(self.backend, "backend", self.backend).plans += 1
        plan = super().plan(*args)
        if self.backend.after_plan:
            self.backend.after_plan(plan)
        return plan

    def execute(self, plan):
        getattr(self.backend, "backend", self.backend).executions += 1
        result = super().execute(plan)
        if self.backend.after_execute:
            self.backend.after_execute(plan, result)
        return result


class Events:
    def __init__(self):
        self.callback = None
        self.stopped = False

    def start(self, callback):
        self.callback = callback

    def stop(self):
        self.stopped = True

    def emit(self, **event):
        return self.callback(event)


class QuietWatcher:
    def __init__(self, root, callback):
        self.root, self.callback = root, callback
        self.stopped = False

    def start(self):
        pass

    def stop(self):
        self.stopped = True


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.local = self.base / "local"
        self.local.mkdir()
        self.state = self.base / "state"
        self.store = JobStore(self.state)

    def test_configuration_is_private_persistent_and_roots_immutable(self):
        job = self.store.add(self.local, "/my-files", "Photos", paused=True)
        updated = self.store.update(job.id, name="Files", paused=False)
        self.assertEqual(JobStore(self.state).list(), [updated])
        self.assertEqual(stat.S_IMODE(self.state.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.state / "jobs.json").stat().st_mode), 0o600)
        self.assertEqual((updated.local_device, updated.local_inode), (self.local.stat().st_dev, self.local.stat().st_ino))
        self.store.remove(job.id)
        self.assertEqual(self.store.list(), [])

    def test_local_and_remote_overlapping_jobs_are_rejected(self):
        self.store.add(self.local, "/my-files/parent")
        child = self.local / "child"
        child.mkdir()
        other = self.base / "other"
        other.mkdir()
        with self.assertRaises(BackgroundError):
            self.store.add(child, "/my-files/elsewhere")
        with self.assertRaises(BackgroundError):
            self.store.add(other, "/my-files/parent/child")
        self.assertEqual(len(self.store.list()), 1)

    def test_corrupt_or_manually_overlapping_configuration_fails_closed(self):
        first = self.store.add(self.local, "/my-files/parent")
        rows = [asdict(first), asdict(replace(first, id="a" * 32, remote_root="/my-files/other"))]
        path = self.state / "jobs.json"
        path.write_text(json.dumps({"version": 1, "jobs": rows}))
        with self.assertRaises(BackgroundError):
            self.store.list()
        path.write_text("broken")
        with self.assertRaises(BackgroundError):
            self.store.list()
        self.assertEqual(path.read_text(), "broken")

    def test_scope_and_symlink_state_are_rejected(self):
        for remote in ("/photos", "/my-files-evil", "/my-files/../a", "/my-files//a", "/my-files/a/"):
            with self.assertRaises(BackgroundError):
                self.store.add(self.local, remote)
        link = self.base / "link"
        link.symlink_to(self.local, target_is_directory=True)
        with self.assertRaises((BackgroundError, RuntimeError)):
            self.store.add(link, "/my-files")
        state_link = self.base / "state-link"
        state_link.symlink_to(self.state, target_is_directory=True)
        with self.assertRaises((BackgroundError, RuntimeError)):
            JobStore(state_link)

    def test_planned_and_active_mounts_block_without_traversing(self):
        store = JobStore(self.state, mount_paths=[self.local / "lazy"])
        with self.assertRaises(BackgroundError):
            store.add(self.local, "/my-files")
        with patch("neutrondrive.background._mount_paths", return_value=[(self.local, "fuse.neutrondrive")]):
            with self.assertRaises(BackgroundError):
                self.store.add(self.local, "/my-files")
        with self.assertRaises(BackgroundError):
            self.store.add(self.base, "/my-files")

    def test_account_and_root_pin_cannot_be_replaced(self):
        job = self.store.add(self.local, "/my-files/a")
        self.store._pin(job.id, "uid-a", "account-one", "/my-files/uid-a")
        for uid, account in (("uid-b", "account-one"), ("uid-a", "account-two")):
            with self.assertRaises(BackgroundError):
                self.store._pin(job.id, uid, account, "/my-files/" + uid)
        self.assertEqual(self.store.list()[0].account_root_uid, "account-one")


class RemoteCacheTests(unittest.TestCase):
    def setUp(self):
        self.cloud = Cloud()
        self.folder = self.cloud.add("folder")
        self.nested = self.cloud.add("folder/nested")
        self.file = self.cloud.add("folder/nested/file.txt", b"old")
        self.cache = RemoteTreeCache(self.cloud)
        self.cache.info("/my-files")
        for path in ("/my-files", self.folder.path, self.nested.path):
            self.cache.list_nodes(path)
        self.cloud.listings.clear()

    def test_folder_delete_event_invalidates_parent_and_cached_descendants(self):
        self.cloud.nodes.pop("folder/nested/file.txt")
        self.cache.event(self.folder.uid, "root-1", ["/my-files"])
        self.assertEqual(self.cache.list_nodes(self.nested.path), [])
        self.assertIn(self.nested.path, self.cloud.listings)
        self.cache.list_nodes("/my-files")
        self.assertIn("/my-files", self.cloud.listings)

    def test_own_trash_invalidates_even_if_write_succeeded_before_error(self):
        def fail_after_write(paths):
            self.assertEqual(paths, [self.folder.path])
            self.cloud.nodes.pop("folder/nested/file.txt")
            raise BackendError("Synthetic lost acknowledgement")
        with patch.object(self.cloud, "trash", side_effect=fail_after_write, create=True):
            with self.assertRaises(BackendError):
                self.cache.trash([self.folder.path])
        self.assertEqual(self.cache.list_nodes(self.nested.path), [])
        self.cache.list_nodes("/my-files")
        self.assertIn("/my-files", self.cloud.listings)

    def test_trashed_uid_metadata_never_reenters_cached_active_listing(self):
        self.cloud.nodes.pop("folder/nested/file.txt")
        trashed = replace(self.file, raw={**self.file.raw, "isTrashed": True})
        with patch.object(self.cloud, "info", return_value=trashed):
            self.assertEqual(self.cache.info(self.file.path), trashed)
        self.assertEqual(self.cache.list_nodes(self.nested.path), [])

    def test_confirmed_trash_echo_is_settled_but_unrelated_disappearance_is_dirty(self):
        from neutrondrive.sync import _Entry
        old = _Entry("file", sha1="a" * 40, size=1, inode=123, device=1)
        plan = SimpleNamespace(_local={"gone": old, "gone/child": old, "concurrent": old})
        result = SimpleNamespace(_local_snapshot={}, _remote_snapshot={}, trashed=["gone"])
        self.assertEqual(_echo_snapshot(plan, result), {"concurrent": old})


class DaemonTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.local = self.base / "local"
        self.local.mkdir()
        self.state = self.base / "state"
        self.store = JobStore(self.state)
        self.cloud = Cloud()
        isolated_environment = patch.dict(os.environ, {
            "XDG_STATE_HOME": str(self.base / "application-state"),
            "XDG_DATA_HOME": str(self.base / "desktop-data"),
        })
        isolated_environment.start()
        self.addCleanup(isolated_environment.stop)
        self.events = Events()
        self.failures = []
        self.daemon = None
        self.thread = None
        self.controller = Controller(self.state)
        self.addCleanup(self.stop)

    def start(self, *, paused=False, remote="/my-files", real_watcher=False):
        job = self.store.add(self.local, remote, paused=paused)
        self.launch(real_watcher=real_watcher)
        return job

    def launch(self, *, real_watcher=False):
        self.daemon = BackgroundDaemon(self.state, self.cloud, self.events, debounce=0.03,
                                       retry_base=0.15, retry_max=0.3, engine_factory=ObservedEngine,
                                       watcher_factory=InotifyWatcher if real_watcher else QuietWatcher)
        def run():
            try:
                self.daemon.run()
            except Exception as error:
                self.failures.append(error)
        self.thread = threading.Thread(target=run)
        self.thread.start()
        eventually(lambda: self.events.callback is not None or self.failures)
        self.assertFalse(self.failures)

    def stop(self):
        if self.daemon and self.thread and self.thread.is_alive():
            self.daemon.command("stop")
            self.thread.join(5)
            self.assertFalse(self.thread.is_alive(), "Daemon did not stop")
        self.assertFalse(self.failures)

    def status(self):
        return self.controller.command("status")["jobs"][0]

    def idle(self):
        return eventually(lambda: self.status() if self.status()["state"] in {"idle", "conflicts"} and not self.status()["pending"] else None)

    def local_event(self, relative):
        next(iter(self.daemon._jobs.values())).watcher.callback(relative, "local")

    def test_startup_scan_then_idle_without_periodic_scans(self):
        (self.local / "initial.txt").write_bytes(b"first")
        self.start()
        self.idle()
        self.assertEqual(self.cloud.contents["initial.txt"], b"first")
        count = self.cloud.plans
        time.sleep(0.18)
        self.assertEqual(self.cloud.plans, count)
        self.assertTrue(self.controller.status()["running"])

    def test_local_edit_does_not_reload_unchanged_remote_subtrees(self):
        untouched = self.cloud.add("untouched")
        nested = self.cloud.add("untouched/nested")
        self.cloud.add("untouched/nested/cloud.txt", b"remote")
        self.start()
        self.idle()
        initial = len(self.cloud.listings)
        time.sleep(0.08)
        self.assertEqual(len(self.cloud.listings), initial)
        self.cloud.listings.clear()
        (self.local / "local.txt").write_bytes(b"new")
        self.local_event("local.txt")
        eventually(lambda: self.cloud.contents.get("local.txt") == b"new")
        self.idle()
        self.assertNotIn(untouched.path, self.cloud.listings)
        self.assertNotIn(nested.path, self.cloud.listings)
        self.assertTrue(self.cloud.listings, "Transfer preflight must remain uncached")

    def test_cached_metadata_never_hides_remote_revision_change_from_preflight(self):
        (self.local / "file.txt").write_bytes(b"original")
        self.cloud.add("file.txt", b"original")
        self.start()
        self.idle()
        # Event delivery is intentionally delayed: transfer safety must hold
        # even when the cached planning view still contains the old revision.
        self.cloud.add("file.txt", b"new cloud")
        (self.local / "file.txt").write_bytes(b"new local")
        self.local_event("file.txt")
        eventually(lambda: bool(self.status()["conflicts"]))
        self.assertEqual(self.cloud.contents["file.txt"], b"new cloud")
        self.assertEqual((self.local / "file.txt").read_bytes(), b"new local")
        self.assertEqual(self.cloud.history, [])

    def test_paused_job_never_scans_until_explicit_resume(self):
        job = self.start(paused=True)
        time.sleep(0.08)
        self.assertEqual(self.cloud.plans, 0)
        self.assertEqual(self.status()["state"], "paused")
        self.controller.command("sync", job.id)
        self.assertEqual(self.cloud.plans, 0)
        self.controller.command("resume", job.id)
        self.idle()
        self.assertEqual(self.cloud.plans, 1)
        self.controller.command("pause", job.id)
        self.assertTrue(self.store.list()[0].paused)

    def test_debounced_local_changes_and_own_echo_do_not_loop(self):
        self.start()
        self.idle()
        path = self.local / "new.txt"
        path.write_bytes(b"new")
        for _ in range(12):
            self.local_event("new.txt")
        eventually(lambda: self.cloud.contents.get("new.txt") == b"new")
        self.idle()
        self.assertEqual(self.cloud.plans, 2)
        self.local_event("new.txt")
        time.sleep(0.1)
        self.assertEqual(self.cloud.plans, 2)

    def test_same_size_edit_with_preserved_mtime_is_not_discarded(self):
        path = self.local / "preserve.txt"
        path.write_bytes(b"old")
        self.start()
        self.idle()
        original = path.stat()
        path.write_bytes(b"new")
        os.utime(path, ns=(original.st_atime_ns, original.st_mtime_ns))
        self.local_event("preserve.txt")
        eventually(lambda: self.cloud.contents.get("preserve.txt") == b"new")

    def test_real_inotify_download_echo_does_not_rescan_forever(self):
        self.cloud.add("download.txt", b"cloud")
        self.start(real_watcher=True)
        self.idle()
        time.sleep(0.15)
        count = self.cloud.plans
        time.sleep(0.15)
        self.assertEqual(self.cloud.plans, count)
        self.assertLessEqual(count, 2)
        (self.local / "upload.txt").write_bytes(b"local")
        eventually(lambda: self.cloud.contents.get("upload.txt") == b"local")

    def test_unrelated_remote_events_do_not_scan_subfolder(self):
        folder = self.cloud.add("folder")
        outside = self.cloud.add("outside")
        unrelated = self.cloud.add("outside/elsewhere", b"elsewhere")
        self.start(remote=folder.path)
        self.idle()
        self.events.emit(root_uid="root-1", node_uid=unrelated.uid, parent_uid=outside.uid, kind="update")
        time.sleep(0.08)
        self.assertEqual(self.cloud.plans, 1)
        child = self.cloud.add("folder/new.txt", b"remote")
        self.events.emit(root_uid="root-1", node_uid=child.uid, parent_uid=folder.uid, kind="create")
        eventually(lambda: (self.local / "new.txt").exists())
        self.idle()
        self.assertEqual(self.cloud.plans, 2)

    def test_unknown_parent_event_during_initial_scan_is_not_lost(self):
        folder = self.cloud.add("folder")
        new_parent = self.cloud.add("folder/new-parent")
        def after_plan(_plan):
            self.cloud.after_plan = None
            child = self.cloud.add("folder/new-parent/arrived.txt", b"arrived")
            self.events.emit(root_uid="root-1", node_uid=child.uid, parent_uid=new_parent.uid, kind="node_created")
        self.cloud.after_plan = after_plan
        self.start(remote=folder.path)
        eventually(lambda: (self.local / "new-parent/arrived.txt").exists())
        self.assertEqual((self.local / "new-parent/arrived.txt").read_bytes(), b"arrived")

    def test_remote_pending_is_durable_before_event_ack(self):
        self.start()
        self.idle()
        with self.daemon._condition:
            self.events.emit(root_uid="root-1", node_uid="new", parent_uid="root-1", kind="create")
            status = json.loads((self.state / "status.json").read_text())
            self.assertTrue(status["jobs"][0]["pending"])
            self.assertIn("remote", status["jobs"][0]["reasons"])

    def test_event_network_error_updates_status_without_tree_scan(self):
        self.start()
        self.idle()
        self.events.emit(kind="error", reset=False)
        time.sleep(0.08)
        self.assertEqual(self.cloud.plans, 1)
        self.assertEqual(self.controller.status()["event_source"]["state"], "offline")
        self.events.emit(kind="ready", root_uid="root-1", reset=False)
        time.sleep(0.06)
        self.assertEqual(self.cloud.plans, 1)
        self.events.emit(kind="ready", root_uid="root-1", reset=True)
        eventually(lambda: self.cloud.plans == 2)

    def test_offline_retry_is_bounded_and_recovers(self):
        self.cloud.fail_info = True
        self.start()
        eventually(lambda: self.status()["state"] == "offline")
        self.assertEqual(self.cloud.plans, 0)
        self.cloud.fail_info = False
        self.idle()
        self.assertEqual(self.cloud.plans, 1)
        self.assertEqual(self.status()["error"], "")

    def test_account_change_blocks_before_transfer_and_resume_keeps_pin(self):
        job = self.start()
        self.idle()
        self.cloud.nodes[""] = replace(self.cloud.nodes[""], uid="different-account")
        (self.local / "private.txt").write_bytes(b"do not send")
        self.local_event("private.txt")
        eventually(lambda: self.status()["state"] == "blocked")
        self.assertNotIn("private.txt", self.cloud.contents)
        self.controller.command("resume", job.id)
        eventually(lambda: self.status()["state"] == "blocked")
        self.assertEqual(self.store.list()[0].account_root_uid, "root-1")

    def test_trashed_selected_root_blocks_before_local_or_remote_mutation(self):
        folder = self.cloud.add("folder", raw={"parentUid": "root-1", "isTrashed": True})
        (self.local / "keep.txt").write_bytes(b"keep")
        self.start(remote=folder.path)
        eventually(lambda: self.status()["state"] == "blocked")
        self.assertEqual(self.cloud.executions, 0)
        self.assertEqual((self.local / "keep.txt").read_bytes(), b"keep")

    def test_trashed_ancestor_of_single_job_blocks_before_mutation(self):
        self.cloud.add("parent", raw={"parentUid": "root-1", "isTrashed": True})
        selected = self.cloud.add("parent/selected")
        (self.local / "keep.txt").write_bytes(b"keep")
        self.start(remote=selected.path)
        eventually(lambda: self.status()["state"] == "blocked")
        self.assertEqual(self.cloud.executions, 0)
        self.assertEqual((self.local / "keep.txt").read_bytes(), b"keep")

    def test_remote_uid_ancestor_overlap_blocks_all_jobs_before_writes(self):
        parent = self.cloud.add("parent")
        child = self.cloud.add("parent/child")
        other = self.base / "other"
        other.mkdir()
        self.store.add(other, child.path)
        (self.local / "private.txt").write_bytes(b"private")
        self.start(remote=parent.path)
        eventually(lambda: all(job["state"] == "blocked" for job in self.controller.status()["jobs"]))
        self.assertEqual(self.cloud.executions, 0)
        self.assertEqual(self.cloud.calls, [])
        self.assertEqual(len(self.store.list()), 2)

    def test_pause_cancels_running_transfer_and_keeps_job_paused(self):
        entered = threading.Event()
        def during_upload(_paths, _parent):
            entered.set()
            if self.cloud.cancel_event.wait(3):
                raise Cancelled("Paused test")
            raise AssertionError("Expected cancellation")
        self.cloud.on_upload = during_upload
        (self.local / "large.txt").write_bytes(b"file")
        job = self.start()
        self.assertTrue(entered.wait(3))
        self.controller.command("pause", job.id)
        eventually(lambda: self.status()["state"] == "paused")
        self.assertTrue(self.store.list()[0].paused)
        self.assertNotIn("large.txt", self.cloud.contents)
        time.sleep(0.08)
        self.assertEqual(self.cloud.executions, 1)

    def test_replaced_local_root_blocks(self):
        self.start()
        self.idle()
        self.local.rename(self.base / "old-local")
        self.local.mkdir()
        (self.local / "private.txt").write_bytes(b"private")
        self.local_event("private.txt")
        eventually(lambda: self.status()["state"] == "blocked")
        self.assertNotIn("private.txt", self.cloud.contents)

    def test_concurrent_new_local_file_is_not_mistaken_for_own_echo(self):
        (self.local / "first.txt").write_bytes(b"first")
        def during_upload(_paths, _parent):
            self.cloud.on_upload = None
            (self.local / "concurrent.txt").write_bytes(b"later")
            self.local_event("concurrent.txt")
        self.cloud.on_upload = during_upload
        self.start()
        eventually(lambda: self.cloud.contents.get("concurrent.txt") == b"later")
        self.idle()
        self.assertEqual(self.cloud.plans, 2)

    def test_concurrent_local_deletion_is_reconciled_after_current_transfer(self):
        (self.local / "stable.txt").write_bytes(b"keep")
        self.cloud.add("stable.txt", b"keep")
        self.start()
        self.idle()
        def during_upload(_paths, _parent):
            self.cloud.on_upload = None
            (self.local / "stable.txt").unlink()
            self.local_event("stable.txt")
        self.cloud.on_upload = during_upload
        (self.local / "trigger.txt").write_bytes(b"trigger")
        self.local_event("trigger.txt")
        eventually(lambda: "stable.txt" not in self.cloud.nodes)
        self.idle()
        self.assertEqual(self.cloud.trashed_contents["stable.txt"], b"keep")
        self.assertFalse((self.local / "stable.txt").exists())

    def test_real_inotify_local_delete_moves_remote_to_trash_without_echo_loop(self):
        path = self.local / "delete.txt"
        path.write_bytes(b"recover me")
        self.start(real_watcher=True)
        self.idle()
        path.unlink()
        eventually(lambda: "delete.txt" not in self.cloud.nodes)
        self.idle()
        self.assertEqual(self.cloud.trashed_contents["delete.txt"], b"recover me")
        self.assertFalse(path.exists())
        count = self.cloud.plans
        time.sleep(0.25)
        self.assertEqual(self.cloud.plans, count)
        self.assertFalse(self.status()["conflicts"])

    def test_remote_delete_event_uses_local_trash_and_restart_never_resurrects(self):
        from neutrondrive.local_trash import LocalTrash
        original = self.local / "delete.txt"
        original.write_bytes(b"recover locally")
        self.start(real_watcher=True)
        self.idle()
        node = self.cloud.nodes["delete.txt"]
        self.cloud.trash([node.path])
        self.events.emit(root_uid="root-1", node_uid=node.uid, parent_uid="root-1",
                         kind="node_updated", is_trashed=True)
        eventually(lambda: not original.exists())
        self.idle()
        receipts = [item for item in LocalTrash().list_entries() if item.original_path == str(original)]
        self.assertEqual(len(receipts), 1)
        self.assertEqual(receipts[0].status, "trashed")
        self.assertEqual(Path(receipts[0].trash_path).read_bytes(), b"recover locally")
        count = self.cloud.plans
        time.sleep(0.25)
        self.assertEqual(self.cloud.plans, count)
        self.stop()
        self.events = Events()
        self.launch(real_watcher=True)
        self.idle()
        self.assertFalse(original.exists())
        self.assertNotIn("delete.txt", self.cloud.nodes)
        self.assertEqual(len(LocalTrash().list_entries()), 1)
        # A deliberate restore is a new local arrival, unlike an automatic
        # resurrection from stale cached metadata or an unfinished journal.
        restored = LocalTrash().restore(receipts[0].id)
        self.assertEqual(restored.status, "restored")
        eventually(lambda: self.cloud.contents.get("delete.txt") == b"recover locally")
        self.idle()
        self.assertNotEqual(self.cloud.nodes["delete.txt"].uid, node.uid)
        self.assertFalse(self.status()["conflicts"])

    def test_folder_delete_event_retains_complete_local_tree_until_cloud_restore(self):
        from neutrondrive.local_trash import LocalTrash
        (self.local / "folder/nested").mkdir(parents=True)
        (self.local / "folder/nested/child.txt").write_bytes(b"child")
        (self.local / "folder/empty").mkdir()
        self.start(real_watcher=True)
        self.idle()
        node = self.cloud.nodes["folder"]
        identities = {str(path.relative_to(self.local)): path.stat().st_ino
                      for path in (self.local / "folder", *(self.local / "folder").rglob("*"))}
        self.cloud.trash([node.path])
        self.events.emit(root_uid="root-1", node_uid=node.uid, parent_uid="root-1",
                         kind="node_updated", is_trashed=True)
        eventually(lambda: self.status()["state"] == "conflicts")
        self.idle()
        self.assertTrue(any("folder: Folder deletion is blocked:" in line for line in self.status()["conflicts"]))
        self.assertEqual((self.local / "folder/nested/child.txt").read_bytes(), b"child")
        self.assertTrue((self.local / "folder/empty").is_dir())
        self.assertEqual(LocalTrash().list_entries(), [])
        self.assertEqual(identities, {str(path.relative_to(self.local)): path.stat().st_ino
                                    for path in (self.local / "folder", *(self.local / "folder").rglob("*"))})
        count = self.cloud.plans
        time.sleep(0.2)
        self.assertEqual(self.cloud.plans, count)
        before = self.status()["last_sync"]
        for path, trashed in list(self.cloud.trashed_nodes.items()):
            self.cloud.nodes[path] = replace(trashed, raw={**trashed.raw, "isTrashed": False})
        self.cloud.trashed_nodes.clear()
        self.cloud.contents.update(self.cloud.trashed_contents)
        self.cloud.trashed_contents.clear()
        self.events.emit(root_uid="root-1", node_uid=node.uid, parent_uid="root-1",
                         kind="node_updated", is_trashed=False)
        eventually(lambda: self.status()["last_sync"] > before and self.status()["state"] == "idle")
        self.idle()
        self.assertEqual(self.cloud.contents["folder/nested/child.txt"], b"child")
        self.assertFalse(self.status()["conflicts"])

    def test_real_inotify_folder_delete_preserves_cloud_then_recreation_resumes_sync(self):
        from neutrondrive.local_trash import LocalTrash
        (self.local / "folder/nested").mkdir(parents=True)
        (self.local / "folder/nested/child.txt").write_bytes(b"child")
        (self.local / "folder/empty").mkdir()
        self.start(real_watcher=True)
        self.idle()
        nodes, contents = dict(self.cloud.nodes), dict(self.cloud.contents)
        shutil.rmtree(self.local / "folder")
        eventually(lambda: self.status()["state"] == "conflicts")
        self.idle()
        self.assertTrue(any("folder: Folder deletion is blocked:" in line for line in self.status()["conflicts"]))
        self.assertEqual(self.cloud.nodes, nodes)
        self.assertEqual(self.cloud.contents, contents)
        self.assertFalse(self.cloud.trashed_nodes)
        self.assertFalse((self.local / "folder").exists())
        self.assertEqual(LocalTrash().list_entries(), [])
        before = self.status()["last_sync"]
        (self.local / "folder/nested").mkdir(parents=True)
        (self.local / "folder/nested/child.txt").write_bytes(b"child")
        (self.local / "folder/empty").mkdir()
        eventually(lambda: self.status()["last_sync"] > before and self.status()["state"] == "idle")
        self.idle()
        self.assertEqual(self.cloud.nodes, nodes)
        self.assertEqual(self.cloud.contents, contents)
        (self.local / "folder/nested/child.txt").write_bytes(b"changed after manual restoration")
        eventually(lambda: self.cloud.contents.get("folder/nested/child.txt") == b"changed after manual restoration")
        self.idle()
        self.assertFalse(self.status()["conflicts"])

    def test_delete_versus_edit_both_directions_stays_conflicted_after_restart(self):
        from neutrondrive.local_trash import LocalTrash
        for name in ("local-delete.txt", "remote-delete.txt"):
            (self.local / name).write_bytes(b"baseline")
        job = self.start()
        self.idle()
        self.controller.command("pause", job.id)
        eventually(lambda: self.status()["state"] == "paused")
        (self.local / "local-delete.txt").unlink()
        self.cloud.add("local-delete.txt", b"changed remotely")
        node = self.cloud.nodes["remote-delete.txt"]
        self.cloud.trash([node.path])
        (self.local / "remote-delete.txt").write_bytes(b"changed locally")
        self.controller.command("resume", job.id)
        eventually(lambda: len(self.status()["conflicts"]) >= 2)
        self.idle()
        self.stop()
        self.events = Events()
        self.launch()
        self.idle()
        self.assertEqual(self.status()["state"], "conflicts")
        self.assertEqual(self.cloud.contents["local-delete.txt"], b"changed remotely")
        self.assertFalse((self.local / "local-delete.txt").exists())
        self.assertNotIn("remote-delete.txt", self.cloud.nodes)
        self.assertEqual((self.local / "remote-delete.txt").read_bytes(), b"changed locally")
        self.assertEqual(LocalTrash().list_entries(), [])

    def test_failed_trash_stays_queued_then_retries_without_resurrection(self):
        original = self.local / "retry.txt"
        original.write_bytes(b"recover after failure")
        self.start()
        self.idle()
        trash = self.cloud.trash
        fail = threading.Event()
        fail.set()
        def flaky(paths):
            if fail.is_set():
                raise BackendError("Synthetic offline trash")
            return trash(paths)
        with patch.object(self.cloud, "trash", side_effect=flaky):
            original.unlink()
            self.local_event("retry.txt")
            eventually(lambda: self.status()["state"] == "offline")
            persisted = json.loads((self.state / "status.json").read_text())["jobs"][0]
            self.assertTrue(persisted["pending"])
            self.assertIn("retry", persisted["reasons"])
            self.assertFalse(original.exists())
            self.assertEqual(self.cloud.contents["retry.txt"], b"recover after failure")
            fail.clear()
            eventually(lambda: "retry.txt" not in self.cloud.nodes)
            self.idle()
        self.assertFalse(original.exists())
        self.assertFalse(self.status()["conflicts"])

    def test_lost_trash_acknowledgement_never_reuploads_deleted_file(self):
        original = self.local / "uncertain.txt"
        original.write_bytes(b"preserved in cloud trash")
        self.start()
        self.idle()
        trash = self.cloud.trash
        calls = []
        def lost_ack(paths):
            calls.append(tuple(paths))
            trash(paths)
            raise BackendError("Synthetic lost reply after successful trash")
        with patch.object(self.cloud, "trash", side_effect=lost_ack):
            original.unlink()
            self.local_event("uncertain.txt")
            eventually(lambda: "uncertain.txt" not in self.cloud.nodes)
            self.idle()
        self.assertFalse(original.exists())
        self.assertEqual(self.cloud.trashed_contents["uncertain.txt"], b"preserved in cloud trash")
        self.assertEqual(len(calls), 1)
        self.stop()
        self.events = Events()
        self.launch()
        self.idle()
        self.assertNotIn("uncertain.txt", self.cloud.nodes)
        self.assertFalse(original.exists())

    def test_singleton_ipc_and_orderly_shutdown(self):
        self.start()
        self.idle()
        other = BackgroundDaemon(self.state, Cloud(), Events(), watcher_factory=QuietWatcher)
        with self.assertRaises(BackgroundError):
            other.run()
        with self.assertRaises(BackgroundError):
            self.controller.command("unknown")
        self.assertEqual(stat.S_IMODE((self.state / "control.sock").stat().st_mode), 0o600)
        self.stop()
        self.assertTrue(self.events.stopped)
        self.assertFalse((self.state / "control.sock").exists())
        self.assertFalse(self.controller.status()["running"])

    def test_unsafe_existing_socket_is_preserved(self):
        target = self.state / "control.sock"
        target.write_text("keep me")
        daemon = BackgroundDaemon(self.state, self.cloud, self.events, watcher_factory=QuietWatcher)
        with self.assertRaises(BackgroundError):
            daemon.run()
        self.assertEqual(target.read_text(), "keep me")


class WatcherTests(unittest.TestCase):
    def test_recursive_writes_and_overflow_but_no_read_events(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "nested").mkdir()
            events = []
            watcher = InotifyWatcher(root, lambda path, reason: events.append((path, reason)))
            try:
                watcher.start()
                (root / "nested/file").write_bytes(b"one")
                eventually(lambda: any(path == "nested/file" for path, _ in events))
                time.sleep(0.05)
                events.clear()
                (root / "nested/file").read_bytes()
                time.sleep(0.08)
                self.assertEqual(events, [])
                watcher._consume(struct.pack("iIII", -1, InotifyWatcher._OVERFLOW, 0, 0))
                self.assertIn((None, "inotify-overflow"), events)
                events.clear()
                (root / ".neutrondrive-backup-test").write_bytes(b"backup")
                time.sleep(0.08)
                self.assertEqual(events, [])
            finally:
                watcher.stop()


if __name__ == "__main__":
    unittest.main()
