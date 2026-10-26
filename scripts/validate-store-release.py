#!/usr/bin/env python3
"""Validate a GitHub release Snap before exposing Store credentials to upload."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def normalized(version):
    return version.removeprefix('v').replace('-beta.', '-beta')


def validate(release, tag, package, checksums, metadata):
    if release.get('draft') or not release.get('published_at') or release.get('tag_name') != tag:
        raise ValueError('Expected the requested published GitHub release')
    if not re.fullmatch(r'v\d+\.\d+\.\d+-beta\.?\d+', tag):
        raise ValueError('This publisher accepts beta releases only')
    for field, expected in {'name': 'neutrondrive', 'confinement': 'strict', 'grade': 'devel',
                            'base': 'core24', 'summary': 'Unofficial Proton Drive client for Linux'}.items():
        if metadata.get(field) != expected:
            raise ValueError(f'Unexpected Snap {field}')
    if normalized(str(metadata.get('version', ''))) != normalized(tag):
        raise ValueError('Snap version does not match release tag')
    architectures = metadata.get('architectures', [])
    if architectures != ['amd64']:
        raise ValueError('Expected an amd64 Snap')
    names = [asset['name'] for asset in release.get('assets', [])]
    if names.count(package.name) != 1 or names.count('SHA256SUMS') != 1:
        raise ValueError('Package and checksums must be assets of this release')
    matches = []
    for line in checksums.splitlines():
        match = re.fullmatch(r'([0-9a-fA-F]{64}) [ *](.+)', line)
        if match and match[2].removeprefix('./') == package.name:
            matches.append(match[1].lower())
    digest_state = hashlib.sha256()
    with package.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest_state.update(chunk)
    digest = digest_state.hexdigest()
    if matches != [digest]:
        raise ValueError('Missing, duplicate or mismatching Snap checksum')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release', type=Path, required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--directory', type=Path, required=True)
    args = parser.parse_args()
    packages = list(args.directory.glob('*_amd64.snap'))
    if len(packages) != 1:
        raise ValueError('Expected exactly one amd64 Snap release asset')
    import yaml
    metadata = yaml.safe_load(subprocess.check_output(
        ['unsquashfs', '-cat', str(packages[0]), 'meta/snap.yaml'], text=True))
    validate(json.loads(args.release.read_text()), args.tag, packages[0],
             (args.directory / 'SHA256SUMS').read_text(), metadata)
    print(packages[0].resolve())


if __name__ == '__main__':
    main()
