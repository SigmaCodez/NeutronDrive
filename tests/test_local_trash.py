"""Only synthetic private trees; no personal trash, credentials or cloud writes."""
import os
from pathlib import Path
import tempfile
import unittest
import uuid

from core_contract import rust_core
from core_contract.local_trash import LocalTrash, LocalTrashError
from core_contract.backend import Cancelled

# Native move race/commit/ordering tests live in core/src/trash_move.rs.
# Native mount identity and nested-fsync tests live in core/src/filesystem.rs.
# Restore parent races and interrupted commits are tested at the native syscall/
# persistence boundary in core/src/trash_restore.rs; Python mocks no longer reach it.
class LocalTrashTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='neutrondrive-trash-test-')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / 'sync'
        self.root.mkdir()
        self.root_fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        self.addCleanup(os.close, self.root_fd)
        self.trash = LocalTrash(self.base / 'state', data_home=self.base / 'xdg-data')
        self.path = self.root / 'hello %.txt'
        self.path.write_bytes(b'valuable original bytes\x00\xff')

    def expected(self, relative=None):
        relative = relative or self.path.name
        path = self.root / relative
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            snapshot = rust_core.backend_value('trash_snapshot', parent=fd, name=path.name)
        finally:
            os.close(fd)
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

    def test_multibyte_trash_prefix_preserves_valid_unicode_and_restores(self):
        self.path = self.root / ('a' + '😀' * 30 + '.txt')
        self.path.write_bytes(b'unicode content')
        receipt = self.move()
        self.assertEqual(Path(receipt.trash_path).read_bytes(), b'unicode content')
        Path(receipt.trash_path).name.encode('utf-8')
        self.trash.restore(receipt.id)
        self.assertEqual(self.path.read_bytes(), b'unicode content')

    def test_managed_trash_is_private_and_outside_sync_root(self):
        self.trash.prefer_desktop = False
        receipt = self.move()
        self.assertEqual(receipt.backend, 'managed')
        self.assertNotIn(self.root, Path(receipt.trash_path).parents)
        self.assertEqual(Path(receipt.trash_path).parent.stat().st_mode & 0o777, 0o700)
        self.trash.restore(receipt.id)
        self.assertTrue(self.path.exists())

    # External mount selection, sticky shared trash and symlink fallback now
    # execute in Rust; fault cases live in core/src/trash_location.rs.

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

    def test_restore_retry_after_user_resolves_target_conflict(self):
        receipt = self.move()
        self.path.write_text('user change')
        with self.assertRaises(LocalTrashError):
            self.trash.restore(receipt.id)
        self.path.rename(self.root / 'user-kept-change')
        self.assertEqual(self.trash.restore(receipt.id).status, 'restored')
        self.assertEqual(self.path.read_bytes(), b'valuable original bytes\x00\xff')
        self.assertEqual((self.root / 'user-kept-change').read_text(), 'user change')

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

    def test_native_restore_cancellation_preserves_type_and_receipt(self):
        receipt = self.move()
        calls = []
        def cancel():
            calls.append(True)
            if len(calls) >= 3:
                raise Cancelled('cancel synthetic restore proof')
        with self.assertRaises(Cancelled) as error:
            self.trash.restore(receipt.id, check_cancel=cancel)
        self.assertEqual(error.exception.receipt.id, receipt.id)
        self.assertTrue(Path(receipt.trash_path).exists())
        self.assertFalse(self.path.exists())

    def test_lock_wait_is_bounded(self):
        import fcntl
        fd = os.open(self.trash.state_dir / 'lock', os.O_CREAT | os.O_RDWR, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            with self.assertRaisesRegex(Exception, 'busy'):
                rust_core.backend_value('trash_journal', operation=dict(
                    action='lock', state=str(self.trash.state_dir), timeout=0))
        finally:
            os.close(fd)

if __name__ == '__main__':
    unittest.main()
