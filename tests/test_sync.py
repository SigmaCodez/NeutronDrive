"""Filesystem-backed sync tests; no Proton account or network required."""

from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import tempfile
import threading
import unittest
from unittest.mock import patch

from neutrondrive.backend import Cancelled, Node
from neutrondrive.sync import SyncEngine, SyncError


def sha1(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()


class FakeBackend:
    def __init__(self):
        self.nodes = {"": Node("root-1", "My files", "folder", path="/my-files")}
        self.contents = {}
        self.calls = []
        self.history = []
        self.counter = 0
        self.on_upload = None
        self.on_download = None
        self.cancel_event = threading.Event()

    def add(self, relative, data=None, **overrides):
        self.counter += 1
        uid = self.nodes[relative].uid if relative in self.nodes else f"node-{self.counter}"
        node = Node(uid, PurePosixPath(relative).name, "folder" if data is None else "file",
                    size=0 if data is None else len(data), revision="" if data is None else f"r{self.counter}",
                    sha1="" if data is None else sha1(data), path=f"/my-files/{uid}")
        self.nodes[relative] = replace(node, **overrides)
        if data is not None:
            self.contents[relative] = data
        return self.nodes[relative]

    def relative(self, path):
        for relative, node in self.nodes.items():
            if node.path == path:
                return relative
        raise RuntimeError(f"Remote path not found: {path}")

    def info(self, path):
        return self.nodes[self.relative(path)]

    def list_nodes(self, path):
        parent = self.relative(path)
        return [node for relative, node in self.nodes.items()
                if relative and str(PurePosixPath(relative).parent) == (parent or ".")]

    def mkdir(self, parent, name):
        relative = "/".join(filter(None, [self.relative(parent), name]))
        if relative in self.nodes:
            raise RuntimeError("Folder already exists")
        self.calls.append(("mkdir", relative))
        return self.add(relative)

    def upload(self, paths, parent, file_strategy="rename", folder_strategy="merge"):
        if self.on_upload:
            self.on_upload(paths, parent)
        for path in paths:
            relative = "/".join(filter(None, [self.relative(parent), path.name]))
            self.calls.append(("upload", relative, file_strategy))
            if relative in self.nodes:
                if file_strategy == "skip":
                    continue
                if file_strategy != "create-new-revision":
                    raise AssertionError("Unsafe upload strategy")
                self.history.append((relative, self.contents[relative]))
            self.add(relative, path.read_bytes())
        return {}

    def download(self, paths, local_folder, file_strategy="rename", folder_strategy="merge"):
        for path in paths:
            relative = self.relative(path)
            self.calls.append(("download", relative))
            (local_folder / self.nodes[relative].name).write_bytes(self.contents[relative])
        if self.on_download:
            self.on_download(paths, local_folder)
        return {}


class SyncTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.local = self.base / "local"
        self.local.mkdir()
        self.state = self.base / "state"
        self.backend = FakeBackend()
        self.engine = SyncEngine(self.backend, self.state)

    def write(self, relative, data):
        path = self.local / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def plan(self):
        return self.engine.plan(self.local, "/my-files")

    def establish(self, relative="a.txt", data=b"original"):
        self.write(relative, data)
        self.backend.add(relative, data)
        result = self.engine.execute(self.plan())
        self.assertFalse(result.errors)
        self.assertFalse(result.conflicts)

    def test_preview_does_not_transfer_and_new_files_converge_both_directions(self):
        self.write("local.txt", b"local")
        self.backend.add("remote.txt", b"remote")
        plan = self.plan()
        self.assertEqual({action.kind for action in plan.actions}, {"upload", "download"})
        self.assertEqual(self.backend.calls, [])
        result = self.engine.execute(plan)
        self.assertTrue(result.ok, result)
        self.assertEqual(self.backend.contents["local.txt"], b"local")
        self.assertEqual((self.local / "remote.txt").read_bytes(), b"remote")
        self.assertEqual(self.plan().actions, [])

    def test_nested_and_empty_folders_in_both_directions(self):
        (self.local / "left/empty").mkdir(parents=True)
        self.write("left/sub/a", b"local")
        self.backend.add("right")
        self.backend.add("right/empty")
        self.backend.add("right/sub")
        self.backend.add("right/sub/b", b"remote")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(self.backend.contents["left/sub/a"], b"local")
        self.assertIn("left/empty", self.backend.nodes)
        self.assertEqual((self.local / "right/sub/b").read_bytes(), b"remote")
        self.assertTrue((self.local / "right/empty").is_dir())

    def test_initial_collision_never_overwrites_either_side(self):
        self.write("same", b"left")
        self.backend.add("same", b"right")
        plan = self.plan()
        self.assertEqual(plan.actions, [])
        self.assertEqual(len(plan.conflicts), 1)
        self.engine.execute(plan)
        self.assertEqual((self.local / "same").read_bytes(), b"left")
        self.assertEqual(self.backend.contents["same"], b"right")

    def test_local_change_uploads_as_revision(self):
        self.establish()
        self.write("a.txt", b"new local")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(self.backend.contents["a.txt"], b"new local")
        self.assertIn(("a.txt", b"original"), self.backend.history)
        self.assertEqual(self.plan().actions, [])

    def test_remote_change_keeps_local_backup(self):
        self.establish()
        self.backend.add("a.txt", b"new remote")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"new remote")
        self.assertEqual(len(result.backups), 1)
        self.assertEqual(Path(result.backups[0]).read_bytes(), b"original")
        next_plan = self.plan()
        self.assertEqual(next_plan.actions, [])
        self.assertTrue(any("backup" in item for item in next_plan.skipped))

    def test_both_changed_is_conflict(self):
        self.establish()
        self.write("a.txt", b"left")
        self.backend.add("a.txt", b"right")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertIn("Both sides changed", plan.conflicts[0].reason)

    def test_deletions_never_propagate_or_resurrect(self):
        self.establish("local.txt")
        self.establish("remote.txt")
        for side in ("local", "remote"):
            with self.subTest(side=side):
                if side == "local":
                    (self.local / f"{side}.txt").unlink()
                else:
                    del self.backend.nodes[f"{side}.txt"]
                plan = self.plan()
                self.assertFalse(plan.actions)
                self.assertTrue(any(item.relative_path == f"{side}.txt" for item in plan.conflicts))

    def test_file_folder_conflict_blocks_children(self):
        self.write("thing/child", b"keep")
        self.backend.add("thing", b"also keep")
        plan = self.plan()
        self.assertEqual(len(plan.conflicts), 1)
        self.assertEqual(plan.actions, [])

    def test_local_change_after_preview_is_blocked(self):
        self.write("a.txt", b"before")
        plan = self.plan()
        self.write("a.txt", b"after")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertEqual(self.backend.calls, [])

    def test_remote_change_after_preview_is_blocked(self):
        self.establish()
        self.write("a.txt", b"local edit")
        plan = self.plan()
        self.backend.add("a.txt", b"remote edit")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertEqual(self.backend.contents["a.txt"], b"remote edit")

    def test_local_edit_while_downloading_survives(self):
        self.establish()
        self.backend.add("a.txt", b"remote edit")
        self.backend.on_download = lambda *_: self.write("a.txt", b"concurrent local")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.conflicts)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"concurrent local")
        self.assertFalse(result.backups)

    def test_remote_edit_while_downloading_survives(self):
        self.establish()
        self.backend.add("a.txt", b"remote edit")
        self.backend.on_download = lambda *_: self.backend.add("a.txt", b"concurrent remote")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.conflicts)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"original")

    def test_download_hash_mismatch_does_not_install(self):
        self.backend.add("a.txt", b"expected")
        self.backend.contents["a.txt"] = b"corrupt"
        result = self.engine.execute(self.plan())
        self.assertTrue(result.conflicts)
        self.assertFalse((self.local / "a.txt").exists())

    def test_upload_reads_private_snapshot_even_if_source_changes(self):
        self.write("a.txt", b"planned")
        self.backend.on_upload = lambda paths, *_: self.write("a.txt", b"edited during upload")
        result = self.engine.execute(self.plan())
        self.assertFalse(result.errors)
        self.assertEqual(self.backend.contents["a.txt"], b"planned")
        self.assertEqual((self.local / "a.txt").read_bytes(), b"edited during upload")
        self.assertTrue(self.plan().conflicts)

    def test_upload_snapshot_preserves_original_mtime(self):
        source = self.write("a.txt", b"planned")
        timestamp = 1_600_000_000_123_456_789
        os.utime(source, ns=(timestamp, timestamp))
        observed = []
        self.backend.on_upload = lambda paths, *_: observed.append(paths[0].stat().st_mtime_ns)
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(observed, [timestamp])

    def test_cancel_during_local_hash_scan_aborts_before_transfer(self):
        self.write("large", b"x" * (3 * 1024 * 1024))
        real_read = os.read
        reads = []

        def cancel_after_first_chunk(fd, count):
            value = real_read(fd, count)
            reads.append(len(value))
            self.backend.cancel_event.set()
            return value

        with patch("neutrondrive.sync.os.read", side_effect=cancel_after_first_chunk):
            with self.assertRaises(Cancelled):
                self.plan()
        self.assertEqual(len(reads), 1)
        self.assertFalse(self.backend.calls)

    def test_cancel_during_download_preserves_existing_local_file(self):
        self.establish()
        self.backend.add("a.txt", b"new remote")
        self.backend.on_download = lambda *_: self.backend.cancel_event.set()
        with self.assertRaises(Cancelled):
            self.engine.execute(self.plan())
        self.assertEqual((self.local / "a.txt").read_bytes(), b"original")

    def test_new_remote_name_racing_upload_is_not_replaced(self):
        self.write("a.txt", b"planned")
        self.backend.on_upload = lambda *_: self.backend.add("a.txt", b"concurrent")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.conflicts)
        self.assertEqual(self.backend.contents["a.txt"], b"concurrent")

    def test_new_local_name_racing_install_is_not_replaced(self):
        self.backend.add("a.txt", b"remote")
        real_link = os.link

        def racing_link(src, dst, **kwargs):
            if str(src).startswith(".neutrondrive-stage-"):
                self.write("a.txt", b"concurrent")
            return real_link(src, dst, **kwargs)

        with patch("neutrondrive.sync.os.link", side_effect=racing_link):
            result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"concurrent")

    def test_failed_install_restores_old_file_and_preserves_backup(self):
        self.establish()
        self.backend.add("a.txt", b"remote edit")
        real_link = os.link

        def failing_link(src, dst, **kwargs):
            if str(src).startswith(".neutrondrive-stage-"):
                raise OSError("Simulated install failure")
            return real_link(src, dst, **kwargs)

        with patch("neutrondrive.sync.os.link", side_effect=failing_link):
            result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"original")
        self.assertEqual(Path(result.backups[0]).read_bytes(), b"original")

    def test_symlinks_and_special_files_are_never_followed(self):
        outside = self.base / "outside"
        outside.mkdir()
        (outside / "secret").write_bytes(b"secret")
        (self.local / "link").symlink_to(outside, target_is_directory=True)
        os.mkfifo(self.local / "pipe")
        self.backend.add("link")
        self.backend.add("link/new", b"remote")
        plan = self.plan()
        self.assertEqual(plan.actions, [])
        self.assertEqual(len(plan.skipped), 2)
        self.engine.execute(plan)
        self.assertFalse((outside / "new").exists())

    def test_root_symlink_is_rejected(self):
        link = self.base / "link"
        link.symlink_to(self.local, target_is_directory=True)
        with self.assertRaises(OSError):
            self.engine.plan(link, "/my-files")

    def test_symlink_swap_after_preview_cannot_write_outside(self):
        self.backend.add("child")
        self.backend.add("child/a", b"remote")
        (self.local / "child").mkdir()
        plan = self.plan()
        outside = self.base / "outside"
        outside.mkdir()
        (self.local / "child").rmdir()
        (self.local / "child").symlink_to(outside, target_is_directory=True)
        result = self.engine.execute(plan)
        self.assertTrue(result.errors)
        self.assertFalse((outside / "a").exists())

    def test_unsafe_names_native_docs_and_missing_hash_are_reported(self):
        self.backend.add("../escape", b"bad", name="../escape")
        self.backend.add("document", b"native", media_type="application/vnd.proton.doc")
        self.backend.add("nohash", b"unknown", sha1="")
        self.backend.add("unsafe", b"unknown", safe=False)
        self.write("wild[card", b"unsupported")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertGreaterEqual(len(plan.skipped), 4)
        self.assertFalse((self.base / "escape").exists())

    def test_duplicate_remote_names_are_blocked(self):
        self.backend.add("one", b"one", name="same")
        self.backend.add("two", b"two", name="same")
        self.write("same", b"local")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(any("Multiple remote" in item for item in plan.skipped))

    def test_baseline_bound_to_remote_uid(self):
        self.establish()
        self.backend.nodes[""] = replace(self.backend.nodes[""], uid="another-account-root")
        self.backend.add("a.txt", b"different account")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertIn("no common baseline", plan.conflicts[0].reason)

    def test_root_account_change_after_preview_aborts(self):
        self.write("a.txt", b"local")
        plan = self.plan()
        self.backend.nodes[""] = replace(self.backend.nodes[""], uid="another-account-root")
        with self.assertRaisesRegex(SyncError, "account changed"):
            self.engine.execute(plan)
        self.assertFalse(self.backend.calls)

    def test_plan_is_single_use_and_stale_baseline_is_rejected(self):
        self.establish()
        first, second = self.plan(), self.plan()
        self.write("b", b"new")
        self.backend.add("b", b"new")
        self.engine.execute(first)
        with self.assertRaisesRegex(SyncError, "fresh preview"):
            self.engine.execute(first)
        with self.assertRaisesRegex(SyncError, "baseline"):
            self.engine.execute(second)

    def test_corrupt_baseline_is_not_reset(self):
        self.establish()
        path = next(self.state.glob("*.json"))
        path.write_text("not json")
        with self.assertRaisesRegex(SyncError, "Corrupt sync baseline"):
            self.plan()
        self.assertEqual(path.read_text(), "not json")

    def test_state_inside_synced_tree_is_rejected(self):
        engine = SyncEngine(self.backend, self.local / "state")
        with self.assertRaisesRegex(SyncError, "outside"):
            engine.plan(self.local, "/my-files")

    def test_another_process_lock_blocks_execution(self):
        import fcntl
        plan = self.plan()
        with (self.state / "sync.lock").open("w") as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(SyncError, "already running"):
                self.engine.execute(plan)


if __name__ == "__main__":
    unittest.main()
