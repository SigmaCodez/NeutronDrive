import copy
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('verify_sync',Path(__file__).parents[1]/'scripts/verify-release-sync.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class ReleaseSyncTests(unittest.TestCase):
    def test_exact_release_texts_and_screenshots_required(self):
        bundle={'release':'v9.0','source_commit':'a'*40,'locales':{l:{'description':l} for l in ('en','de','fr','es','it')},'previews':{'images':{'example.png':{'sha256':'b'*64}}}}
        live={k:copy.deepcopy(bundle[k]) for k in ('release','source_commit','locales')}
        live.update(screenshots={'example.png':'b'*64},screenshots_release='v9.0',screenshots_pending=False)
        module.verify(bundle,live)
        for field,value in [('release','v8.0'),('source_commit','c'*40),('locales',{}),('screenshots',{}),('screenshots_release','v8.0'),('screenshots_pending',True)]:
            with self.subTest(field=field),self.assertRaises(ValueError):module.verify(bundle,{**live,field:value})
