"""Release ordering, source restrictions and corrupt-download rejection."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from core_contract import updates


def release(tag='v0.6.0-alpha7', **values):
    name=f'neutrondrive_{tag[1:]}-1_amd64.deb'
    result=dict(tag_name=tag, html_url=updates.REPOSITORY+'/releases/tag/'+tag,
                published_at='2026-10-23', prerelease=True, draft=False,
                assets=[dict(name=name, browser_download_url=updates.REPOSITORY+'/releases/download/'+tag+'/'+name,
                             digest='sha256:'+hashlib.sha256(b'package').hexdigest(), size=7)])
    result.update(values)
    return result


class UpdateTests(unittest.TestCase):
    def choose(self, data, preview=True, current='0.6.0a6', format='deb', machine='x86_64'):
        return updates.choose_release(data,current,preview,format,machine)

    def test_numeric_prerelease_order_and_stable_priority(self):
        self.assertGreater(updates.version_key('v0.6.0-alpha10'),updates.version_key('0.6.0a9'))
        self.assertGreater(updates.version_key('v0.6.0'),updates.version_key('0.6.0-rc99'))
        self.assertEqual(self.choose([release('v0.6.0-alpha10'),release()]).tag,'v0.6.0-alpha10')
        self.assertIsNone(self.choose([release('v0.6.0-alpha6')]))
        self.assertIsNone(self.choose([release()],preview=False))
        self.assertIsNone(self.choose([release(prerelease=False)],preview=False))
        self.assertIsNotNone(self.choose([release('v0.6.0',prerelease=False)],preview=False))

    def test_no_drafts_unpublished_invalid_tags_or_foreign_urls(self):
        self.assertIsNone(self.choose([release(draft=True),release(published_at=None),
            release(html_url='https://example.org'),release(tag_name='../../bad')]))
        with self.assertRaises(ValueError): self.choose({})

    def test_architecture_format_and_digest_fail_closed(self):
        self.assertTrue(self.choose([release()]).asset_url)
        for kwargs in ({'format':'flatpak'},{'format':'snap'},{'machine':'aarch64'},{'machine':'unknown'}):
            self.assertFalse(self.choose([release()],**kwargs).asset_url)
        for key,value in [('digest',None),('digest','sha256:bad'),('browser_download_url','https://example.org/a.deb'),('size',True),('size',updates.MAX_PACKAGE+1)]:
            item=release();item['assets'][0][key]=value
            self.assertFalse(self.choose([item]).asset_url)

    def test_native_reverify_detects_modified_payload(self):
        item=self.choose([release()])
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'neutrondrive.deb'
            path.write_bytes(b'package')
            updates.verify_download(path,item)
            path.write_bytes(b'changed')
            with self.assertRaises(ValueError): updates.verify_download(path,item)

    def test_native_reverify_rejects_hardlink(self):
        import os
        item=self.choose([release()])
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'neutrondrive.deb'
            path.write_bytes(b'package')
            os.link(path,Path(folder)/'alias')
            with self.assertRaises(ValueError): updates.verify_download(path,item)

    def test_sandbox_overrides_native_package_detection(self):
        with patch.dict('os.environ',{'FLATPAK_ID':'app'}):self.assertEqual(updates.install_format(),'flatpak')
        with patch.dict('os.environ',{'SNAP':'/snap/app'},clear=True):self.assertEqual(updates.install_format(),'snap')

    def test_cancellation_removes_partial_file(self):
        import threading
        cancelled=threading.Event(); cancelled.set()
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(InterruptedError): updates.download(self.choose([release()]),folder,cancelled)
            self.assertEqual(list(Path(folder).iterdir()),[])
