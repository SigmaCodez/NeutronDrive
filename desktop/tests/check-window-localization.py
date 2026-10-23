#!/usr/bin/env python3
"""Check system locale selection in the real Rust-backed Qt Quick window."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
catalogs = Path(sys.argv[2]).resolve()
english = json.loads((catalogs / 'en.json').read_text())
for code in sorted(path.stem for path in catalogs.glob('*.json')) + ['ja-JP']:
    selected = 'en' if code == 'ja-JP' else code
    translated = json.loads((catalogs / (selected + '.json')).read_text())
    title = translated.get('Meine Dateien', english['Meine Dateien'])
    with tempfile.TemporaryDirectory(prefix='nd-native-window-language-') as temporary:
        root = Path(temporary)
        env = dict(os.environ, PATH='/nonexistent', LC_ALL=code.replace('-', '_')+'.UTF-8',
                   LANG=code.replace('-', '_')+'.UTF-8', LANGUAGE='', QT_QPA_PLATFORM='offscreen',
                   XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'),
                   XDG_CACHE_HOME=str(root/'cache'), XDG_RUNTIME_DIR=str(root/'runtime'))
        (root/'runtime').mkdir(mode=0o700)
        for name in ('SNAP', 'FLATPAK_ID', 'APPIMAGE', 'APPDIR'):
            env.pop(name, None)
        result = subprocess.run([str(binary), '--check-window', str(catalogs), '', selected, title],
                                env=env, capture_output=True, text=True, timeout=25)
        output = result.stdout + result.stderr
        assert result.returncode == 0, (code, result.returncode, output)
        assert 'NATIVE_WINDOW_LANGUAGE_OK '+selected in output, (code, output)
        assert 'NATIVE_KEYBOARD_AND_MOUSE_OK' in output, (code, output)
        assert 'NATIVE_MAIN_WINDOW_OK' in output, (code, output)
print('NATIVE_MAIN_WINDOW_ALL_LANGUAGES_AND_ENGLISH_FALLBACK_OK')
