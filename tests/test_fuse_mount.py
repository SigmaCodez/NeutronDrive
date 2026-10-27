"""Opt-in real Linux-kernel FUSE tests; fake backend, no account or cloud writes.

Run with NEUTRONDRIVE_FUSE_TEST=1, the native Rust core and fuse3 installed. /dev/fuse must be available to the invoking user.
"""
import errno
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

from core_contract.fuse import MountManager, availability
from test_ondemand import FakeBackend


@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_FUSE_TEST') == '1', 'opt-in kernel FUSE tests')
class KernelMountTests(unittest.TestCase):
    def setUp(self):
        available, reason = availability()
        if not available:
            self.fail(reason)  # Requested kernel testing must not silently skip.
        self.temp = tempfile.TemporaryDirectory(prefix='neutrondrive-fuse-kernel-')
        self.directory = Path(self.temp.name)
        # The real Rust FUSE startup also registers desktop bookmarks. Isolate
        # every user-profile path even when this test file runs on its own.
        profile = {key: str(self.directory / suffix) for key, suffix in (
            ('HOME', 'home'), ('XDG_CONFIG_HOME', 'config'),
            ('XDG_STATE_HOME', 'user-state'), ('XDG_CACHE_HOME', 'cache'),
            ('XDG_DATA_HOME', 'data'))}
        for path in profile.values():
            Path(path).mkdir(mode=0o700)
        environment = patch.dict(os.environ, profile)
        environment.start()
        self.addCleanup(environment.stop)
        self.mount = self.directory / 'mount'
        self.backend = FakeBackend()
        self.manager = MountManager(self.backend, self.directory / 'state')
        self.addCleanup(self._cleanup)

    def _cleanup(self):
        self.after_kernel_release(self.manager.unmount)
        self.temp.cleanup()

    def test_kernel_bookmark_stays_in_isolated_profile(self):
        self.manager.mount(self.mount)
        bookmarks = self.directory / "config/gtk-3.0/bookmarks"
        expected = self.mount.as_uri() + " Proton Drive\n"
        self.assertEqual(bookmarks.read_text(), expected)
        self.after_kernel_release(self.manager.unmount)
        self.manager.mount(self.mount)
        self.assertEqual(bookmarks.read_text(), expected)

    def after_kernel_release(self, operation):
        # close(2) may return before the asynchronous FUSE RELEASE is handled.
        # Only EBUSY is retryable, and a genuinely retained handle still fails.
        deadline = time.monotonic() + 5
        while True:
            try:
                return operation()
            except OSError as exc:
                if exc.errno != errno.EBUSY or time.monotonic() >= deadline:
                    raise
                time.sleep(0.02)

    def test_kernel_stat_read_pin_evict_and_offline_remount(self):
        self.manager.mount(self.mount)
        cloud = self.mount / 'hello.txt'
        self.assertEqual(sorted(p.name for p in self.mount.iterdir()), ['hello.txt'])
        self.assertEqual(cloud.stat().st_size, 21)
        self.assertEqual(self.backend.downloads, 0)
        self.assertEqual(os.getxattr(cloud, 'user.neutrondrive.state'), b'online-only')
        with cloud.open('rb') as opened:
            self.assertEqual(opened.read(), b'hello from the cloud\n')
            self.assertEqual(self.backend.downloads, 1)
            with self.assertRaises(OSError) as busy:
                self.manager.pin('hello.txt')
            self.assertEqual(busy.exception.errno, errno.EBUSY)
        self.after_kernel_release(lambda: self.manager.pin('hello.txt'))
        self.assertEqual(os.getxattr(cloud, 'user.neutrondrive.state'), b'pinned')
        self.manager.unmount()
        self.backend.online = False
        self.manager.mount(self.mount, offline=True)
        self.assertEqual(cloud.read_bytes(), b'hello from the cloud\n')
        self.manager.pin('hello.txt', False)
        self.after_kernel_release(lambda: self.manager.evict('hello.txt'))
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

    def test_status_is_responsive_during_native_hydration(self):
        import threading
        from concurrent.futures import ThreadPoolExecutor
        entered, resume = threading.Event(), threading.Event()
        original = self.backend.download
        def blocked(*args, **kwargs):
            entered.set()
            if not resume.wait(5):
                raise RuntimeError('Test hydration was not released')
            return original(*args, **kwargs)
        self.backend.download = blocked
        self.manager.mount(self.mount)
        with ThreadPoolExecutor(2) as pool:
            reading = pool.submit((self.mount / 'hello.txt').read_bytes)
            try:
                self.assertTrue(entered.wait(5))
                status = pool.submit(self.manager.status).result(timeout=1)
                self.assertTrue(status['mounted'])
            finally:
                resume.set()
            self.assertEqual(reading.result(timeout=5), b'hello from the cloud\n')

    def test_kernel_metadata_permissions_and_busy_unmount(self):
        self.manager.mount(self.mount, writable=True)
        cloud = self.mount / 'hello.txt'
        self.assertEqual(cloud.stat().st_mode & 0o777, 0o600)
        self.assertEqual(os.listxattr(cloud), ['user.neutrondrive.state'])
        with self.assertRaises(OSError) as error:
            os.chmod(cloud, 0o777)
        self.assertEqual(error.exception.errno, errno.ENOTSUP)
        with self.assertRaises(OSError) as error:
            os.getxattr(cloud, 'user.unknown')
        self.assertEqual(error.exception.errno, errno.ENODATA)
        handle = os.open(cloud, os.O_RDONLY)
        try:
            with self.assertRaises(OSError) as error:
                self.manager.unmount()
            self.assertEqual(error.exception.errno, errno.EBUSY)
            self.assertTrue(self.manager.status()['mounted'])
            self.assertEqual(os.read(handle, 100), b'hello from the cloud\n')
        finally:
            os.close(handle)
        self.manager.unmount()

    def test_native_session_callback_failure_unmounts_and_returns(self):
        from core_contract.backend import service_call
        from core_contract.ondemand import OnDemandFS
        self.mount.mkdir()
        fs = OnDemandFS(self.backend, self.directory / 'callback-cache')
        def rejected_ready():
            raise RuntimeError('deliberate callback failure')
        try:
            with self.assertRaisesRegex(RuntimeError, 'deliberate callback failure'):
                service_call(self.backend, 'fuse_session', {
                    'id': fs._id, 'mountpoint': str(self.mount),
                    'cache': str(fs.cache_dir), 'writable': False}, on_started=rejected_ready)
            self.assertFalse(os.path.ismount(self.mount))
        finally:
            fs.close()

    def test_kernel_production_backend_uses_native_sdk_transport(self):
        from unittest import mock
        from core_contract.backend import Backend
        sdk = self.directory / 'sdk'
        sdk.write_text('#!/bin/sh\nfor last do :; done\nfile=\'{"uid":"data","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}\'\nif [ "$2" = "download" ]; then printf abc > "$last/data"; printf \'%s\\n\' \'{}\'; exit 0; fi\nif [ "$2" = "list" ]; then printf \'[%s]\\n\' "$file"; exit 0; fi\ncase "$last" in /my-files|/my-files/root) printf \'%s\\n\' \'{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}\'; exit 0;; esac\nprintf \'%s\\n\' "$file"\n')
        sdk.chmod(0o700)
        backend = Backend(str(sdk))
        def forbidden(*args, **kwargs):
            raise AssertionError('Production mount called a Python SDK service')
        backend.info = backend.list_nodes = backend.download = forbidden
        self.manager = MountManager(backend, self.directory / 'native-state')
        with mock.patch.dict(os.environ, XDG_STATE_HOME=str(self.directory / 'sdk-state')):
            try:
                self.manager.mount(self.mount)
                self.assertEqual((self.mount / 'data').read_bytes(), b'abc')
                self.manager.pin('data')
            finally:
                self.manager.unmount()

    def test_native_manager_account_event_blocks_mount_and_closed_events_are_ignored(self):
        from types import SimpleNamespace
        self.manager.mount(self.mount, writable=True)
        self.manager.invalidate(SimpleNamespace(root_uid='different-root', kind='ready'))
        self.assertTrue(self.manager.status()['read_only'])
        with self.assertRaises(OSError):
            self.manager.refresh()
        with self.assertRaises(OSError):
            (self.mount / 'hello.txt').read_bytes()
        self.manager.unmount()
        before = self.manager.status()
        self.manager.invalidate(SimpleNamespace(root_uid='different-root', kind='ready'))
        self.assertEqual(self.manager.status(), before)

    def test_replaced_prepared_mountpoint_is_rejected_and_reservation_released(self):
        self.manager._call('prepare', mountpoint=str(self.mount), remote_root='/my-files', writable=False, offline=False)
        self.mount.rename(self.directory / 'previous-point')
        self.mount.mkdir()
        try:
            with self.assertRaises(OSError) as error:
                self.manager._call('run')
            self.assertEqual(error.exception.errno, errno.ESTALE)
            self.assertFalse(self.manager.status()['mounted'])
            self.manager.mount(self.mount)
            self.assertEqual((self.mount / 'hello.txt').read_bytes(), b'hello from the cloud\n')
        finally:
            self.manager._call('abort')

    def test_thread_start_failure_releases_native_cache_and_session(self):
        from unittest import mock
        with mock.patch('core_contract.fuse.threading.Thread.start', side_effect=RuntimeError('test start failure')):
            with self.assertRaisesRegex(RuntimeError, 'test start failure'):
                self.manager.mount(self.mount)
        self.assertFalse(self.manager.status()['mounted'])
        self.manager.mount(self.mount)
        other = MountManager(self.backend, self.directory / 'second-state')
        with self.assertRaises(OSError) as error:
            other.mount(self.directory / 'second-mount')
        self.assertEqual(error.exception.errno, errno.EBUSY)
        self.assertEqual((self.mount / 'hello.txt').read_bytes(), b'hello from the cloud\n')


if __name__ == '__main__':
    unittest.main()
