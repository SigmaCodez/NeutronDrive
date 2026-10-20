"""Filesystem-backed sync tests; no Proton account or network required."""

from dataclasses import replace
import hashlib
import json
import os
import shutil
from pathlib import Path, PurePosixPath
import tempfile
import threading
import unittest
from unittest.mock import patch

from neutrondrive.backend import BackendError, Cancelled, Node
from neutrondrive.local_trash import LocalTrash
from neutrondrive.sync import SyncEngine, SyncError


def sha1(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()


class FakeBackend:
    def __init__(self):
        self.nodes = {"": Node("root-1", "My files", "folder", path="/my-files")}
        self.contents = {}
        self.trashed_nodes = {}
        self.trashed_contents = {}
        self.calls = []
        self.history = []
        self.counter = 0
        self.on_upload = None
        self.on_download = None
        self.on_trash = None
        self.cancel_event = threading.Event()

    def add(self, relative, data=None, **overrides):
        self.counter += 1
        uid = self.nodes[relative].uid if relative in self.nodes else f"node-{self.counter}"
        node = Node(uid, PurePosixPath(relative).name, "folder" if data is None else "file",
                    size=0 if data is None else len(data), revision="" if data is None else f"r{self.counter}",
                    sha1="" if data is None else sha1(data), path=f"/my-files/{uid}")
        parent = str(PurePosixPath(relative).parent)
        parent = "" if parent == "." else parent
        overrides.setdefault("raw", {"parentUid": self.nodes.get(parent, self.nodes[""]).uid})
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
        for node in [*self.nodes.values(), *self.trashed_nodes.values()]:
            if node.path == path:
                return node
        raise RuntimeError("Remote path not found")

    def trash(self, paths):
        if self.on_trash:
            self.on_trash(paths)
        for path in paths:
            relative = self.relative(path)
            self.calls.append(("trash", relative))
            for name in list(self.nodes):
                if name == relative or name.startswith(relative + "/"):
                    node = self.nodes.pop(name)
                    self.trashed_nodes[name] = replace(node, raw={**node.raw, "isTrashed": True})
                    if name in self.contents:
                        self.trashed_contents[name] = self.contents.pop(name)
        return {}

    def list_nodes(self, path):
        try:
            parent = self.relative(path)
            source = self.nodes
        except RuntimeError:
            parent = next(relative for relative, node in self.trashed_nodes.items() if node.path == path)
            source = self.trashed_nodes
        return [node for relative, node in source.items()
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
        self.trash = LocalTrash(self.base / "trash-state", data_home=self.base / "trash-data")
        self.engine = SyncEngine(self.backend, self.state, local_trash=self.trash)

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

    def test_verified_local_deletion_moves_remote_to_trash_without_resurrection(self):
        self.establish("local.txt")
        (self.local / "local.txt").unlink()
        plan = self.plan()
        self.assertEqual([(action.kind, action.relative_path) for action in plan.actions], [("trash_remote", "local.txt")])
        result = self.engine.execute(plan)
        self.assertTrue(result.ok, result)
        self.assertEqual(result.trashed, ["local.txt"])
        self.assertEqual(self.backend.trashed_contents["local.txt"], b"original")
        self.assertNotIn("local.txt", self.backend.nodes)
        self.assertEqual(self.plan().actions, [])

    def test_verified_remote_trash_moves_local_to_recoverable_trash(self):
        self.establish("remote.txt")
        self.backend.trash([self.backend.nodes["remote.txt"].path])
        plan = self.plan()
        self.assertEqual([action.kind for action in plan.actions], ["trash_local"])
        result = self.engine.execute(plan)
        self.assertTrue(result.ok, result)
        self.assertFalse((self.local / "remote.txt").exists())
        receipt = self.trash.list_entries()[0]
        self.assertEqual(Path(receipt.trash_path).read_bytes(), b"original")
        self.assertEqual(result.trashed, ["remote.txt"])
        self.assertFalse(self.plan().actions)

    def baseline(self):
        path = next(self.state.glob("*.json"))
        return path, json.loads(path.read_text())

    def establish_tree(self):
        self.write("folder/nested/file.txt", b"valuable")
        (self.local / "folder/empty").mkdir()
        self.backend.add("folder")
        self.backend.add("folder/nested")
        self.backend.add("folder/nested/file.txt", b"valuable")
        self.backend.add("folder/empty")
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)

    def test_complete_folder_deletion_uses_one_remote_trash_operation(self):
        self.establish_tree()
        shutil.rmtree(self.local / "folder")
        plan = self.plan()
        self.assertEqual([(a.kind, a.relative_path) for a in plan.actions], [("trash_remote", "folder")])
        result = self.engine.execute(plan)
        self.assertTrue(result.ok, result)
        self.assertEqual(self.backend.trashed_contents["folder/nested/file.txt"], b"valuable")
        self.assertEqual([call for call in self.backend.calls if call[0] == "trash"], [("trash", "folder")])

    def test_complete_remote_folder_deletion_retains_local_subtree_in_trash(self):
        self.establish_tree()
        self.backend.trash([self.backend.nodes["folder"].path])
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(result.trashed, ["folder"])
        receipt = self.trash.list_entries()[0]
        self.assertEqual((Path(receipt.trash_path) / "nested/file.txt").read_bytes(), b"valuable")
        self.assertTrue((Path(receipt.trash_path) / "empty").is_dir())

    def test_folder_remote_new_child_blocks_whole_deletion_without_cascade(self):
        self.establish_tree()
        shutil.rmtree(self.local / "folder")
        self.backend.add("folder/new.txt", b"new")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(any(c.relative_path == "folder" for c in plan.conflicts))
        self.engine.execute(plan)
        self.assertIn("folder/nested/file.txt", self.backend.nodes)

    def test_folder_local_ignored_child_blocks_whole_deletion(self):
        self.establish_tree()
        self.write("folder/.neutrondrive-backup-important", b"keep")
        self.backend.trash([self.backend.nodes["folder"].path])
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.engine.execute(plan)
        self.assertEqual((self.local / "folder/.neutrondrive-backup-important").read_bytes(), b"keep")

    def test_folder_unsupported_remote_child_blocks_deletion(self):
        self.establish_tree()
        shutil.rmtree(self.local / "folder")
        self.backend.add("folder/document", b"doc", media_type="application/vnd.proton.doc")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)

    def test_delete_versus_local_edit_conflicts(self):
        self.establish()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        self.write("a.txt", b"edited locally")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.engine.execute(plan)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"edited locally")

    def test_delete_versus_same_content_local_replacement_conflicts(self):
        self.establish()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        old = self.local / "a.txt"
        moved = self.base / "old.txt"
        old.rename(moved)
        old.write_bytes(b"original")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.engine.execute(plan)
        self.assertEqual(old.read_bytes(), b"original")

    def test_delete_versus_new_remote_revision_even_same_content_conflicts(self):
        self.establish()
        (self.local / "a.txt").unlink()
        self.backend.add("a.txt", b"original")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.assertIn("a.txt", self.backend.nodes)

    def test_local_move_blocks_both_old_and_new_paths(self):
        self.establish()
        (self.local / "a.txt").rename(self.local / "renamed.txt")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.engine.execute(plan)
        self.assertIn("a.txt", self.backend.nodes)
        self.assertNotIn("renamed.txt", self.backend.nodes)

    def test_active_uid_moved_outside_selected_remote_tree_is_not_trash(self):
        selected = self.backend.add("selected")
        outside = self.backend.add("outside")
        self.write("a.txt", b"original")
        node = self.backend.add("selected/a.txt", b"original")
        self.engine.execute(self.engine.plan(self.local, selected.path))
        self.backend.nodes["outside/a.txt"] = replace(self.backend.nodes.pop("selected/a.txt"), raw={"parentUid": outside.uid})
        self.backend.contents["outside/a.txt"] = self.backend.contents.pop("selected/a.txt")
        plan = self.engine.plan(self.local, selected.path)
        self.assertFalse(plan.actions)
        self.assertTrue(any("active elsewhere" in c.reason for c in plan.conflicts))
        self.assertEqual((self.local / "a.txt").read_bytes(), b"original")

    def test_missing_uid_without_confirmed_trash_is_not_local_deletion_proof(self):
        self.establish()
        self.backend.nodes.pop("a.txt")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.assertTrue((self.local / "a.txt").exists())

    def test_uid_moved_between_listing_and_info_is_not_trashed(self):
        selected = self.backend.add("selected")
        outside = self.backend.add("outside")
        self.write("a.txt", b"valuable")
        node = self.backend.add("selected/a.txt", b"valuable")
        self.assertTrue(self.engine.execute(self.engine.plan(self.local, selected.path)).ok)
        (self.local / "a.txt").unlink()
        plan = self.engine.plan(self.local, selected.path)
        info = self.backend.info
        def move_before_info(path):
            if path == node.path and "selected/a.txt" in self.backend.nodes:
                self.backend.nodes["outside/a.txt"] = replace(
                    self.backend.nodes.pop("selected/a.txt"), raw={"parentUid": outside.uid})
                self.backend.contents["outside/a.txt"] = self.backend.contents.pop("selected/a.txt")
            return info(path)
        with patch.object(self.backend, "info", side_effect=move_before_info):
            result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertEqual(self.backend.contents["outside/a.txt"], b"valuable")
        self.assertFalse(self.backend.trashed_nodes)
        self.assertFalse(any(call[0] == "trash" for call in self.backend.calls))

    def test_uid_moved_outside_then_trashed_does_not_delete_local_copy(self):
        selected = self.backend.add("selected")
        outside = self.backend.add("outside")
        self.write("a.txt", b"valuable")
        node = self.backend.add("selected/a.txt", b"valuable")
        self.assertTrue(self.engine.execute(self.engine.plan(self.local, selected.path)).ok)
        self.backend.nodes["outside/a.txt"] = replace(
            self.backend.nodes.pop("selected/a.txt"), raw={"parentUid": outside.uid})
        self.backend.contents["outside/a.txt"] = self.backend.contents.pop("selected/a.txt")
        self.backend.trash([node.path])
        plan = self.engine.plan(self.local, selected.path)
        self.assertFalse(plan.actions)
        self.assertTrue(any("different parent" in conflict.reason for conflict in plan.conflicts))
        result = self.engine.execute(plan)
        self.assertFalse(result.ok)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"valuable")
        self.assertFalse(self.trash.list_entries())

    def test_legacy_missing_side_conflicts_but_converged_entries_migrate(self):
        self.establish("missing.txt")
        self.establish("present.txt")
        path, baseline = self.baseline()
        baseline["version"] = 1
        for row in baseline["entries"].values():
            for key in list(row):
                if key not in {"kind", "sha1", "remote_uid", "remote_revision"}:
                    row.pop(key)
        path.write_text(json.dumps(baseline))
        (self.local / "missing.txt").unlink()
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(any(c.relative_path == "missing.txt" for c in plan.conflicts))
        self.engine.execute(plan)
        _, updated = self.baseline()
        self.assertEqual(updated["version"], 2)
        self.assertEqual(updated["entries"]["missing.txt"]["identity_version"], 1)
        self.assertEqual(updated["entries"]["present.txt"]["identity_version"], 2)
        self.assertIn("missing.txt", self.backend.nodes)

    def test_stale_deletion_preview_never_trashes_reappeared_local_source(self):
        self.establish()
        (self.local / "a.txt").unlink()
        plan = self.plan()
        self.write("a.txt", b"reappeared")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertIn("a.txt", self.backend.nodes)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"reappeared")

    def test_stale_deletion_preview_never_trashes_edited_local_target(self):
        self.establish()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        plan = self.plan()
        self.write("a.txt", b"new local")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"new local")
        self.assertEqual(self.trash.list_entries(), [])

    def test_failed_remote_trash_retries_only_after_fresh_proof(self):
        self.establish()
        (self.local / "a.txt").unlink()
        def fail(_paths):
            self.backend.on_trash = None
            raise BackendError("temporary failure")
        self.backend.on_trash = fail
        result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        self.assertIn("a.txt", self.baseline()[1]["pending_deletions"])
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(self.baseline()[1]["pending_deletions"], {})
        self.assertEqual(self.backend.trashed_contents["a.txt"], b"original")

    def test_remote_trash_lost_ack_is_reconciled_without_reupload(self):
        self.establish()
        (self.local / "a.txt").unlink()
        real_trash = self.backend.trash
        def lost_ack(paths):
            real_trash(paths)
            raise BackendError("response lost after server operation")
        with patch.object(self.backend, "trash", side_effect=lost_ack):
            result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        self.assertTrue(self.baseline()[1]["pending_deletions"])
        recovered = self.engine.execute(self.plan())
        self.assertTrue(recovered.ok, recovered)
        self.assertEqual(recovered.trashed, ["a.txt"])
        self.assertEqual(self.baseline()[1]["entries"], {})
        self.assertEqual(self.baseline()[1]["pending_deletions"], {})
        self.assertFalse((self.local / "a.txt").exists())

    def test_local_trash_lost_ack_requires_matching_persisted_receipt(self):
        self.establish()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        real_trash = self.trash.trash
        def lost_ack(*args, **kwargs):
            real_trash(*args, **kwargs)
            raise BackendError("response lost after local rename")
        with patch.object(self.trash, "trash", side_effect=lost_ack):
            result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        recovered = self.engine.execute(self.plan())
        self.assertTrue(recovered.ok, recovered)
        self.assertEqual(recovered.trashed, ["a.txt"])
        self.assertEqual(len(self.trash.list_entries()), 1)
        self.assertEqual(self.baseline()[1]["pending_deletions"], {})

    def test_crash_after_durable_intent_preserves_retryable_deletion(self):
        self.establish()
        (self.local / "a.txt").unlink()
        with patch.object(self.backend, "trash", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.engine.execute(self.plan())
        self.assertTrue(self.baseline()[1]["pending_deletions"])
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertNotIn("a.txt", self.backend.nodes)

    def test_failed_intent_persistence_prevents_remote_mutation(self):
        self.establish()
        (self.local / "a.txt").unlink()
        with patch.object(self.engine, "_save", side_effect=OSError("disk full")):
            result = self.engine.execute(self.plan())
        self.assertTrue(result.errors)
        self.assertIn("a.txt", self.backend.nodes)
        self.assertFalse(self.backend.trashed_nodes)

    def test_both_deleted_cleans_baseline_without_new_operations(self):
        self.establish()
        (self.local / "a.txt").unlink()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        self.backend.calls.clear()
        result = self.engine.execute(self.plan())
        self.assertTrue(result.ok, result)
        self.assertEqual(self.backend.calls, [])
        self.assertEqual(self.baseline()[1]["entries"], {})

    def test_trashed_selected_ancestor_blocks_plan(self):
        parent = self.backend.add("parent")
        selected = self.backend.add("parent/selected")
        self.backend.nodes["parent"] = replace(parent, raw={**parent.raw, "isTrashed": True})
        with self.assertRaisesRegex(SyncError, "Trashed"):
            self.engine.plan(self.local, selected.path)

    def test_ancestor_trashed_during_last_preflight_scan_blocks_remote_trash(self):
        ancestor = self.backend.add("ancestor")
        selected = self.backend.add("ancestor/selected")
        self.write("folder/child", b"valuable")
        self.backend.add("ancestor/selected/folder")
        self.backend.add("ancestor/selected/folder/child", b"valuable")
        self.assertTrue(self.engine.execute(self.engine.plan(self.local, selected.path)).ok)
        shutil.rmtree(self.local / "folder")
        plan = self.engine.plan(self.local, selected.path)
        scan = self.engine._scan_remote
        def race(*args, **kwargs):
            result = scan(*args, **kwargs)
            if kwargs.get("fresh"):
                self.backend.nodes["ancestor"] = replace(ancestor, raw={**ancestor.raw, "isTrashed": True})
            return result
        with patch.object(self.engine, "_scan_remote", side_effect=race):
            result = self.engine.execute(plan)
        self.assertFalse(result.ok)
        self.assertIn("ancestor/selected/folder/child", self.backend.nodes)
        self.assertFalse(any(call[0] == "trash" for call in self.backend.calls))

    def test_ancestor_trashed_during_last_preflight_scan_blocks_local_trash(self):
        ancestor = self.backend.add("ancestor")
        selected = self.backend.add("ancestor/selected")
        self.write("a.txt", b"valuable")
        node = self.backend.add("ancestor/selected/a.txt", b"valuable")
        self.assertTrue(self.engine.execute(self.engine.plan(self.local, selected.path)).ok)
        self.backend.trash([node.path])
        plan = self.engine.plan(self.local, selected.path)
        scan = self.engine._scan_local
        def race(*args, **kwargs):
            result = scan(*args, **kwargs)
            self.backend.nodes["ancestor"] = replace(ancestor, raw={**ancestor.raw, "isTrashed": True})
            return result
        with patch.object(self.engine, "_scan_local", side_effect=race):
            result = self.engine.execute(plan)
        self.assertFalse(result.ok)
        self.assertEqual((self.local / "a.txt").read_bytes(), b"valuable")
        self.assertFalse(self.trash.list_entries())

    def test_folder_new_child_after_preview_blocks_remote_trash(self):
        self.establish_tree()
        shutil.rmtree(self.local / "folder")
        plan = self.plan()
        self.backend.add("folder/new.txt", b"new")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertIn("folder/new.txt", self.backend.nodes)
        self.assertFalse(self.backend.trashed_nodes)

    def test_folder_new_child_during_remote_trash_retains_journal_and_conflict(self):
        self.establish_tree()
        shutil.rmtree(self.local / "folder")
        plan = self.plan()
        self.backend.on_trash = lambda _paths: self.backend.add("folder/new.txt", b"new")
        result = self.engine.execute(plan)
        self.assertTrue(result.conflicts)
        self.assertFalse(result.trashed)
        self.assertEqual(self.backend.trashed_contents["folder/new.txt"], b"new")
        self.assertIn("folder", self.baseline()[1]["pending_deletions"])
        retried = self.engine.execute(self.plan())
        self.assertTrue(retried.conflicts)
        self.assertIn("folder", self.baseline()[1]["entries"])

    def test_local_root_replacement_invalidates_v2_baseline(self):
        self.establish()
        self.local.rename(self.base / "old-root")
        self.local.mkdir()
        with self.assertRaisesRegex(SyncError, "root identity changed"):
            self.plan()
        self.assertIn("a.txt", self.backend.nodes)

    def test_delete_versus_type_change_conflicts(self):
        self.establish()
        self.backend.trash([self.backend.nodes["a.txt"].path])
        (self.local / "a.txt").unlink()
        self.write("a.txt/child", b"keep")
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.assertEqual((self.local / "a.txt/child").read_bytes(), b"keep")

    def test_same_content_remote_uid_replacement_is_not_original_trash_target(self):
        self.establish()
        (self.local / "a.txt").unlink()
        old = self.backend.nodes.pop("a.txt")
        self.backend.add("a.txt", b"original", revision=old.revision)
        plan = self.plan()
        self.assertFalse(plan.actions)
        self.assertTrue(plan.conflicts)
        self.assertFalse(self.backend.trashed_nodes)

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
