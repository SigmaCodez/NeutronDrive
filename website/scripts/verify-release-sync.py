#!/usr/bin/env python3
"""Confirm the live snapshot is the exact text/image bundle just published."""
import json
from pathlib import Path
import sys
from urllib.request import urlopen


def verify(bundle, live):
    for field in ('release', 'source_commit', 'locales'):
        if live.get(field) != bundle[field]:
            raise ValueError(f'Live {field} does not match published manifest')
    hashes = {name: image['sha256'] for name, image in bundle['previews']['images'].items()}
    if live.get('screenshots') != hashes or live.get('screenshots_release') != bundle['release'] or live.get('screenshots_pending') is not False:
        raise ValueError('Live screenshots do not match published manifest')


if __name__ == '__main__':
    bundle = json.loads(Path(sys.argv[1]).read_text())
    with urlopen('https://neutrondrive.sigmacodez.dev/live/current/content.json', timeout=30) as response:
        live = json.load(response)
    verify(bundle, live)
    print(f"Homepage text and screenshot snapshot verified: {bundle['release']}")
