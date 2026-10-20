"""Opt-in real Linux-kernel FUSE tests; fake backend, no account or cloud writes.

Run with NEUTRONDRIVE_FUSE_TEST=1 and python3-pyfuse3, python3-trio,
fuse3 installed. /dev/fuse must be available to the invoking user.
"""
import errno
import os
from pathlib import Path
import tempfile
import time
import unittest

from neutrondrive.fuse import MountManager, availability
from test_ondemand import FakeBackend


@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_FUSE_TEST') == '1', 'opt-in kernel FUSE tests')
class KernelMountTests(unittest.TestCase):
    def setUp(self):
        available, reason = availability()
        if not available:
            self.fail(reason)  # Requested kernel testing must not silently skip.
        self.temp = tempfile.TemporaryDirectory(prefix='neutrondrive-fuse-kernel-')
        self.directory = Path(self.temp.name)
        self.mount = self.directory / 'mount'
        self.backend = FakeBackend()
        self.manager = MountManager(self.backend, self.directory / 'state')
        self.addCleanup(self._cleanup)

    def _cleanup(self):
        self.manager.unmount()
        self.temp.cleanup()

    def test_kernel_stat_read_pin_evict_and_offline_remount(self):
        self.manager.mount(self.mount)
        cloud = self.mount / 'hello.txt'
        self.assertEqual(sorted(p.name for p in self.mount.iterdir()), ['hello.txt'])
        self.assertEqual(cloud.stat().st_size, 21)
        self.assertEqual(self.backend.downloads, 0)
        self.assertEqual(os.getxattr(cloud, 'user.neutrondrive.state'), b'online-only')
        self.assertEqual(cloud.read_bytes(), b'hello from the cloud\n')
        self.assertEqual(self.backend.downloads, 1)
        self.manager.pin('hello.txt')
        self.assertEqual(os.getxattr(cloud, 'user.neutrondrive.state'), b'pinned')
        self.manager.unmount()
        self.backend.online = False
        self.manager.mount(self.mount, offline=True)
        self.assertEqual(cloud.read_bytes(), b'hello from the cloud\n')
        self.manager.pin('hello.txt', False)
        deadline = time.monotonic() + 5
        while True:
            try:
                self.manager.evict('hello.txt')
                break
            except OSError as exc:
                if exc.errno != errno.EBUSY or time.monotonic() >= deadline:
                    raise
                time.sleep(0.02)
        with self.assertRaises(OSError) as cm:
            cloud.read_bytes()
        self.assertEqual(cm.exception.errno, errno.ENETDOWN)

    def test_kernel_create_write_revision_rename_move_trash(self):
        self.manager.mount(self.mount, writable=True)
        cloud = self.mount / 'hello.txt'
        with cloud.open('r+b', buffering=0) as output:
            output.truncate(0)
            output.write(b'edited in an application')
            os.fsync(output.fileno())
        self.assertEqual(self.backend.content['node1'], b'edited in an application')
        temporary = self.mount / '.editor-save'
        temporary.write_bytes(b'editor temp plus replace')
        os.replace(temporary, cloud)
        self.assertEqual(cloud.read_bytes(), b'editor temp plus replace')
        self.assertEqual(self.backend.content['node1'], b'editor temp plus replace')
        self.assertFalse(temporary.exists())
        new = self.mount / 'created.txt'
        with new.open('wb', buffering=0) as output:
            output.write(b'new from Linux')
            os.fsync(output.fileno())
        self.assertEqual(new.read_bytes(), b'new from Linux')
        renamed = self.mount / 'renamed.txt'
        new.rename(renamed)
        folder = self.mount / 'Folder'
        folder.mkdir()
        moved = folder / 'renamed.txt'
        renamed.rename(moved)
        self.assertEqual(moved.read_bytes(), b'new from Linux')
        moved.unlink()
        folder.rmdir()
        self.assertEqual(sorted(p.name for p in self.mount.iterdir()), ['hello.txt'])
        self.assertEqual(self.manager.status()['recovery'], [])

    def test_kernel_account_change_prevents_upload_and_preserves_recovery(self):
        from dataclasses import replace
        self.manager.mount(self.mount, writable=True)
        fd = os.open(self.mount / 'hello.txt', os.O_RDWR | os.O_TRUNC)
        os.write(fd, b'belongs only to original account')
        self.backend.nodes['root'] = replace(self.backend.nodes['root'], uid='changed-account')
        try:
            with self.assertRaises(OSError) as cm:
                os.fsync(fd)
            self.assertEqual(cm.exception.errno, errno.EACCES)
        finally:
            try:
                os.close(fd)
            except OSError:
                pass
        self.assertEqual(self.backend.uploads, 0)
        self.assertTrue(self.manager.status()['read_only'])
        self.manager.unmount()
        recovery = self.manager.status()['recovery']
        self.assertEqual(Path(recovery[0]['data_path']).read_bytes(), b'belongs only to original account')

    def test_kernel_readonly_blocks_writes_and_unpinned_offline_open(self):
        self.manager.mount(self.mount)
        cloud = self.mount / 'hello.txt'
        self.assertEqual(cloud.stat().st_size, 21)
        with self.assertRaises(OSError) as cm:
            cloud.write_bytes(b'forbidden')
        self.assertEqual(cm.exception.errno, errno.EROFS)
        self.assertEqual(self.backend.downloads, 0)
        self.manager.unmount()
        self.manager.mount(self.mount, offline=True)
        with self.assertRaises(OSError) as cm:
            cloud.read_bytes()
        self.assertEqual(cm.exception.errno, errno.ENETDOWN)

    def test_kernel_conflicting_fsync_returns_error_and_keeps_recovery(self):
        self.manager.mount(self.mount, writable=True)
        cloud = self.mount / 'hello.txt'
        fd = os.open(cloud, os.O_RDWR | os.O_TRUNC)
        os.write(fd, b'local recovery bytes')
        self.backend.update('node1', b'concurrent remote edit')
        try:
            with self.assertRaises(OSError) as cm:
                os.fsync(fd)
            self.assertEqual(cm.exception.errno, errno.ESTALE)
        finally:
            try:
                os.close(fd)
            except OSError:
                pass  # flush also returns the conflict on close.
        deadline = time.monotonic() + 5
        while not self.manager.status()['recovery'] and time.monotonic() < deadline:
            time.sleep(0.02)
        recovery = self.manager.status()['recovery']
        self.assertEqual(len(recovery), 1)
        self.assertEqual(Path(recovery[0]['data_path']).read_bytes(), b'local recovery bytes')
        self.assertEqual(self.backend.content['node1'], b'concurrent remote edit')


if __name__ == '__main__':
    unittest.main()
