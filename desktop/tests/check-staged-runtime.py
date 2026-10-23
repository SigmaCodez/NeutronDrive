#!/usr/bin/env python3
"""Run the staged native payload without development/Python runtime paths."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1]).resolve()
app = root / 'lib/neutrondrive'
binary = app / 'bin/neutrondrive-qt'
for path in app.rglob('*'):
    if path.is_file():
        assert path.suffix not in ('.py', '.pyc', '.pyi'), str(path)
        assert not path.name.startswith(('libpyside', 'libshiboken', 'libpython')), str(path)
with tempfile.TemporaryDirectory(prefix='nd-staged-native-check-') as temporary:
    env = dict(os.environ, PATH='/nonexistent', QT_QPA_PLATFORM='offscreen',
               XDG_STATE_HOME=temporary+'/state', XDG_CONFIG_HOME=temporary+'/config',
               XDG_CACHE_HOME=temporary+'/cache', DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent')
    for key in ('LD_LIBRARY_PATH', 'LD_PRELOAD', 'QT_PLUGIN_PATH', 'QML_IMPORT_PATH',
                'QML2_IMPORT_PATH', 'DISPLAY', 'WAYLAND_DISPLAY', 'APPDIR', 'APPIMAGE'):
        env.pop(key, None)
    version = subprocess.run([str(binary), '--version'], env=env, text=True, capture_output=True, timeout=30)
    assert version.returncode == 0, version.stderr
    result = subprocess.run([str(binary), '--check-window', str(app/'neutrondrive/locales')],
                            env=env, text=True, capture_output=True, timeout=45)
    assert result.returncode == 0, (result.returncode, result.stderr)
    assert 'NATIVE_MAIN_WINDOW_OK' in result.stderr, result.stderr
    print('STAGED_NATIVE_RUNTIME_NO_PYTHON_OK')
    print(version.stdout.strip())
