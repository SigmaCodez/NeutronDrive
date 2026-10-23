"""Process-based UI regression support; never imports an application binding."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def native_binary():
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'build/native-qt-target'))
    binary = Path(os.environ.get('NEUTRONDRIVE_NATIVE_EXECUTABLE', target / 'debug/neutrondrive-qt'))
    if not binary.is_file():
        raise AssertionError('Build the Rust Qt frontend before enabling UI tests')
    return binary.resolve()


def check(mode, fixture=False):
    binary = native_binary()
    runtime = Path(os.environ.get('NEUTRONDRIVE_QT_RUNTIME', ROOT / 'build/qt/PySide6/Qt'))
    with tempfile.TemporaryDirectory(prefix='nd-native-ui-regression-') as temporary:
        root = Path(temporary)
        env = dict(os.environ, QT_QPA_PLATFORM='offscreen',
                   XDG_CONFIG_HOME=str(root/'config'), XDG_STATE_HOME=str(root/'state'),
                   XDG_CACHE_HOME=str(root/'cache'), XDG_RUNTIME_DIR=str(root/'runtime'))
        (root/'runtime').mkdir(mode=0o700)
        for name in ('SNAP', 'FLATPAK_ID', 'APPIMAGE', 'APPDIR'):
            env.pop(name, None)
        if runtime.is_dir():
            env['LD_LIBRARY_PATH'] = str(runtime/'lib')
            env['QT_PLUGIN_PATH'] = str(runtime/'plugins')
            env['QML2_IMPORT_PATH'] = str(runtime/'qml')
        if fixture:
            args = ['/bin/bash', str(ROOT/'desktop/tests/check-window-fixture.sh'),
                    str(binary), str(ROOT/'src/neutrondrive/locales')]
        else:
            args = ['/usr/bin/env', 'PATH=/nonexistent', str(binary), mode]
            if mode == '--check-window':
                args.append(str(ROOT/'src/neutrondrive/locales'))
        result = subprocess.run(['dbus-run-session', '--config-file='+str(ROOT/'desktop/tests/session.conf'),
                                 '--', *args], env=env, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise AssertionError(result.stdout + result.stderr)
        return result.stdout + result.stderr
