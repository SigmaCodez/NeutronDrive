"""Content safety, offline metadata and conservative writeback without a cloud account."""
from dataclasses import replace
import errno
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

from neutrondrive.backend import BackendError, Node
from neutrondrive.ondemand import OnDemandFS, OnDemandError, _key


class FakeBackend:
    def __init__(self):
        self.nodes = {'root': Node('root', 'My files', 'folder', path='/my-files')}
        self.parents = {}
        self.content = {}
        self.downloads = 0
        self.uploads = 0
        self.online = True
        self.corrupt_download = False
        self.change_on_download = False
        self.fail_upload = False
        self.mutate_after_upload = False
        self.sequence = 0
        self.add('hello.txt', b'hello from the cloud\n')

    def add(self, name, data=None, parent='root'):
        self.sequence += 1
        uid = 'node' + str(self.sequence)
        self.parents[uid] = parent
        self.nodes[uid] = Node(uid, name, 'folder' if data is None else 'file',
                               None if data is None else len(data), revision='r1',
                               sha1='' if data is None else hashlib.sha1(data).hexdigest(),
                               path='/my-files/' + uid)
        if data is not None:
            self.content[uid] = data
        return self.nodes[uid]

    def update(self, uid, data):
        node = self.nodes[uid]
        self.nodes[uid] = replace(node, size=len(data), sha1=hashlib.sha1(data).hexdigest(), revision=node.revision + 'x')
        self.content[uid] = data

    def _check(self):
        if not self.online:
            raise BackendError('Network unavailable')

    def info(self, path):
        self._check()
        uid = 'root' if path == '/my-files' else path.rsplit('/', 1)[-1]
        return self.nodes[uid]

    def list_nodes(self, path):
        parent = self.info(path).uid
        return [n for uid, n in self.nodes.items() if self.parents.get(uid) == parent]

    def download(self, paths, destination):
        self._check()
        self.downloads += 1
        node = self.info(paths[0])
        data = b'corrupted' if self.corrupt_download else self.content[node.uid]
        (destination / node.name).write_bytes(data)
        if self.change_on_download:
            self.update(node.uid, b'remote edited during download')

    def upload(self, paths, parent, file_strategy='rename'):
        self._check()
        self.uploads += 1
        if self.fail_upload:
            raise BackendError('Synthetic upload failure')
        parent_uid = self.info(parent).uid
        name = paths[0].name
        data = paths[0].read_bytes()
        matches = [n for n in self.list_nodes(parent) if n.name == name]
        if matches and file_strategy == 'create-new-revision':
            uid = matches[0].uid
            self.update(uid, data)
        else:
            uid = self.add(name, data, parent_uid).uid
        if self.mutate_after_upload:
            self.update(uid, b'another device wrote after upload')

    def mkdir(self, parent, name):
        return self.add(name, parent=self.info(parent).uid)

    def rename(self, path, name):
        node = self.info(path)
        self.nodes[node.uid] = replace(node, name=name)

    def move(self, paths, parent):
        self.parents[self.info(paths[0]).uid] = self.info(parent).uid

    def trash(self, paths):
        for path in paths:
            uid = self.info(path).uid
            self.parents[uid] = 'trash'


class OnDemandTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.cache = Path(self.temp.name) / 'cache'
        self.backend = FakeBackend()
        self.fs = OnDemandFS(self.backend, self.cache, writable=True)
        self.inode = self.fs.lookup(1, 'hello.txt').inode

    def tearDown(self):
        self.fs.close()
        self.temp.cleanup()

    def test_listing_does_not_download_or_make_content_files(self):
        self.assertEqual(self.fs.metadata(self.inode)['size'], 21)
        self.assertEqual(self.backend.downloads, 0)
        self.assertEqual(list(self.fs.objects.iterdir()), [])

    def test_open_downloads_once_and_verifies(self):
        handle = self.fs.open(self.inode)
        self.assertEqual(self.fs.read(handle, 6, 4), b'from')
        self.fs.release(handle)
        handle = self.fs.open(self.inode)
        self.assertEqual(self.backend.downloads, 1)
        self.fs.release(handle)
        cached = list(self.fs.objects.iterdir())[0]
        self.assertEqual(cached.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.cache.stat().st_mode & 0o777, 0o700)

    def test_corrupted_download_never_becomes_cached(self):
        self.backend.corrupt_download = True
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode)
        self.assertEqual(list(self.fs.objects.iterdir()), [])

    def test_revision_change_during_download_never_becomes_cached(self):
        self.backend.change_on_download = True
        with self.assertRaises(OnDemandError) as cm:
            self.fs.open(self.inode)
        self.assertEqual(cm.exception.errno, errno.ESTALE)
        self.assertEqual(list(self.fs.objects.iterdir()), [])

    def test_cache_tampering_is_rejected(self):
        self.fs.pin(self.inode)
        cached = list(self.fs.objects.iterdir())[0]
        cached.write_bytes(b'tampered')
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode)

    def test_cache_symlink_is_rejected(self):
        target = self.fs.objects / _key(self.fs.entries[self.inode].node)
        target.symlink_to('/etc/passwd')
        with self.assertRaises(OSError):
            self.fs.open(self.inode)

    def test_offline_pin_and_metadata_survive_remount(self):
        self.fs.pin(self.inode)
        self.fs.close()
        self.backend.online = False
        self.fs = OnDemandFS(self.backend, self.cache, offline=True)
        node = self.fs.lookup(1, 'hello.txt')
        handle = self.fs.open(node.inode)
        self.assertEqual(self.fs.read(handle, 0, 100), b'hello from the cloud\n')
        self.fs.release(handle)
        self.assertTrue(self.fs.metadata(node.inode)['pinned'])

    def test_offline_uncached_content_fails_never_returns_zeros(self):
        self.fs.close()
        self.fs = OnDemandFS(self.backend, self.cache, offline=True)
        with self.assertRaises(OnDemandError) as cm:
            self.fs.open(self.inode)
        self.assertEqual(cm.exception.errno, errno.ENETDOWN)

    def test_evict_pinned_and_open_denied_then_succeeds(self):
        self.fs.pin(self.inode)
        with self.assertRaises(OnDemandError):
            self.fs.evict(self.inode)
        self.fs.pin(self.inode, False)
        h = self.fs.open(self.inode)
        with self.assertRaises(OnDemandError):
            self.fs.evict(self.inode)
        self.fs.release(h)
        self.fs.evict(self.inode)
        self.assertFalse(self.fs.metadata(self.inode)['cached'])

    def test_account_root_binding(self):
        self.fs.close()
        self.backend.nodes['root'] = replace(self.backend.nodes['root'], uid='another-account')
        with self.assertRaises(OnDemandError) as cm:
            OnDemandFS(self.backend, self.cache)
        self.assertEqual(cm.exception.errno, errno.ESTALE)

    def test_second_mount_cannot_share_cache(self):
        with self.assertRaises(OnDemandError) as cm:
            OnDemandFS(self.backend, self.cache)
        self.assertEqual(cm.exception.errno, errno.EBUSY)

    def test_unsafe_cache_ancestor_is_rejected(self):
        symlink = Path(self.temp.name) / 'link'
        symlink.symlink_to(self.cache, target_is_directory=True)
        with self.assertRaises(OSError):
            OnDemandFS(self.backend, symlink / 'child')

    def test_writeback_creates_revision_and_preserves_old_cache(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'new content')
        self.assertEqual(self.backend.uploads, 0)
        self.fs.flush(h)
        self.assertEqual(self.backend.content['node1'], b'new content')
        self.assertEqual(self.backend.uploads, 1)
        self.fs.release(h)
        self.assertEqual(self.fs.pending_recovery(), [])
        self.assertIn(b'hello from the cloud\n', [p.read_bytes() for p in self.fs.objects.iterdir()])

    def test_concurrent_remote_edit_keeps_local_recovery(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'local edit')
        self.backend.update('node1', b'other device')
        with self.assertRaises(OnDemandError) as cm:
            self.fs.flush(h)
        self.assertEqual(cm.exception.errno, errno.ESTALE)
        self.fs.release(h)
        self.assertEqual(self.backend.content['node1'], b'other device')
        recovery = self.fs.pending_recovery()
        self.assertEqual(len(recovery), 1)
        self.assertEqual(Path(recovery[0]['data_path']).read_bytes(), b'local edit')
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode, os.O_RDWR)

    def test_upload_failure_survives_remount(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'durable local data')
        self.backend.fail_upload = True
        with self.assertRaises(BackendError):
            self.fs.flush(h)
        self.fs.close()
        self.fs = OnDemandFS(self.backend, self.cache)
        self.assertEqual(Path(self.fs.pending_recovery()[0]['data_path']).read_bytes(), b'durable local data')

    def test_post_upload_race_keeps_local_copy(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'my edit')
        self.backend.mutate_after_upload = True
        with self.assertRaises(OnDemandError):
            self.fs.flush(h)
        self.fs.release(h)
        self.assertEqual(Path(self.fs.pending_recovery()[0]['data_path']).read_bytes(), b'my edit')

    def test_release_without_flush_retains_recovery(self):
        h = self.fs.open(self.inode, os.O_RDWR)
        self.fs.write(h, 0, b'LOCAL')
        self.fs.release(h)
        self.assertEqual(len(self.fs.pending_recovery()), 1)
        self.assertEqual(self.backend.uploads, 0)

    def test_create_does_not_fake_remote_success(self):
        h, entry = self.fs.create(1, 'new.txt')
        self.fs.write(h, 0, b'new file')
        self.assertEqual(len(self.backend.nodes), 2)
        self.fs.flush(h)
        self.fs.release(h)
        self.assertEqual(self.fs.lookup(1, 'new.txt').inode, entry.inode)
        self.assertIn(b'new file', self.backend.content.values())

    def test_create_race_does_not_overwrite(self):
        h, _ = self.fs.create(1, 'new.txt')
        self.fs.write(h, 0, b'local')
        node = self.backend.add('new.txt', b'other')
        with self.assertRaises(OnDemandError):
            self.fs.flush(h)
        self.fs.release(h)
        self.assertEqual(self.backend.content[node.uid], b'other')
        self.assertEqual(len(self.fs.pending_recovery()), 1)

    def test_read_only_write_denied(self):
        self.fs.writable = False
        with self.assertRaises(OnDemandError) as cm:
            self.fs.open(self.inode, os.O_RDWR)
        self.assertEqual(cm.exception.errno, errno.EROFS)
        self.assertEqual(self.backend.downloads, 0)

    def test_single_writer_and_no_readers_during_write(self):
        h = self.fs.open(self.inode, os.O_RDWR)
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode, os.O_RDWR)
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode)
        self.fs.release(h)

    def test_duplicate_names_and_native_documents_are_not_exposed(self):
        self.backend.add('hello.txt', b'duplicate')
        doc = self.backend.add('document', b'content')
        self.backend.nodes[doc.uid] = replace(doc, sha1='')
        self.fs.refresh()
        self.assertEqual(self.fs.listdir(), [])

    def test_mkdir_move_rename_trash_and_offline_deletion(self):
        folder = self.fs.mkdir(1, 'Folder')
        self.fs.rename(1, 'hello.txt', 1, 'renamed.txt')
        self.fs.rename(1, 'renamed.txt', folder.inode, 'renamed.txt')
        self.assertEqual(self.fs.lookup(folder.inode, 'renamed.txt').inode, self.inode)
        self.fs.unlink(folder.inode, 'renamed.txt')
        self.fs.unlink(1, 'Folder', directory=True)
        self.fs.close()
        self.fs = OnDemandFS(self.backend, self.cache, offline=True)
        self.assertEqual(self.fs.listdir(), [])

    def test_no_destructive_rename_replacement(self):
        self.backend.add('target.txt', b'keep')
        self.fs.refresh()
        with self.assertRaises(OnDemandError) as cm:
            self.fs.rename(1, 'hello.txt', 1, 'target.txt', no_replace=True)
        self.assertEqual(cm.exception.errno, errno.EEXIST)

    def test_editor_temp_replace_preserves_target_uid_and_revisions(self):
        h = self.fs.open(self.inode)
        self.fs.release(h)
        temp = self.backend.add('.editor-temp', b'replacement content')
        self.fs.refresh()
        original = self.backend.nodes['node1'].revision
        self.fs.rename(1, '.editor-temp', 1, 'hello.txt')
        self.assertEqual(self.backend.content['node1'], b'replacement content')
        self.assertNotEqual(self.backend.nodes['node1'].revision, original)
        self.assertEqual(self.backend.parents[temp.uid], 'trash')
        self.assertEqual(self.fs.pending_recovery(), [])

    def test_editor_replace_rejects_remote_edit_even_after_listing_refresh(self):
        h = self.fs.open(self.inode)
        self.fs.release(h)
        temp = self.backend.add('.editor-temp', b'local editing result')
        self.backend.update('node1', b'remote simultaneous edit')
        self.fs.refresh()
        with self.assertRaises(OnDemandError) as cm:
            self.fs.rename(1, '.editor-temp', 1, 'hello.txt')
        self.assertEqual(cm.exception.errno, errno.ESTALE)
        self.assertEqual(self.backend.content['node1'], b'remote simultaneous edit')
        self.assertEqual(self.backend.parents[temp.uid], 'root')

    def test_editor_replace_trash_failure_retains_both_and_recovery(self):
        temp = self.backend.add('.editor-temp', b'new editor bytes')
        self.fs.refresh()
        def fail_trash(paths):
            raise BackendError('Synthetic source-trash failure')
        self.backend.trash = fail_trash
        with self.assertRaises(BackendError):
            self.fs.rename(1, '.editor-temp', 1, 'hello.txt')
        self.assertEqual(self.backend.content['node1'], b'new editor bytes')
        self.assertEqual(self.backend.parents[temp.uid], 'root')
        recovery = self.fs.pending_recovery()
        self.assertEqual(Path(recovery[0]['data_path']).read_bytes(), b'new editor bytes')
        self.assertEqual(recovery[0]['intent']['operation'], 'replace')

    def test_network_loss_keeps_existing_mount_pinned_files_readable(self):
        self.fs.pin(self.inode)
        self.backend.online = False
        self.fs.invalidate()
        node = self.fs.lookup(1, 'hello.txt')
        h = self.fs.open(node.inode)
        self.assertEqual(self.fs.read(h, 0, 100), b'hello from the cloud\n')
        self.fs.release(h)
        with self.assertRaises(BackendError):
            self.fs.open(node.inode, os.O_RDWR)

    def test_evict_frees_all_verified_revisions_for_file(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'new revision')
        self.fs.flush(h)
        self.fs.release(h)
        self.assertEqual(len(list(self.fs.objects.iterdir())), 2)
        self.fs.evict(self.inode)
        self.assertEqual(list(self.fs.objects.iterdir()), [])

    def test_pinned_snapshot_survives_remote_refresh_then_network_loss(self):
        self.fs.pin(self.inode)
        self.backend.update('node1', b'new remote version')
        self.fs.refresh()
        self.backend.online = False
        h = self.fs.open(self.inode)
        self.assertEqual(self.fs.read(h, 0, 100), b'hello from the cloud\n')
        self.assertEqual(self.fs.metadata(self.inode)['size'], 21)
        self.fs.release(h)
        self.backend.online = True
        h = self.fs.open(self.inode)
        self.assertEqual(self.fs.read(h, 0, 100), b'new remote version')
        self.fs.release(h)

    def test_account_change_before_writeback_blocks_cloud_and_retains_edit(self):
        h = self.fs.open(self.inode, os.O_RDWR | os.O_TRUNC)
        self.fs.write(h, 0, b'old account private edit')
        self.backend.nodes['root'] = replace(self.backend.nodes['root'], uid='new-account')
        with self.assertRaises(OnDemandError) as cm:
            self.fs.flush(h)
        self.assertEqual(cm.exception.errno, errno.EACCES)
        self.assertEqual(self.backend.uploads, 0)
        self.fs.release(h)
        self.assertEqual(Path(self.fs.pending_recovery()[0]['data_path']).read_bytes(), b'old account private edit')
        with self.assertRaises(OnDemandError):
            self.fs.refresh()
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode)

    def test_account_event_blocks_refresh_and_new_opens(self):
        from types import SimpleNamespace
        from neutrondrive.fuse import MountManager
        manager = MountManager(self.backend, Path(self.temp.name) / 'state')
        manager.fs = self.fs
        manager.invalidate(SimpleNamespace(root_uid='different-root', kind='ready'))
        self.assertTrue(self.fs.account_blocked)
        self.assertTrue(manager.status()['read_only'])
        with self.assertRaises(OnDemandError):
            self.fs.refresh()

    def test_disk_full_create_has_no_phantom_entry(self):
        from unittest.mock import patch
        with patch.object(self.fs, '_journal', side_effect=OSError(errno.ENOSPC, 'synthetic disk full')):
            with self.assertRaises(OSError):
                self.fs.create(1, 'cannot-create.txt')
        self.assertNotIn('cannot-create.txt', self.fs.children[1])
        self.assertEqual(self.fs.handles, {})
        self.assertEqual(list(self.fs.recovery.iterdir()), [])
        self.assertEqual(self.backend.uploads, 0)

    def test_read_only_truncate_is_rejected(self):
        with self.assertRaises(OnDemandError) as cm:
            self.fs.open(self.inode, os.O_RDONLY | os.O_TRUNC)
        self.assertEqual(cm.exception.errno, errno.EINVAL)
        self.assertEqual(self.backend.downloads, 0)

    def test_hardlinked_cache_object_is_rejected(self):
        self.fs.pin(self.inode)
        cached = list(self.fs.objects.iterdir())[0]
        os.link(cached, Path(self.temp.name) / 'linked-copy')
        with self.assertRaises(OnDemandError):
            self.fs.open(self.inode)

    def test_metadata_event_invalidation(self):
        self.fs.listdir()
        self.backend.add('external.txt', b'other device')
        self.assertEqual(len(self.fs.listdir()), 1)
        self.fs.invalidate(object())
        self.assertEqual(len(self.fs.listdir()), 2)


if __name__ == '__main__':
    unittest.main()
