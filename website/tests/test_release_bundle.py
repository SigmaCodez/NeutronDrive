import base64
import copy
import importlib.util
import json
from pathlib import Path
import unittest
import tempfile

SITE=Path(__file__).resolve().parents[1]
ROOT=SITE.parent
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
sync=load('sync_bundle',SITE/'scripts/sync-server-content.py')
publisher=load('publisher',ROOT/'scripts/build-website-release.py') if (ROOT/'scripts/build-website-release.py').exists() else None

class BundleTests(unittest.TestCase):
    def bundle(self):
        d=json.loads((SITE/'public/website-content.json').read_text())
        d['source_commit']='a'*40
        for fields in d['locales'].values(): fields['releaseNotes']=fields['description']
        d['previews']={'published_release':d['release'],'language':'en','binary_version':'NeutronDrive 0.1.0-beta.1','images':{}}
        for name in sync.FILES:
            data=(SITE/'public/screenshots'/name).read_bytes()
            d['previews']['images'][name]={'base64':base64.b64encode(data).decode(),'sha256':sync.digest(data)}
        return d
    def test_complete_bundle(self):
        d=self.bundle(); texts,images=sync.validate_bundle(d,d['release'],'a'*40)
        self.assertEqual(len(texts),6);self.assertEqual(len(images),5)
    def test_legacy_five_language_bundle_remains_readable(self):
        d=self.bundle();del d['locales']['pt']
        texts,_=sync.validate_bundle(d,d['release'],'a'*40)
        self.assertEqual(set(texts),sync.LEGACY_LANGUAGES)

    def test_old_or_missing_language_rejected(self):
        d=self.bundle();del d['locales']['it']
        with self.assertRaises(ValueError):sync.validate_bundle(d,d['release'],'a'*40)
        d=self.bundle()
        with self.assertRaises(ValueError):sync.validate_bundle(d,'v9.0','a'*40)
        with self.assertRaises(ValueError):sync.validate_bundle(d,d['release'],'b'*40)
    def test_corrupt_png_and_wrong_binary_rejected(self):
        d=self.bundle();d['previews']['images']['photos-light.png']['sha256']='0'*64
        with self.assertRaises(ValueError):sync.validate_bundle(d,d['release'],'a'*40)
        d=self.bundle();d['previews']['binary_version']='NeutronDrive 0.6-alpha'
        with self.assertRaises(ValueError):sync.validate_bundle(d,d['release'],'a'*40)
    def test_publisher_requires_reviewed_tag(self):
        if publisher is None:self.skipTest('Publisher lives outside Docker website context')
        with self.assertRaises(ValueError):publisher.reviewed_text(SITE/'public/website-content.json','v9.0')

    def test_new_publisher_requires_portuguese(self):
        if publisher is None:self.skipTest('Publisher lives outside Docker website context')
        d=json.loads((ROOT/'docs/website-content.json').read_text())
        del d['locales']['pt']
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'content.json';path.write_text(json.dumps(d))
            with self.assertRaisesRegex(ValueError,'requires.*pt'):
                publisher.reviewed_text(path,d['release'])
