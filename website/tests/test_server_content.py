import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('sync', Path(__file__).parents[1] / 'scripts/sync-server-content.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
SITE = Path(__file__).parents[1]

class ServerContentTests(unittest.TestCase):
    def test_invalid_manifest_and_image_are_rejected(self):
        with self.assertRaises(ValueError):
            sync.locales({'schema': 1, 'release': 'old', 'locales': {'en': {}}}, 'new')
        with self.assertRaises(ValueError):
            sync.locales({'schema': 1, 'release': 'new', 'locales': {'en': {'legal': 'changed'}}}, 'new')
        with self.assertRaises(ValueError):
            sync.png(b'not an image', sync.digest(b'not an image'))

    def test_new_release_missing_captures_preserves_labelled_images(self):
        def source(url, optional=False):
            if '/releases?' in url:
                return [{'tag_name': 'v9.0', 'published_at': '2027-01-01T00:00:00Z', 'body': 'New release', 'prerelease': False}]
            if '/commits/' in url:
                return {'sha': 'a' * 40}
            if url.endswith('website-content.json'):
                return {'schema': 1, 'release': 'old', 'locales': {'en': {}}}
            return None
        with tempfile.TemporaryDirectory() as directory, patch.object(sync, 'get_json', side_effect=source):
            root = Path(directory); (root/'snapshots').mkdir()
            result = sync.sync(root, SITE/'public')
            self.assertTrue(result['changed'])
            payload = json.loads((root/'current/content.json').read_text())
            self.assertEqual(payload['release'], 'v9.0')
            self.assertTrue(payload['screenshots_pending'])
            self.assertEqual(payload['screenshots_release'], 'v0.1.0-beta1')
            self.assertEqual(payload['locales'], {})
            self.assertFalse(sync.sync(root, SITE/'public')['changed'])
            previous = (root/'current').readlink()
            with patch.object(sync, 'get_json', side_effect=RuntimeError('network unavailable')):
                with self.assertRaises(RuntimeError): sync.sync(root, SITE/'public')
            self.assertEqual((root/'current').readlink(), previous)

    def test_verified_new_capture_switches_snapshot(self):
        images = {name:(SITE/'public/screenshots'/name).read_bytes() for name in sync.FILES}
        def source(url, optional=False):
            if '/releases?' in url:
                return [{'tag_name':'v9.0', 'published_at':'2027-01-01T00:00:00Z','body':'New notes'}]
            if '/commits/' in url: return {'sha':'a'*40}
            if url.endswith('website-content.json'):
                return {'schema':1,'release':'v9.0','locales':{'en':{'description':'New description'},'de':{'description':'Neue Beschreibung'}}}
            return {'published_release':'v9.0','images':{'previews/'+name:sync.digest(data) for name,data in images.items()}}
        with tempfile.TemporaryDirectory() as directory, patch.object(sync,'get_json',side_effect=source), patch.object(sync,'fetch',side_effect=lambda url,limit:images[url.rsplit('/',1)[1]]):
            root=Path(directory);(root/'snapshots').mkdir()
            sync.sync(root,SITE/'public')
            data=json.loads((root/'current/content.json').read_text())
            self.assertFalse(data['screenshots_pending'])
            self.assertEqual(data['screenshots_release'],'v9.0')
            self.assertEqual(data['locales']['de']['description'],'Neue Beschreibung')
            before=(root/'current').readlink()
            # A corrupt download cannot switch the live snapshot.
            with patch.object(sync,'fetch',return_value=b'bad'):
                (root/'current/screenshots'/sync.FILES[0]).unlink()
                with self.assertRaises(ValueError):sync.sync(root,SITE/'public')
            self.assertEqual((root/'current').readlink(),before)
