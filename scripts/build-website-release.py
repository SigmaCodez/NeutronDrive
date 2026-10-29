#!/usr/bin/env python3
"""Capture the shipped DEB and bundle reviewed homepage copy in six languages."""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('website_sync', ROOT/'website/scripts/sync-server-content.py')
validation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)
CAPTURES = {'files-grid-dark.png':'dark-grid.png', 'files-grid-light.png':'light-grid.png',
            'photos-light.png':'light-photos.png', 'computer-backup-light.png':'light-computer-backup.png',
            'sharing-light.png':'share-window-light.png'}


def version_key(value):
    return re.sub(r'[.\-]', '', value.removeprefix('NeutronDrive ').removeprefix('v'))


def reviewed_text(path, tag):
    document = json.loads(path.read_text())
    clean = validation.locales(document, tag)
    if set(clean) != set(validation.LANGUAGES):
        raise ValueError('Release website content requires en, de, fr, es, it and pt')
    for language, fields in clean.items():
        fields['faq'] = validation.release_faq(fields, tag, strict=True)
        if not {'description', 'features', 'faq', 'trust', 'views', 'releaseNotes'} <= fields.keys():
            raise ValueError(f'Incomplete homepage content: {language}')
        for key, value in fields.items():
            cells = [value] if isinstance(value, str) else [cell for row in value for cell in row]
            if any(not cell.strip() for cell in cells):
                raise ValueError(f'Empty translated text: {language}.{key}')
    return clean


def verify_package(package, checksums):
    matches = []
    for line in checksums.read_text().splitlines():
        match = re.fullmatch(r'([a-fA-F0-9]{64}) [ *](.+)', line)
        if match and match[2] == package.name:
            matches.append(match[1].lower())
    actual = hashlib.sha256(package.read_bytes()).hexdigest()
    if matches != [actual]:
        raise ValueError('Published DEB checksum missing, ambiguous or invalid')
    return actual


def run(command, env=None, timeout=90):
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f'{command[0]} failed:\n{result.stdout}\n{result.stderr}')
    return result.stdout + result.stderr


def build(args):
    tag = args.tag
    if not re.fullmatch(r'v[0-9][A-Za-z0-9._-]+', tag):
        raise ValueError('Expected a version release tag')
    if not re.fullmatch(r'[0-9a-f]{40}', args.commit):
        raise ValueError('Expected resolved release commit')
    content_source = args.content_source or args.source
    content_commit = args.content_commit or args.commit
    if bool(args.content_source) != bool(args.content_commit) or not re.fullmatch(r'[0-9a-f]{40}', content_commit):
        raise ValueError('Content override requires both a source path and resolved commit')
    content_path = content_source/'docs/website-content.json'
    correction = content_source/'docs/website-content-corrections'/f'{tag}.json'
    if args.content_source and correction.exists():
        content_path = correction
    localized = reviewed_text(content_path, tag)
    package_hash = verify_package(args.deb, args.checksums)
    with tempfile.TemporaryDirectory(prefix='nd-release-capture-') as temporary:
        root = Path(temporary)
        run(['dpkg-deb', '--extract', str(args.deb), str(root/'package')])
        app = root/'package/usr/lib/neutrondrive'
        binary = app/'bin/neutrondrive-qt'
        catalogs = app/'neutrondrive/locales'
        env = {k:v for k,v in os.environ.items() if k not in ('GH_TOKEN','GITHUB_TOKEN','SNAP','FLATPAK_ID','APPIMAGE','APPDIR','DISPLAY','WAYLAND_DISPLAY')}
        env.update(LANG='en_US.UTF-8', LC_ALL='en_US.UTF-8', LANGUAGE='en',
                   QT_QPA_PLATFORM='offscreen', QT_QUICK_BACKEND='software',
                   LD_LIBRARY_PATH=str(app/'qt/lib'), QT_PLUGIN_PATH=str(app/'qt/plugins'),
                   QML_IMPORT_PATH=str(app/'qt/qml'), QML2_IMPORT_PATH=str(app/'qt/qml'))
        version = run([str(binary), '--version'], env).strip()
        if version_key(version) != version_key(tag):
            raise ValueError(f'Package version {version!r} does not match {tag}')
        images = root/'images'; images.mkdir()
        bus = ['dbus-run-session', '--config-file='+str(args.source/'desktop/tests/session.conf'), '--']
        for name, command, marker in (
            ('main', [str(binary),'--check-window',str(catalogs),str(images),'en','My files'], 'NATIVE_MAIN_WINDOW_OK'),
            ('sharing', ['/bin/bash',str(args.source/'desktop/tests/check-window-fixture.sh'),str(binary),str(catalogs),str(images)], 'NATIVE_WINDOW_LITERAL_SDK_ARGUMENT_OK'),
        ):
            profile = root/name; profile.mkdir()
            runtime=profile/'runtime'; runtime.mkdir(mode=0o700)
            env.update(XDG_CONFIG_HOME=str(profile/'config'), XDG_STATE_HOME=str(profile/'state'),
                       XDG_CACHE_HOME=str(profile/'cache'), XDG_DATA_HOME=str(profile/'data'), XDG_RUNTIME_DIR=str(runtime))
            # Use the actual packaged English title; no hardcoded translated label.
            if name == 'main': command[-1] = json.loads((catalogs/'en.json').read_text())['Meine Dateien']
            output = run(bus+command, env)
            if marker not in output:
                raise ValueError(f'{name} capture did not complete its GUI checks')
        captured = {}
        for name, source in CAPTURES.items():
            data = (images/source).read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            validation.png(data, digest)
            captured[name] = {'sha256':digest, 'base64':base64.b64encode(data).decode()}
        bundle = {'schema':1,'release':tag,'source_commit':args.commit,'content_source_commit':content_commit,'locales':localized,
                  'previews':{'published_release':tag,'binary_version':version,
                    'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
                    'package':args.deb.name,'package_sha256':package_hash,
                    'captured_at':datetime.now(timezone.utc).isoformat(),
                    'language':'en','data':'Synthetic demo and sharing fixture data; no live account.',
                    'images':captured}}
        args.output.parent.mkdir(parents=True,exist_ok=True)
        payload=json.dumps(bundle,ensure_ascii=False,indent=2)+'\n'
        if len(payload.encode()) > 16_000_000:
            raise ValueError('Website bundle exceeds server size limit')
        validation.validate_bundle(bundle,tag,args.commit)
        args.output.write_text(payload)
        print(f'Generated {args.output}: six languages, five fresh verified PNGs, {version}')

if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--tag',required=True)
    parser.add_argument('--commit',required=True)
    parser.add_argument('--source',type=Path,default=ROOT)
    parser.add_argument('--content-source',type=Path,help='Reviewed text correction checkout; must match the same release tag')
    parser.add_argument('--content-commit',help='Resolved commit of the reviewed text correction')
    parser.add_argument('--deb',type=Path,required=True)
    parser.add_argument('--checksums',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    build(parser.parse_args())
