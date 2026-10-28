#!/usr/bin/env python3
"""Publish GitHub release data atomically; never execute downloaded content."""
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import tempfile
from datetime import datetime, timezone
from urllib.error import HTTPError
from urllib.parse import quote, urlsplit
from urllib.request import Request, build_opener, HTTPRedirectHandler

REPO = 'SigmaCodez/NeutronDrive'
API = f'https://api.github.com/repos/{REPO}'
FILES = ('files-grid-dark.png', 'files-grid-light.png', 'photos-light.png', 'computer-backup-light.png', 'sharing-light.png')
LANGUAGES = ('en', 'de', 'fr', 'es', 'it')

class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('Unexpected source redirect')


def fetch(url, limit=2_000_000):
    parsed = urlsplit(url)
    if parsed.scheme != 'https' or parsed.hostname not in ('api.github.com', 'raw.githubusercontent.com'):
        raise ValueError('Unexpected source host')
    request = Request(url, headers={'User-Agent': 'NeutronDrive-Content-Sync', 'Accept': 'application/vnd.github+json'})
    with build_opener(NoRedirect).open(request, timeout=30) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError('Source exceeds size limit')
    return data


class AssetRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        destination = urlsplit(newurl)
        if destination.scheme != 'https' or destination.hostname != 'release-assets.githubusercontent.com' or destination.username or destination.password or destination.port not in (None,443):
            raise ValueError('Unexpected release asset redirect')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def validate_bundle(document, tag, commit):
    translated = locales(document, tag)
    if set(translated) != set(LANGUAGES) or document.get('source_commit') != commit:
        raise ValueError('Release bundle language set or source commit mismatch')
    for fields in translated.values():
        if not {'description', 'features', 'faq', 'trust', 'views', 'releaseNotes'} <= fields.keys():
            raise ValueError('Incomplete release homepage copy')
    previews = document['previews']
    if previews.get('published_release') != tag or previews.get('language') != 'en':
        raise ValueError('Release bundle screenshot provenance mismatch')
    normalize = lambda value: re.sub(r'[.\-]', '', value.removeprefix('NeutronDrive ').removeprefix('v'))
    if normalize(previews.get('binary_version', '')) != normalize(tag):
        raise ValueError('Screenshot binary version mismatch')
    images = {}
    for name in FILES:
        entry = previews['images'][name]
        images[name] = base64.b64decode(entry['base64'], validate=True)
        if len(images[name]) > 8_000_000:
            raise ValueError('Screenshot exceeds size limit')
        png(images[name], entry['sha256'])
    return translated, images


def release_bundle(release, commit):
    assets = [a for a in release.get('assets', []) if a.get('name') == 'neutrondrive-website.json']
    if not assets:
        return None
    if len(assets) != 1:
        raise ValueError('Ambiguous website bundle')
    asset = assets[0]
    if asset.get('state') != 'uploaded' or not 0 < asset.get('size', 0) <= 16_000_000 or type(asset.get('id')) is not int:
        raise ValueError('Incomplete or oversized website bundle')
    request = Request(f"{API}/releases/assets/{asset['id']}", headers={
        'User-Agent':'NeutronDrive-Content-Sync', 'Accept':'application/octet-stream'})
    with build_opener(AssetRedirect).open(request, timeout=30) as response:
        data = response.read(16_000_001)
    if len(data) != asset['size']:
        raise ValueError('Website bundle size mismatch')
    if asset.get('digest') and asset['digest'] != 'sha256:' + digest(data):
        raise ValueError('Website bundle digest mismatch')
    return validate_bundle(json.loads(data), release['tag_name'], commit)


def get_json(url, optional=False):
    try:
        return json.loads(fetch(url))
    except HTTPError as error:
        if optional and error.code == 404:
            return None
        raise


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False).encode()


def latest(releases):
    releases = [r for r in releases if not r.get('draft') and r.get('published_at')]
    result = max(releases, key=lambda r: datetime.fromisoformat(r['published_at'].replace('Z', '+00:00')))
    if not isinstance(result.get('tag_name'), str) or not result['tag_name'] or len(result['tag_name']) > 200:
        raise ValueError('Invalid release tag')
    return result


