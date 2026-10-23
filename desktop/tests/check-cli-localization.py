#!/usr/bin/env python3
"""Exercise native CLI language selection without a GUI, SDK or Python on PATH."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
catalogs = Path(sys.argv[2]).resolve()
header = 'NeutronDrive — unabhängiger Proton Drive Client für Linux'
no_display = 'No graphical session is available. Start NeutronDrive in a Wayland or X11 session.'
english = json.loads((catalogs / 'en.json').read_text())
with tempfile.TemporaryDirectory(prefix='nd-native-cli-languages-') as temporary:
    root = Path(temporary)
    for code in sorted(path.stem for path in catalogs.glob('*.json')) + ['ja-JP']:
        selected = 'en' if code == 'ja-JP' else code
        translated = json.loads((catalogs / (selected + '.json')).read_text())
        env = dict(os.environ, PATH='/nonexistent', LC_ALL=code.replace('-', '_')+'.UTF-8',
                   LANG=code.replace('-', '_')+'.UTF-8', LANGUAGE='', QT_QPA_PLATFORM='invalid-platform',
                   XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'),
                   XDG_CACHE_HOME=str(root/'cache'))
        env.pop('DISPLAY', None)
        env.pop('WAYLAND_DISPLAY', None)
        result = subprocess.run([str(binary), '--help'], env=env, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, (code, result.stderr)
        assert translated.get(header, english[header]) in result.stdout, (code, result.stdout)
        for flag in ('--background', '--mount-foreground DIR', '--state-dir DIR', '--offline', '--cli PATH'):
            assert flag in result.stdout, (code, flag)
        result = subprocess.run([str(binary), '--demo'], env=env, capture_output=True, text=True, timeout=15)
        assert result.returncode == 1, (code, result)
        assert translated.get(no_display, english[no_display]) in result.stderr, (code, result.stderr)
    assert not list(root.iterdir()), 'Help and no-display errors must not create account/config/state files'
print('NATIVE_CLI_ALL_LANGUAGES_AND_ENGLISH_FALLBACK_OK')
