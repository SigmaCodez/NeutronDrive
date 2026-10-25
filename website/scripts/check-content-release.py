#!/usr/bin/env python3
"""Compare published release content with the deployed website, never local state."""
import hashlib
import json
import sys
from datetime import datetime
from urllib.error import HTTPError
from urllib.request import Request, urlopen

API = 'https://api.github.com/repos/SigmaCodez/NeutronDrive/releases?per_page=100'
LIVE = 'https://neutrondrive.sigmacodez.dev/content-version.json'


def fetch(url):
    request = Request(url, headers={'User-Agent': 'NeutronDrive-content-review', 'Cache-Control': 'no-cache'})
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def latest(releases):
    valid = [r for r in releases if not r.get('draft') and r.get('published_at')]
    return max(valid, key=lambda r: datetime.fromisoformat(r['published_at'].replace('Z', '+00:00')))


def fingerprint(release):
    data = {k: release.get(k) for k in ('id', 'tag_name', 'name', 'body', 'prerelease', 'published_at', 'updated_at')}
    data['assets'] = sorted([
        {k: a.get(k) for k in ('name', 'size', 'updated_at', 'digest', 'state')}
        for a in release.get('assets', [])
    ], key=lambda a: a['name'])
    return hashlib.sha256(json.dumps(data, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def needs_update(release, deployed):
    return (deployed.get('release_fingerprint') != fingerprint(release)
            or deployed.get('screenshots_pending', False))


def main():
    release = latest(fetch(API))
    try:
        deployed = fetch(LIVE)
    except HTTPError as error:
        if error.code != 404:
            raise
        deployed = {}
    print(json.dumps({
        'needs_update': needs_update(release, deployed),
        'release_fingerprint': fingerprint(release),
        'release': release,
        'deployed': deployed,
    }, indent=2, ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'Release check failed: {error}', file=sys.stderr)
        sys.exit(1)