def text(value, limit=5000):
    if not isinstance(value, str) or len(value) > limit or 'codezwerk' in value.lower():
        raise ValueError('Invalid public text')
    return value



def release_faq(fields, tag, strict=False):
    """Schema 1: four general FAQs, three release FAQs, one contribution FAQ.

    Sources may omit the three derived rows (five rows total). Legacy bundles
    with eight rows are upgraded on ingestion. Release notes are the sole
    source for changes, upgrades and limits; never relabel an old answer.
    """
    faq = fields.get('faq', [])
    if len(faq) not in (5, 8):
        if strict:
            raise ValueError('Expected four general FAQs and one contribution FAQ')
        return faq
    general = faq[:4]
    contribution = faq[-1:]
    notes = fields.get('releaseNotes', '').replace('\\n', '\n').replace('\r\n', '\n').strip()
    sections = [part.split('\n', 1) for part in re.split(r'\n\s*\n', notes)]
    version = tag.removeprefix('v')
    current = re.compile(r'(?<![\w.])' + re.escape(version) + r'(?![\w.])')
    valid = (len(sections) == 3 and all(len(row) == 2 and row[0].strip().endswith('?')
             and row[1].strip() for row in sections)
             and all(current.search(row[0]) for row in sections[:2]))
    if not valid:
        if strict:
            raise ValueError('Release notes need three Q&A sections; changes and upgrade headings must name the exact release')
        return general + contribution
    return general + [[text(q.strip()), text(a.strip())] for q, a in sections] + contribution


def locales(document, tag):
    if document is None:
        return {}
    if document.get('schema') != 1 or document.get('release') != tag:
        raise ValueError('Website text manifest does not match the release')
    result = {}
    for language, fields in document['locales'].items():
        if language not in LANGUAGES or not isinstance(fields, dict):
            raise ValueError('Unknown content locale')
        clean = {}
        for key, value in fields.items():
            if key in ('description', 'releaseNotes'):
                clean[key] = text(value, 100000 if key == 'releaseNotes' else 2000)
            elif key in ('features', 'faq', 'trust', 'views'):
                count = {'features': 4, 'trust': 3, 'views': 5}.get(key)
                width = 3 if key == 'views' else 2
                if not isinstance(value, list) or (count and len(value) != count) or not 1 <= len(value) <= 20:
                    raise ValueError('Invalid content rows')
                clean[key] = []
                for row in value:
                    if not isinstance(row, list) or len(row) != width:
                        raise ValueError('Invalid content row')
                    clean[key].append([text(cell) for cell in row])
            else:
                raise ValueError('Unknown homepage field')
        if 'releaseNotes' in clean:
            clean['releaseNotes'] = clean['releaseNotes'].replace('\\n', '\n')
        if 'faq' in clean:
            clean['faq'] = release_faq(clean, tag)
        result[language] = clean
    if 'en' not in result:
        raise ValueError('English content fallback is required')
    return result


def png(data, expected):
    if digest(data) != expected or data[:8] != b'\x89PNG\r\n\x1a\n' or len(data) < 33:
        raise ValueError('Screenshot hash or PNG signature mismatch')
    width, height = struct.unpack('>II', data[16:24])
    if not 1 <= width <= 8192 or not 1 <= height <= 8192:
        raise ValueError('Screenshot dimensions exceed limit')


def atomic_json(path, data):
    temp = path.with_suffix('.tmp')
    temp.write_bytes(encoded(data) + b'\n')
    temp.replace(path)


