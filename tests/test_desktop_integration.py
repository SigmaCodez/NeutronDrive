import os
import errno
import json
import signal
import subprocess
import threading
import time
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from neutrondrive.backend import BackendError
from neutrondrive import desktop_integration as integration
from neutrondrive.desktop_service import ServiceCore, DesktopClient


class FakeManager:
    def __init__(self, *args):
        self.mounted = False
        self.calls = []

    def status(self):
        return {'mounted': self.mounted}

    def mount(self, **kwargs):
        self.mounted = True
        self.calls.append(('mount', kwargs))
        return self.status()

    def unmount(self):
        self.mounted = False

    def pin(self, path, pinned=True):
        if path == 'slow':
            time.sleep(1)

    def evict(self, path):
        pass

    def refresh(self):
        pass

    def invalidate(self, event):
        self.calls.append(('invalidate', event.kind))


class FakeBackend:
    def reset_cancel(self):
        pass

    def cancel(self):
        pass


class DesktopIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.environment = patch.dict(os.environ, {
            'XDG_CONFIG_HOME': str(self.root / 'config'),
            'XDG_STATE_HOME': str(self.root / 'state'),
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_autostart_roundtrip_and_literal_argument_escaping(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(integration, 'autostart_path', return_value=self.root / 'app.desktop'), patch.object(integration, 'app_command', return_value=['/tmp/my app/$x%f`name', '--background']):
            integration.set_autostart(True)
            text = (self.root / 'app.desktop').read_text()
            self.assertIn('\\\\$x%%f\\\\`name', text)
            self.assertTrue(integration.autostart_enabled())
            integration.set_autostart(False)
            self.assertFalse((self.root / 'app.desktop').exists())

    def test_autostart_does_not_replace_foreign_file_or_symlink(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(integration, 'autostart_path', return_value=self.root / 'app.desktop'):
            (self.root / 'app.desktop').write_text('foreign')
            with self.assertRaises(BackendError):
                integration.set_autostart(False)
            (self.root / 'app.desktop').unlink()
            (self.root / 'target').write_text('preserve')
            (self.root / 'app.desktop').symlink_to(self.root / 'target')
            with self.assertRaises(BackendError):
                integration.set_autostart(True)
            self.assertEqual((self.root / 'target').read_text(), 'preserve')

    def test_desktop_argument_newlines_rejected(self):
        with self.assertRaises(BackendError):
            integration.desktop_quote('bad\nExec=other')

    def test_service_rejects_arbitrary_actions_and_invalid_mount_arguments(self):
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        for request in (
            {'action': 'shell', 'command': 'anything'},
            {'action': 'mount', 'mountpoint': '/tmp', 'writable': 'false'},
            {'action': 'mount', 'mountpoint': '/tmp', 'remote_root': '/trash'},
            {'action': 'mount', 'mountpoint': '/tmp', 'command': 'anything'},
        ):
            with self.subTest(request=request), self.assertRaises(BackendError):
                core.request(request)
        self.assertEqual(core.manager.calls, [])

    def test_mount_config_persists_and_unmount_disables_restore(self):
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        core.request({'action': 'mount', 'mountpoint': str(self.root / 'virtual'), 'offline': True})
        self.assertTrue(core.configuration_value()['restore'])
        self.assertTrue(core.request({'action': 'status'})['mounted'])
        self.assertEqual(core.configuration.stat().st_mode & 0o777, 0o600)
        core.request({'action': 'unmount'})
        self.assertFalse(core.configuration_value()['restore'])
        self.assertFalse(core.request({'action': 'status'})['mounted'])

    def test_service_configuration_rejects_links_and_unsafe_modes(self):
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        outside = self.root / 'outside'
        outside.write_text('{}')
        core.configuration.symlink_to(outside)
        with self.assertRaises(BackendError):
            core.configuration_value()
        core.configuration.unlink()
        core.configuration.write_text('{}')
        core.configuration.chmod(0o644)
        with self.assertRaises(BackendError):
            core.configuration_value()

    def test_mount_config_failure_rolls_back_mount(self):
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        with patch.object(core, '_save', side_effect=OSError(errno.ENOSPC, 'synthetic full disk')):
            with self.assertRaises(OSError):
                core.request({'action': 'mount', 'mountpoint': str(self.root / 'virtual'), 'offline': True})
        self.assertFalse(core.manager.mounted)

    def test_busy_unmount_preserves_event_watcher(self):
        class Events:
            stopped = False
            status = {'state': 'ready'}
            def stop(self):
                self.stopped = True
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        watcher = Events()
        core.events = watcher
        core.manager.mounted = True
        with patch.object(core.manager, 'unmount', side_effect=OSError(errno.EBUSY, 'busy')):
            with self.assertRaises(OSError):
                core.request({'action': 'unmount'})
        self.assertIs(core.events, watcher)
        self.assertFalse(watcher.stopped)
        self.assertTrue(core.manager.mounted)

    def test_event_stop_does_not_cancel_mount_backend(self):
        from neutrondrive.backend import Backend
        created = []
        class Events:
            running = False
            status = {'state': 'ready'}
            def __init__(self, backend, *args, **kwargs):
                self.backend = backend
                created.append(self)
            def start(self, callback):
                pass
            def stop(self):
                self.backend.cancel()
        backend = Backend('/nonexistent-test-cli')
        core = ServiceCore(backend, self.root / 'service', FakeManager, Events)
        core.request({'action': 'mount', 'mountpoint': str(self.root / 'virtual')})
        self.assertIsNot(created[0].backend, backend)
        self.assertIs(created[0].backend._mutex, backend._mutex)
        core.request({'action': 'unmount'})
        self.assertFalse(backend.cancel_event.is_set())

    def test_status_is_responsive_during_operation(self):
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        worker = threading.Thread(target=lambda: core.request({'action': 'pin', 'path': 'slow'}))
        worker.start()
        try:
            deadline = time.monotonic() + 2
            while not core.status()['busy'] and time.monotonic() < deadline:
                time.sleep(.01)
            started = time.monotonic()
            status = core.request({'action': 'status'})
            self.assertLess(time.monotonic() - started, .2)
            self.assertEqual(status['operation'], 'pin')
        finally:
            worker.join(3)

    def test_autostart_stale_tempfile_does_not_block_update(self):
        with patch.object(integration, 'app_command', return_value=['/usr/bin/neutrondrive', '--background']):
            integration.set_autostart(True)
            stale = integration.autostart_path().with_suffix('.desktop.tmp')
            stale.write_text('leftover interrupted update')
            integration.set_autostart(True)
            self.assertTrue(integration.autostart_enabled())
            self.assertEqual(stale.read_text(), 'leftover interrupted update')

    def test_snap_autostart_uses_declared_private_path_and_no_nested_snap_run(self):
        data = self.root / 'snap-user-data'
        with patch.dict(os.environ, {'SNAP': '/snap/neutrondrive/test', 'SNAP_USER_DATA': str(data)}):
            integration.set_autostart(True, '/should-not-be-persisted-cli')
            expected = data / '.config/autostart/io.github.SigmaCodez.NeutronDrive.desktop'
            self.assertEqual(integration.autostart_path(), expected)
            command = next(line[5:] for line in expected.read_text().splitlines() if line.startswith('Exec='))
            import shlex
            self.assertEqual(shlex.split(command), ['neutrondrive', '--background'])
            self.assertTrue(integration.autostart_enabled())

    def test_strict_snap_fuse_reports_native_package_requirement(self):
        from neutrondrive.fuse import availability
        with patch.dict(os.environ, {'SNAP': '/snap/neutrondrive/test'}):
            available, reason = availability()
        self.assertFalse(available)
        self.assertIn('native', reason)

    def test_staging_snap_omits_native_activation_and_native_retains_it(self):
        import importlib.util
        stage_path = Path(__file__).resolve().parents[1] / 'packaging/stage.py'
        spec = importlib.util.spec_from_file_location('packaging_stage_test', stage_path)
        stage = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(stage)
        binary = self.root / 'synthetic-backend'
        binary.write_bytes(b'synthetic payload; binary verification is tested separately')
        for native in (False, True):
            target = self.root / ('native' if native else 'snap')
            with patch.object(stage.backend, 'verify'):
                stage.stage(target, '/usr', 'x86_64', binary, host_services=native)
            activation = target / 'usr/share/dbus-1/services/io.github.SigmaCodez.NeutronDrive.Service.service'
            unit = target / 'usr/lib/systemd/user/neutrondrive-background.service'
            self.assertEqual(activation.exists(), native)
            self.assertEqual(unit.exists(), native)
            self.assertTrue((target / 'usr/bin/neutrondrive').is_file())
            from neutrondrive.i18n import SUPPORTED
            catalogs = target / 'usr/lib/neutrondrive/neutrondrive/locales'
            self.assertEqual({p.stem for p in catalogs.glob('*.json')}, set(SUPPORTED))
            source_catalogs = stage.ROOT / 'src/neutrondrive/locales'
            for code in SUPPORTED:
                self.assertEqual((catalogs / (code + '.json')).read_bytes(),
                                 (source_catalogs / (code + '.json')).read_bytes())
            translations = target / 'usr/lib/neutrondrive/qt/PySide6/Qt/translations'
            self.assertTrue((translations / 'qtbase_fr.qm').is_file())
            self.assertTrue((translations / 'qtdeclarative_fr.qm').is_file())

    def test_service_does_not_mount_inside_sync_folder(self):
        from neutrondrive.background import JobStore
        sync = self.root / 'sync'
        sync.mkdir()
        JobStore(self.root / 'state/neutrondrive/background').add(sync, '/my-files/test')
        core = ServiceCore(FakeBackend(), self.root / 'service', FakeManager)
        with self.assertRaisesRegex(BackendError, 'überlappen'):
            core.request({'action': 'mount', 'mountpoint': str(sync / 'virtual'), 'offline': True})
        self.assertEqual(core.manager.calls, [])


class BusIntegrationTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('NEUTRONDRIVE_DBUS_TEST') == '1', 'opt-in isolated session bus tests')
    def test_real_bus_transport_activation_fallback_and_responsive_status(self):
        for mode in ('roundtrip', 'activation', 'fallback', 'desktop-quote', 'snap-isolation'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix='neutrondrive-bus-') as directory:
                root = Path(directory)
                env = os.environ.copy()
                env.update({'XDG_STATE_HOME': str(root / 'state'), 'XDG_CONFIG_HOME': str(root / 'config'),
                            'XDG_DATA_HOME': str(root / 'data'), 'PYTHONPATH': str(Path(__file__).resolve().parents[1] / 'src') + os.pathsep + env.get('PYTHONPATH', '')})
                for key in ('FLATPAK_ID', 'SNAP', 'SNAP_USER_DATA', 'SNAP_USER_COMMON'):
                    env.pop(key, None)
                if mode == 'activation':
                    service_dir = root / 'data/dbus-1/services'
                    service_dir.mkdir(parents=True)
                    from neutrondrive.desktop_service import BUS_NAME
                    command = [sys.executable, str(Path(__file__).resolve()), '--dbus-server', str(root)]
                    import shlex
                    (service_dir / (BUS_NAME + '.service')).write_text('[D-BUS Service]\nName=' + BUS_NAME + '\nExec=' + shlex.join(command) + '\n')
                completed = subprocess.run(['dbus-run-session', '--', sys.executable, str(Path(__file__).resolve()),
                                            '--dbus-case', mode, str(root)], env=env, capture_output=True, text=True, timeout=20)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertIn('BUS_TEST_OK', completed.stdout)

    @unittest.skipUnless(os.environ.get('NEUTRONDRIVE_DBUS_TEST') == '1' and os.environ.get('NEUTRONDRIVE_FUSE_TEST') == '1', 'opt-in real D-Bus to FUSE kernel test')
    def test_real_bus_mounts_kernel_filesystem_without_cloud_credentials(self):
        with tempfile.TemporaryDirectory(prefix='neutrondrive-bus-kernel-') as directory:
            root = Path(directory)
            env = os.environ.copy()
            env.update({'XDG_STATE_HOME': str(root / 'state'), 'XDG_CONFIG_HOME': str(root / 'config'),
                        'XDG_DATA_HOME': str(root / 'data')})
            for key in ('FLATPAK_ID', 'SNAP', 'SNAP_USER_DATA', 'SNAP_USER_COMMON'):
                env.pop(key, None)
            result = subprocess.run(['dbus-run-session', '--', sys.executable, str(Path(__file__).resolve()),
                                     '--dbus-case', 'kernel', directory], env=env, capture_output=True, text=True, timeout=25)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('BUS_TEST_OK', result.stdout)


def _bus_case(mode, directory):
    from neutrondrive.desktop_service import BUS_NAME, OBJECT_PATH, INTERFACE, _gio
    Gio, GLib = _gio()
    root = Path(directory)
    client = DesktopClient()
    child = None
    service_pid = None
    try:
        assert client.request('status') == {'available': False, 'mounted': False}
        if mode == 'desktop-quote':
            script = root / 'capture.py'
            output = root / 'arguments.json'
            script.write_text('import json,sys;open(sys.argv[1],"w").write(json.dumps(sys.argv[2:]))')
            payload = 'literal \\ \" $HOME %f `touch-does-not-run`'
            with patch.object(integration, 'app_command', return_value=[sys.executable, str(script), str(output), payload]):
                integration.set_autostart(True)
            app = Gio.DesktopAppInfo.new_from_filename(str(integration.autostart_path()))
            assert app is not None
            app.launch([], None)
            deadline = time.monotonic() + 5
            while not output.exists() and time.monotonic() < deadline:
                time.sleep(.02)
            assert json.loads(output.read_text()) == [payload]
            print('BUS_TEST_OK')
            return
        if mode in {'roundtrip', 'kernel', 'snap-isolation'}:
            server_mode = '--dbus-kernel-server' if mode == 'kernel' else '--dbus-server'
            child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), server_mode, str(root)],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            deadline = time.monotonic() + 5
            while not client.request('status').get('available') and time.monotonic() < deadline:
                time.sleep(.02)
            assert client.request('status')['available']
        else:
            assert not (root / 'activated').exists()
            assert not client.request('restore').get('mounted')
            if mode == 'activation':
                assert (root / 'activated').exists()
        connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        service_pid = connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                                          'GetConnectionUnixProcessID', GLib.Variant('(s)', (BUS_NAME,)),
                                          GLib.VariantType.new('(u)'), Gio.DBusCallFlags.NONE, 3000, None).unpack()[0]
        if mode == 'snap-isolation':
            assert client.request('mount', mountpoint=str(root / 'native-mount'), offline=True)['mounted']
            env = os.environ.copy()
            env.update({'SNAP': '/snap/neutrondrive/test', 'SNAP_USER_DATA': str(root / 'snap-data')})
            result = subprocess.run([sys.executable, str(Path(__file__).resolve()), '--snap-restore-case'],
                                    env=env, capture_output=True, text=True, timeout=10)
            assert result.returncode == 0, result.stderr
            assert 'SNAP_ISOLATION_OK' in result.stdout
            assert client.request('status')['mounted']
            client.request('unmount')
            print('BUS_TEST_OK')
            return
        if mode == 'kernel':
            mount = root / 'mount'
            assert client.request('mount', mountpoint=str(mount), writable=True)['mounted']
            cloud = mount / 'hello.txt'
            assert cloud.stat().st_size == 21
            assert os.getxattr(cloud, 'user.neutrondrive.state') == b'online-only'
            assert cloud.read_bytes() == b'hello from the cloud\n'
            client.request('pin', path='hello.txt')
            assert os.getxattr(cloud, 'user.neutrondrive.state') == b'pinned'
            client.request('unmount')
            client.request('mount', mountpoint=str(mount), offline=True)
            assert cloud.read_bytes() == b'hello from the cloud\n'
            deadline = time.monotonic() + 5
            while True:
                try:
                    client.request('evict', path='hello.txt')
                    break
                except BackendError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.02)
            try:
                cloud.read_bytes()
            except OSError as exc:
                assert exc.errno == errno.ENETDOWN
            else:
                raise AssertionError('Evicted file read as if content existed')
            client.request('unmount')
            print('BUS_TEST_OK')
            return
        if mode != 'fallback':
            result = client.request('mount', mountpoint=str(root / 'mount'), offline=True)
            assert result['mounted']
            errors = []
            def pin():
                try:
                    client.request('pin', path='slow')
                except Exception as exc:
                    errors.append(type(exc).__name__)
            worker = threading.Thread(target=pin)
            worker.start()
            deadline = time.monotonic() + 3
            while not client.request('status').get('busy') and time.monotonic() < deadline:
                time.sleep(.01)
            started = time.monotonic()
            assert client.request('status')['operation'] == 'pin'
            assert time.monotonic() - started < .4
            worker.join(5)
            assert not errors
            try:
                connection.call_sync(BUS_NAME, OBJECT_PATH, INTERFACE, 'Request', GLib.Variant('(s)', ('[]',)),
                                     GLib.VariantType.new('(s)'), Gio.DBusCallFlags.NONE, 3000, None)
            except GLib.Error:
                pass
            else:
                raise AssertionError('Malformed request accepted')
            assert not client.request('unmount')['mounted']
            assert not client.request('restore')['mounted']
        print('BUS_TEST_OK')
    finally:
        if child is not None:
            child.terminate()
            child.communicate(timeout=10)
        elif service_pid is not None:
            os.kill(service_pid, signal.SIGTERM)


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--snap-restore-case':
        from neutrondrive.desktop_service import BUS_NAME, _gio
        assert BUS_NAME.endswith('.SnapService')
        client = DesktopClient()
        assert not client.request('status')['available']
        client.request('restore')
        Gio, GLib = _gio()
        connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        pid = connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                                   'GetConnectionUnixProcessID', GLib.Variant('(s)', (BUS_NAME,)),
                                   GLib.VariantType.new('(u)'), Gio.DBusCallFlags.NONE, 3000, None).unpack()[0]
        try:
            assert not client.request('status')['mounted']
            print('SNAP_ISOLATION_OK')
        finally:
            os.kill(pid, signal.SIGTERM)
        raise SystemExit(0)
    if len(sys.argv) > 1 and sys.argv[1] == '--dbus-kernel-server':
        from neutrondrive.desktop_service import run_service
        from test_ondemand import FakeBackend as CloudBackend
        class KernelBackend(CloudBackend):
            def cancel(self):
                pass
            def reset_cancel(self):
                pass
        class Events:
            running = False
            status = {'state': 'ready'}
            def __init__(self, *args, **kwargs):
                pass
            def start(self, callback):
                pass
            def stop(self):
                pass
        location = Path(sys.argv[2])
        core = ServiceCore(KernelBackend(), location / 'service', event_factory=Events)
        raise SystemExit(run_service(core=core))
    if len(sys.argv) > 1 and sys.argv[1] == '--dbus-server':
        from neutrondrive.desktop_service import run_service
        location = Path(sys.argv[2])
        (location / 'activated').write_text('started')
        core = ServiceCore(FakeBackend(), location / 'service', FakeManager)
        raise SystemExit(run_service(core=core))
    if len(sys.argv) > 1 and sys.argv[1] == '--dbus-case':
        _bus_case(sys.argv[2], sys.argv[3])
    else:
        unittest.main()

