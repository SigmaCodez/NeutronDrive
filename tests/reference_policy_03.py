"""Frozen 0.3 planner oracle for migration differential tests; never imported by runtime."""
from __future__ import annotations
from core_contract.sync import SyncAction, SyncConflict, SyncError

def _under(path: str, blocked: set[str]) -> bool:
    return any(path == item or path.startswith(item + "/") for item in blocked)



def _baseline_matches(entry: _Entry, previous: dict[str, Any], side: str, *, ancestor: bool = False) -> bool:
    if previous.get("identity_version") != 2 or entry.kind != previous.get("kind"):
        return False
    if entry.kind == "file" and (entry.sha1 != previous.get("sha1") or entry.size != previous.get("size")):
        return False
    if side == "local":
        return ((entry.device, entry.inode) == (previous.get("local_device"), previous.get("local_inode"))
                and ((entry.kind == "folder" and ancestor) or (entry.mtime_ns, entry.ctime_ns) == (previous.get("local_mtime_ns"), previous.get("local_ctime_ns"))))
    return (entry.uid == previous.get("remote_uid") and entry.revision == previous.get("remote_revision")
            and (entry.kind == "folder" or bool(entry.revision)))


def _subtree(entries: dict[str, Any], relative: str) -> dict[str, Any]:
    return {path: entry for path, entry in entries.items() if path == relative or path.startswith(relative + "/")}


def _require_file_deletion(previous: dict[str, Any]) -> None:
    if previous.get("kind") == "folder":
        raise SyncError("Folder deletion is blocked: the public Proton SDK cannot verify all children of a trashed folder. "
                        "Review this folder's contents manually. Individual files in existing folders remain supported.")


def _deletion_proof(relative: str, target: dict[str, _Entry], baseline: dict[str, dict[str, Any]],
                    blocked: set[str], side: str) -> None:
    previous, current = baseline.get(relative, {}), target.get(relative)
    _require_file_deletion(previous)
    if current is None or not _baseline_matches(current, previous, side):
        raise SyncError("Deletion conflicts with an edit, replacement, or unverified old baseline")
    expected, actual = _subtree(baseline, relative), _subtree(target, relative)
    if current.kind == "folder":
        if not previous.get("subtree_complete"):
            raise SyncError("Folder deletion lacks a complete verified subtree baseline")
        if any(path == relative or path.startswith(relative + "/") for path in blocked):
            raise SyncError("Folder contains unsupported, ignored, or unreadable children")
        if set(expected) != set(actual):
            raise SyncError("Folder contents changed since the deletion baseline")
        if any(not _baseline_matches(actual[path], entry, side) for path, entry in expected.items()):
            raise SyncError("Folder deletion conflicts with changed or replaced descendants")



def plan_policy(self, plan, local, cloud, baseline, pending, blocked, remote):
    # A rename/move inside either scanned tree is not deletion proof.
    # Block both its old and new locations to avoid a trash+copy cascade.
    for old, previous in baseline.items():
        moved = []
        if old not in local and previous.get("identity_version") == 2:
            moved.extend(path for path, entry in local.items()
                         if (entry.device, entry.inode) == (previous.get("local_device"), previous.get("local_inode")))
        if old not in cloud and previous.get("remote_uid"):
            moved.extend(path for path, entry in cloud.items() if entry.uid == previous["remote_uid"])
        if moved:
            plan.conflicts.append(SyncConflict(old, "A baseline identity moved to another path; resolve the move first"))
            plan._blocked.update([old, *moved])
    covered_deletions = set()
    for relative in sorted(local.keys() | cloud.keys() | baseline.keys(), key=lambda x: (x.count("/"), x)):
        if _under(relative, plan._blocked | covered_deletions):
            continue
        left, right, previous = local.get(relative), cloud.get(relative), baseline.get(relative)
        reason = ""
        if left is None and right is None:
            continue
        pending_root = next((path for path in pending if relative == path or relative.startswith(path + "/")), None)
        if pending_root and ((left is not None and right is not None)
                             or pending[pending_root]["direction"] != ("local" if left is not None else "remote")):
            reason = "Earlier trash operation has an uncertain outcome; retained deletion intent requires review"
        elif left is not None and right is not None:
            if left.kind != right.kind:
                reason = "File/folder type conflict"
            elif left.kind == "folder" or left.sha1 == right.sha1:
                continue
            elif not previous or previous.get("kind") != "file":
                reason = "Both sides already exist with different contents; no common baseline"
            elif left.sha1 == previous["sha1"]:
                plan.actions.append(SyncAction("download", relative, "Remote changed; retain a local backup"))
            elif right.sha1 == previous["sha1"]:
                plan.actions.append(SyncAction("upload", relative, "Local changed; create a remote revision"))
            else:
                reason = "Both sides changed since the last successful synchronization"
        elif previous:
            try:
                side = "local" if left is not None else "remote"
                self._check_ancestor_baselines(relative, local, cloud, baseline)
                _deletion_proof(relative, local if left is not None else cloud, baseline, set(blocked), side)
                if left is not None:
                    self._prove_remote_deletion(plan, remote, relative)
                plan.actions.append(SyncAction("trash_local" if left is not None else "trash_remote", relative,
                                               "Propagate verified deletion to recoverable trash"))
                covered_deletions.add(relative)
            except SyncError as error:
                reason = str(error)
        elif left is not None:
            plan.actions.append(SyncAction("mkdir_remote" if left.kind == "folder" else "upload", relative))
        elif right is not None:
            plan.actions.append(SyncAction("mkdir_local" if right.kind == "folder" else "download", relative))
        if reason:
            plan.conflicts.append(SyncConflict(relative, reason))
            plan._blocked.add(relative)
    return plan