def publish(root, content, images):
    """Snapshot URL is content-addressed; switch JSON and images together."""
    key = digest(encoded(content))
    snapshot = root / 'snapshots' / key
    current = root / 'current'
    if current.is_symlink() and os.readlink(current) == f'snapshots/{key}':
        return False
    if not snapshot.exists():
        with tempfile.TemporaryDirectory(dir=root, prefix='.staging-') as staging:
            stage = Path(staging)
            (stage / 'screenshots').mkdir()
            for name in FILES:
                (stage / 'screenshots' / name).write_bytes(images[name])
            payload = {**content, 'snapshot': key,
                       'screenshot_base': f'/live/snapshots/{key}/screenshots'}
            (stage / 'content.json').write_bytes(encoded(payload) + b'\n')
            for path in stage.rglob('*'):
                path.chmod(0o755 if path.is_dir() else 0o644)
            stage.chmod(0o755)
            stage.rename(snapshot)
    temporary = root / '.current-next'
    temporary.unlink(missing_ok=True)
    temporary.symlink_to(f'snapshots/{key}')
    temporary.replace(current)
    # Retain recent snapshots for open tabs and recovery, cap storage growth.
    snapshots = sorted((root / 'snapshots').iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
    for old in snapshots[10:]:
        if old != snapshot:
            shutil.rmtree(old)
    return True


def sync(root, seed):
    release = latest(get_json(f'{API}/releases?per_page=100'))
    tag = release['tag_name']
    commit = get_json(f'{API}/commits/{quote(tag, safe="")}')['sha']
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Invalid release commit')
    raw = f'https://raw.githubusercontent.com/{REPO}/{commit}/docs/'
    bundle = release_bundle(release, commit)
    manifest = None if bundle else get_json(raw + 'website-content.json', optional=True)
    if manifest is not None and manifest.get('release') != tag:
        manifest = None  # An inherited older manifest is not current release content.
    if manifest is None:
        baseline = json.loads((seed / 'website-content.json').read_text())
        manifest = baseline if baseline['release'] == tag else None
    translated = locales(manifest, tag)
    previous_path = root / 'current/content.json'
    previous = json.loads(previous_path.read_text()) if previous_path.exists() else None
    capture = None if bundle else get_json(raw + 'GUI-PREVIEWS.json', optional=True)
    images = {}
    pending = not capture or capture.get('published_release') != tag
    if not pending:
        hashes = capture.get('images', {})
        pending = not all(re.fullmatch(r'[0-9a-f]{64}', hashes.get('previews/' + name, '')) for name in FILES)
    if bundle:
        translated, images = bundle
        capture_tag, pending = tag, False
    elif pending:
        marker = previous or json.loads((seed / 'content-version.json').read_text())
        capture_tag = marker['screenshots_release']
        image_dir = root / 'current/screenshots' if previous else seed / 'screenshots'
        for name in FILES:
            images[name] = (image_dir / name).read_bytes()
            png(images[name], marker['screenshots'][name])
    else:
        capture_tag = tag
        for name in FILES:
            expected = hashes['previews/' + name]
            old = root / 'current/screenshots' / name
            if previous and previous['screenshots'].get(name) == expected and old.exists():
                data = old.read_bytes()
            else:
                data = fetch(raw + 'previews/' + name, 8_000_000)
            png(data, expected)
            images[name] = data
    content = {
        'schema': 1, 'release': tag, 'published_at': release['published_at'],
        'prerelease': bool(release.get('prerelease')), 'source_commit': commit,
        'release_url': f'https://github.com/{REPO}/releases/tag/{quote(tag, safe="")}',
        'release_notes': text(release.get('body') or '', 100000),
        'locales': translated, 'screenshots_release': capture_tag,
        'screenshots_pending': pending,
        'screenshots': {name: digest(images[name]) for name in FILES},
    }
    changed = publish(root, content, images)
    return {'release': tag, 'changed': changed, 'screenshots_pending': pending}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, default=Path('/var/lib/neutrondrive-content'))
    parser.add_argument('--seed', type=Path, default=Path('/usr/local/lib/neutrondrive-content/seed'))
    args = parser.parse_args()
    os.umask(0o022)
    args.root.mkdir(parents=True, exist_ok=True)
    (args.root / 'snapshots').mkdir(exist_ok=True)
    with (args.root / '.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        status = {'checked_at': datetime.now(timezone.utc).isoformat()}
        try:
            status.update(sync(args.root, args.seed), ok=True)
        except Exception as error:
            status.update(ok=False, error=str(error))
            atomic_json(args.root / 'status.json', status)
            raise
        atomic_json(args.root / 'status.json', status)
        print(json.dumps(status))

if __name__ == '__main__':
    main()
