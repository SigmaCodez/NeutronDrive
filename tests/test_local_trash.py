"""Only synthetic private trees; no personal trash, credentials or cloud writes."""
import errno
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import uuid

from neutrondrive import local_trash as module
from neutrondrive.local_trash import LocalTrash, LocalTrashError
from neutrondrive.backend import Cancelled


class LocalTrashTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='neutrondrive-trash-test-')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / 'sync'
        self.root.mkdir()
        self.root_fd = os.open(self.root, module.DIR)
        self.addCleanup(os.close, self.root_fd)
        self.trash = LocalTrash(self.base / 'state', data_home=self.base / 'xdg-data')
        self.path = self.root / 'hello %.txt'
        self.path.write_bytes(b'valuable original bytes\x00\xff')

    def expected(self, relative=None):
        relative = relative or self.path.name
        with module._parent(self.root_fd, module._parts(relative)) as (fd, name, _):
            snapshot = module._snapshot(fd, name)
        return snapshot[''], {relative + '/' + name if name else relative: item for name, item in snapshot.items()}

    def move(self, *, relative=None, expected=None, **kwargs):
        relative = relative or self.path.name
        if expected is None:
            expected = self.expected(relative)[0]
        return self.trash.trash(self.root_fd, relative, expected, root_path=self.root, **kwargs)

    def test_desktop_move_is_same_inode_with_durable_standard_metadata_and_restore(self):
        original = self.path.stat()
        receipt = self.move()
        self.assertEqual(receipt.backend, 'desktop')
        self.assertEqual(receipt.status, 'trashed')
        self.assertFalse(self.path.exists())
        self.assertEqual(Path(receipt.trash_path).stat().st_ino, original.st_ino)
        self.assertEqual(Path(receipt.trash_path).read_bytes(), b'valuable original bytes\x00\xff')
        self.assertIn('hello%20%25.txt', Path(receipt.metadata_path).read_text())
        journal = self.trash.entries_dir / (receipt.id + '.json')
        self.assertEqual(journal.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.trash.list_entries(), [receipt])
        self.assertEqual(self.trash.restore(receipt.id).status, 'restored')
        self.assertEqual(self.path.stat().st_ino, original.st_ino)
        self.assertFalse(Path(receipt.trash_path).exists())

    def test_managed_trash_is_private_and_outside_sync_root(self):
        self.trash.prefer_desktop = False
        receipt = self.move()
        self.assertEqual(receipt.backend, 'managed')
        self.assertNotIn(self.root, Path(receipt.trash_path).parents)
        self.assertEqual(Path(receipt.trash_path).parent.stat().st_mode & 0o777, 0o700)
        self.trash.restore(receipt.id)
        self.assertTrue(self.path.exists())

    def test_external_mount_uses_trash_uid_and_relative_info_path(self):
        with patch.object(module, '_mount_top', side_effect=lambda path: self.base if self.base / 'sync' in (path, *path.parents) else Path('/')):
            receipt = self.move()
        self.assertIn(f'.Trash-{os.getuid()}', receipt.trash_path)
        self.assertIn('Path=sync/hello%20%25.txt\n', Path(receipt.metadata_path).read_text())

    def test_external_sticky_shared_trash_is_preferred(self):
        shared = self.base / '.Trash'
        shared.mkdir(mode=0o1777)
        shared.chmod(0o1777)
        with patch.object(module, '_mount_top', side_effect=lambda path: self.base if self.root in (path, *path.parents) else Path('/')):
            receipt = self.move()
        self.assertIn(f'/.Trash/{os.getuid()}/', receipt.trash_path)

    def test_unsafe_shared_trash_falls_back_without_following_link(self):
        outside = self.base / 'outside'
        outside.mkdir()
        (self.base / '.Trash').symlink_to(outside)
        with patch.object(module, '_mount_top', side_effect=lambda path: self.base if self.root in (path, *path.parents) else Path('/')):
            receipt = self.move()
        self.assertIn(f'.Trash-{os.getuid()}', receipt.trash_path)
        self.assertEqual(list(outside.iterdir()), [])

    def test_changed_source_is_not_moved(self):
        expected = self.expected()[0]
        self.path.write_bytes(b'new local change')
        with self.assertRaises(LocalTrashError):
            self.move(expected=expected)
        self.assertEqual(self.path.read_bytes(), b'new local change')

    def test_final_callback_cancellation_preserves_source(self):
        def cancel():
            raise RuntimeError('remote account changed')
        with self.assertRaisesRegex(LocalTrashError, 'remote account changed'):
            self.move(validate=cancel)
        self.assertTrue(self.path.exists())

    def test_final_callback_modification_is_detected(self):
        with self.assertRaises(LocalTrashError):
            self.move(validate=lambda: self.path.write_bytes(b'late writer'))
        self.assertEqual(self.path.read_bytes(), b'late writer')

    def test_symlink_source_and_ancestor_are_rejected(self):
        expected = self.expected()[0]
        self.path.unlink()
        target = self.base / 'outside'
        target.write_text('preserve')
        self.path.symlink_to(target)
        with self.assertRaises(LocalTrashError):
            self.move(expected=expected)
        self.assertEqual(target.read_text(), 'preserve')
        (self.root / 'link').symlink_to(self.base, target_is_directory=True)
        with self.assertRaises(LocalTrashError):
            self.move(relative='link/outside', expected=expected)

    def test_hardlinks_are_rejected(self):
        expected = self.expected()[0]
        os.link(self.path, self.root / 'other')
        with self.assertRaises(LocalTrashError):
            self.move(expected=expected)
        self.assertTrue(self.path.exists())

    def test_root_replacement_is_rejected(self):
        expected = self.expected()[0]
        self.root.rename(self.base / 'original-root')
        self.root.mkdir()
        with self.assertRaises(LocalTrashError):
            self.move(expected=expected)
        self.assertTrue((self.base / 'original-root' / self.path.name).exists())

    def test_folder_requires_exact_complete_tree(self):
        folder = self.root / 'folder'
        folder.mkdir()
        (folder / 'one').write_text('one')
        (folder / 'nested').mkdir()
        (folder / 'nested' / 'two').write_text('two')
        expected, tree = self.expected('folder')
        with self.assertRaises(LocalTrashError):
            self.move(relative='folder', expected=expected)
        with self.assertRaises(LocalTrashError):
            self.move(relative='folder', expected=expected, expected_tree={'folder': expected})
        receipt = self.move(relative='folder', expected=expected, expected_tree=tree)
        self.assertFalse(folder.exists())
        self.trash.restore(receipt.id)
        self.assertEqual((folder / 'nested' / 'two').read_text(), 'two')

    def test_folder_new_child_before_move_is_preserved(self):
        folder = self.root / 'folder'
        folder.mkdir()
        expected, tree = self.expected('folder')
        with self.assertRaises(LocalTrashError):
            self.move(relative='folder', expected=expected, expected_tree=tree,
                      validate=lambda: (folder / 'new').write_text('new'))
        self.assertEqual((folder / 'new').read_text(), 'new')

    def test_restore_never_overwrites_existing_file_or_symlink(self):
        receipt = self.move()
        self.path.write_text('new file at original name')
        with self.assertRaises(LocalTrashError):
            self.trash.restore(receipt.id)
        self.assertEqual(self.path.read_text(), 'new file at original name')
        self.assertTrue(Path(receipt.trash_path).exists())
        self.path.unlink()
        target = self.base / 'outside'
        target.write_text('do not overwrite')
        self.path.symlink_to(target)
        with self.assertRaises(LocalTrashError):
            self.trash.restore(receipt.id)
        self.assertEqual(target.read_text(), 'do not overwrite')

    def test_restore_blocks_changed_parent_and_supports_explicit_alternate(self):
        folder = self.root / 'folder'
        folder.mkdir()
        (folder / 'one').write_text('one')
        receipt = self.move(relative='folder/one')
        folder.rename(self.root / 'old-folder')
        folder.mkdir()
        with self.assertRaises(LocalTrashError):
            self.trash.restore(receipt.id)
        alternate = self.root / 'recovered'
        self.trash.restore(receipt.id, alternate)
        self.assertEqual(alternate.read_text(), 'one')

    def test_modified_trash_content_blocks_automatic_restore(self):
        receipt = self.move()
        Path(receipt.trash_path).write_text('edited in trash')
        with self.assertRaisesRegex(LocalTrashError, 'contents changed'):
            self.trash.restore(receipt.id)
        self.assertFalse(self.path.exists())
        self.assertEqual(Path(receipt.trash_path).read_text(), 'edited in trash')

    def test_retry_uuid_does_not_overwrite_original_receipt(self):
        identifier = str(uuid.uuid4())
        receipt = self.move(operation_id=identifier)
        original_journal = (self.trash.entries_dir / (identifier + '.json')).read_bytes()
        self.path.write_text('new independent local file')
        with self.assertRaises(LocalTrashError) as error:
            self.move(operation_id=identifier)
        self.assertEqual(error.exception.receipt.id, identifier)
        self.assertEqual((self.trash.entries_dir / (identifier + '.json')).read_bytes(), original_journal)
        self.assertEqual(self.path.read_text(), 'new independent local file')
        self.assertTrue(Path(receipt.trash_path).exists())

    def test_collision_in_trash_never_overwrites_content(self):
        rename = module._rename_noreplace
        def collide(src_fd, src, dst_fd, dst):
            fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=dst_fd)
            os.write(fd, b'other trash object')
            os.close(fd)
            rename(src_fd, src, dst_fd, dst)
        with patch.object(module, '_rename_noreplace', side_effect=collide):
            with self.assertRaises(LocalTrashError) as error:
                self.move()
        self.assertTrue(self.path.exists())
        self.assertEqual(Path(error.exception.receipt.trash_path).read_bytes(), b'other trash object')

    def test_replacement_in_last_rename_window_is_rolled_back_without_loss(self):
        rename = module._rename_noreplace
        calls = []
        def race(src_fd, src, dst_fd, dst):
            if not calls:
                calls.append(True)
                self.path.rename(self.root / 'original-preserved')
                self.path.write_text('replacement raced rename')
            rename(src_fd, src, dst_fd, dst)
        with patch.object(module, '_rename_noreplace', side_effect=race):
            with self.assertRaises(LocalTrashError) as error:
                self.move()
        self.assertEqual(error.exception.receipt.status, 'aborted')
        self.assertEqual(self.path.read_text(), 'replacement raced rename')
        self.assertEqual((self.root / 'original-preserved').read_bytes(), b'valuable original bytes\x00\xff')

    def test_post_move_race_with_occupied_original_keeps_recovery(self):
        rename = module._rename_noreplace
        calls = []
        def race(src_fd, src, dst_fd, dst):
            rename(src_fd, src, dst_fd, dst)
            if not calls:
                calls.append(True)
                self.path.write_text('another file now occupies original')
                fd = os.open(dst, os.O_WRONLY | os.O_TRUNC, dir_fd=dst_fd)
                os.write(fd, b'concurrent edit via already-open descriptor')
                os.close(fd)
        with patch.object(module, '_rename_noreplace', side_effect=race):
            with self.assertRaises(LocalTrashError) as error:
                self.move()
        self.assertEqual(error.exception.receipt.status, 'recovery')
        self.assertTrue(Path(error.exception.receipt.trash_path).exists())
        self.assertEqual(self.path.read_text(), 'another file now occupies original')

    def test_durable_prepared_journal_survives_commit_failure(self):
        save = self.trash._save
        def fail_commits(fd, record, *, initial=False):
            if not initial:
                raise OSError(errno.EIO, 'synthetic fsync failure')
            return save(fd, record, initial=True)
        with patch.object(self.trash, '_save', side_effect=fail_commits):
            with self.assertRaises(LocalTrashError) as error:
                self.move()
        record = self.trash.list_entries()[0]
        self.assertEqual(record.status, 'prepared')
        self.assertTrue(Path(record.trash_path).exists())
        self.trash.restore(record.id)
        self.assertEqual(self.path.read_bytes(), b'valuable original bytes\x00\xff')

    def test_cross_device_rename_failure_does_not_copy_or_delete(self):
        with patch.object(module, '_rename_noreplace', side_effect=OSError(errno.EXDEV, 'synthetic separate filesystem')):
            with self.assertRaises(LocalTrashError):
                self.move()
        self.assertEqual(self.path.read_bytes(), b'valuable original bytes\x00\xff')

    def test_lists_only_registered_receipts_and_detects_missing_objects(self):
        receipt = self.move()
        (Path(receipt.trash_path).parent / 'unrelated-personal-file').write_text('not enumerated')
        self.assertEqual([x.id for x in self.trash.list_entries()], [receipt.id])
        Path(receipt.trash_path).rename(self.base / 'desktop-restored')
        self.assertEqual(self.trash.list_entries()[0].status, 'missing')

    def test_symlink_registry_and_trash_directories_are_never_followed(self):
        external = self.base / 'external'
        external.mkdir()
        (self.base / 'bad-state').symlink_to(external)
        with self.assertRaises((LocalTrashError, OSError)):
            LocalTrash(self.base / 'bad-state', data_home=self.base / 'xdg')
        self.trash.data_home.mkdir()
        (self.trash.data_home / 'Trash').symlink_to(external)
        receipt = self.move()
        self.assertEqual(receipt.backend, 'managed')
        self.assertEqual(list(external.iterdir()), [])

    def test_source_parent_race_preserves_payload_and_replacement_directory(self):
        folder = self.root / 'folder'
        folder.mkdir()
        (folder / 'one').write_text('one')
        rename = module._rename_noreplace
        def race(src_fd, src, dst_fd, dst):
            folder.rename(self.root / 'old-folder')
            folder.mkdir()
            (folder / 'one').write_text('unrelated replacement')
            return rename(src_fd, src, dst_fd, dst)
        with patch.object(module, '_rename_noreplace', side_effect=race):
            with self.assertRaises(LocalTrashError) as error:
                self.move(relative='folder/one')
        self.assertEqual(error.exception.receipt.status, 'recovery')
        self.assertEqual(Path(error.exception.receipt.trash_path).read_text(), 'one')
        self.assertEqual((folder / 'one').read_text(), 'unrelated replacement')

    def test_restore_parent_race_reports_actual_preserved_location(self):
        folder = self.root / 'folder'
        folder.mkdir()
        (folder / 'one').write_text('one')
        receipt = self.move(relative='folder/one')
        rename = module._rename_noreplace
        def race(src_fd, src, dst_fd, dst):
            folder.rename(self.root / 'old-folder')
            folder.mkdir()
            (folder / 'one').write_text('unrelated replacement')
            return rename(src_fd, src, dst_fd, dst)
        with patch.object(module, '_rename_noreplace', side_effect=race):
            with self.assertRaises(LocalTrashError) as error:
                self.trash.restore(receipt.id)
        self.assertEqual(error.exception.receipt.status, 'recovery')
        self.assertEqual((self.root / 'old-folder' / 'one').read_text(), 'one')
        self.assertEqual((folder / 'one').read_text(), 'unrelated replacement')
        self.assertIn('/old-folder/one', self.trash.get_receipt(receipt.id).restore_path)

    def test_interrupted_restore_is_reconciled_by_inode_and_content(self):
        receipt = self.move()
        save = self.trash._save
        def fail_commit(fd, record, *, initial=False):
            if record['status'] != 'restoring':
                raise OSError(errno.EIO, 'synthetic final commit failure')
            return save(fd, record, initial=initial)
        with patch.object(self.trash, '_save', side_effect=fail_commit):
            with self.assertRaises(LocalTrashError):
                self.trash.restore(receipt.id)
        self.assertTrue(self.path.exists())
        self.assertEqual(self.trash.get_receipt(receipt.id).status, 'recovery')
        final = self.trash.restore(receipt.id)
        self.assertEqual(final.status, 'restored')
        self.assertEqual(self.path.read_bytes(), b'valuable original bytes\x00\xff')

    def test_journal_write_failure_before_rename_leaves_source(self):
        with patch.object(self.trash, '_save', side_effect=OSError(errno.ENOSPC, 'synthetic full disk')):
            with self.assertRaises(LocalTrashError):
                self.move()
        self.assertTrue(self.path.exists())
        self.assertEqual(self.trash.list_entries(), [])

    def test_mount_identity_change_is_rejected_even_on_same_device(self):
        # Simulate a bind mount's distinct mnt_id with the same st_dev.
        with patch.object(module, '_mount_id', side_effect=[1, 2]):
            with self.assertRaises(LocalTrashError):
                module._snapshot(self.root_fd, self.path.name)
        self.assertTrue(self.path.exists())

    def test_restore_retry_after_user_resolves_target_conflict(self):
        receipt = self.move()
        self.path.write_text('user change')
        with self.assertRaises(LocalTrashError):
            self.trash.restore(receipt.id)
        self.path.rename(self.root / 'user-kept-change')
        self.assertEqual(self.trash.restore(receipt.id).status, 'restored')
        self.assertEqual(self.path.read_bytes(), b'valuable original bytes\x00\xff')
        self.assertEqual((self.root / 'user-kept-change').read_text(), 'user change')

    def test_every_nested_directory_is_fsynced_before_move(self):
        folder = self.root / 'folder'
        (folder / 'nested').mkdir(parents=True)
        (folder / 'nested' / 'data').write_text('new child')
        real_fsync = os.fsync
        directories = set()
        def fsync(fd):
            import stat
            if stat.S_ISDIR(os.fstat(fd).st_mode):
                directories.add(Path(os.readlink('/proc/self/fd/' + str(fd))))
            return real_fsync(fd)
        with patch.object(module.os, 'fsync', side_effect=fsync):
            module._snapshot(self.root_fd, 'folder')
        self.assertIn(folder, directories)
        self.assertIn(folder / 'nested', directories)

    def test_cancelled_hash_keeps_source_and_cancellation_type(self):
        expected = self.expected()[0]
        calls = []
        def cancel():
            calls.append(True)
            if len(calls) >= 3:
                raise Cancelled('cancel synthetic trash hash')
        with self.assertRaises(Cancelled):
            self.move(expected=expected, check_cancel=cancel)
        self.assertTrue(self.path.exists())

    def test_lock_wait_is_bounded(self):
        import fcntl
        fd = os.open(self.trash.state_dir / 'lock', os.O_CREAT | os.O_RDWR, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            with self.assertRaisesRegex(LocalTrashError, 'busy'):
                with self.trash._locked(timeout=0):
                    self.fail('lock unexpectedly acquired')
        finally:
            os.close(fd)

    def test_final_remote_validation_runs_after_local_hashing(self):
        snapshot = module._snapshot
        events = []
        def traced(*args, **kwargs):
            events.append('hash')
            return snapshot(*args, **kwargs)
        rename = module._rename_noreplace
        def moved(*args):
            events.append('rename')
            return rename(*args)
        expected = self.expected()[0]
        with patch.object(module, '_snapshot', side_effect=traced), patch.object(module, '_rename_noreplace', side_effect=moved):
            self.move(expected=expected, validate=lambda: events.append('remote-proof'))
        self.assertEqual(events, ['hash', 'hash', 'remote-proof', 'rename', 'hash'])

    def test_restore_parent_moved_without_replacement_has_recovery_path(self):
        folder = self.root / 'folder'
        folder.mkdir()
        (folder / 'one').write_text('one')
        receipt = self.move(relative='folder/one')
        rename = module._rename_noreplace
        def race(src_fd, src, dst_fd, dst):
            folder.rename(self.root / 'old-folder')
            return rename(src_fd, src, dst_fd, dst)
        with patch.object(module, '_rename_noreplace', side_effect=race):
            with self.assertRaises(LocalTrashError) as error:
                self.trash.restore(receipt.id)
        self.assertIn('/old-folder/one', error.exception.receipt.restore_path)
        self.assertEqual(Path(error.exception.receipt.restore_path).read_text(), 'one')


if __name__ == '__main__':
    unittest.main()
