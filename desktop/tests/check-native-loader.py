#!/usr/bin/env python3
"""Exercise the installed ELF layout without a launcher library environment."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

binary, runtime = map(lambda value: Path(value).resolve(), sys.argv[1:])
with tempfile.TemporaryDirectory(prefix='nd-native-loader-') as temporary:
    root = Path(temporary)
    (root / 'bin').mkdir()
    installed = root / 'bin/neutrondrive-qt'
    shutil.copy2(binary, installed)
    (root / 'qt').symlink_to(runtime, target_is_directory=True)
    dynamic = subprocess.check_output(['readelf', '-d', str(installed)], text=True)
    assert '$ORIGIN/../qt/lib' in dynamic, dynamic
    env = dict(os.environ, PATH='/no-programs', XDG_STATE_HOME=str(root / 'state'),
               XDG_CACHE_HOME=str(root / 'cache'), XDG_CONFIG_HOME=str(root / 'config'),
               QT_QPA_PLATFORM='invalid-platform')
    for key in ('LD_LIBRARY_PATH', 'LD_PRELOAD', 'QT_PLUGIN_PATH', 'QML_IMPORT_PATH', 'QML2_IMPORT_PATH', 'DISPLAY', 'WAYLAND_DISPLAY'):
        env.pop(key, None)
    result = subprocess.run([str(installed), '--version'], env=env, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr
    assert result.stdout.startswith('NeutronDrive '), result.stdout
print('NATIVE_DIRECT_ELF_LOADER_OK')
