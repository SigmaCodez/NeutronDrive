#!/usr/bin/env python3
"""Build-time integration fixture; the application subprocess has no Python PATH."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='nd-native-appimage-') as temporary:
    root = Path(temporary)
    source = root / 'mounted image with spaces'
    source.mkdir()
    launcher = source / 'AppRun'
    launcher.write_text('#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- "${0%/*}" && pwd -P)\nexec ' + shlex.quote(str(binary)) + ' --appimage-bootstrap "$root" "$@"\n')
    launcher.chmod(0o700)
    manifest = {'AppRun': hashlib.sha256(launcher.read_bytes()).hexdigest()}
    (source / 'runtime-manifest.json').write_text(json.dumps(manifest))
    env = dict(os.environ, PATH='/no-programs', XDG_CACHE_HOME=str(root / 'cache'),
               XDG_STATE_HOME=str(root / 'state'), XDG_CONFIG_HOME=str(root / 'config'),
               QT_QPA_PLATFORM='invalid-platform')
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    def run(path):
        return subprocess.run([str(path), '--version'], env=env, capture_output=True, text=True, timeout=30)
    first = run(launcher)
    assert first.returncode == 0, first.stderr
    assert first.stdout.startswith('NeutronDrive '), first.stdout
    cache = root / 'cache/neutrondrive-appimage'
    targets = [p for p in cache.iterdir() if p.is_dir()]
    assert len(targets) == 1, targets
    target = targets[0]
    shutil.rmtree(source)
    second = run(target / 'AppRun')
    assert second.returncode == 0 and second.stdout == first.stdout, second
    (root / 'cache-alias').symlink_to(root / 'cache', target_is_directory=True)
    env['XDG_CACHE_HOME'] = str(root / 'cache-alias')
    aliased = run(target / 'AppRun')
    assert aliased.returncode == 0 and aliased.stdout == first.stdout, aliased
    # Invoke the trusted binary directly to check corrupt cache handling. Never
    # execute the corrupted launcher itself in the regression test.
    (target / 'AppRun').write_text('corrupted launcher')
    corrupt = subprocess.run([str(binary), '--appimage-bootstrap', str(target), '--version'],
                             env=env, capture_output=True, text=True, timeout=30)
    assert corrupt.returncode == 1 and 'damaged' in corrupt.stderr, corrupt
    assert (target / 'AppRun').read_text() == 'corrupted launcher'
print('NATIVE_APPIMAGE_STABLE_CACHE_OK')
