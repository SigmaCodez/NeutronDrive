"""Store metadata and beta publication guardrails; no network or account needed."""
import copy
import importlib.machinery
import importlib.util
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
loader = importlib.machinery.SourceFileLoader('publish_snap_beta', str(ROOT/'scripts/publish-snap-beta'))
spec = importlib.util.spec_from_loader(loader.name, loader)
publisher = importlib.util.module_from_spec(spec)
loader.exec_module(publisher)

class StorePreparationTests(unittest.TestCase):
    def test_beta_upload_rejects_wrong_application_or_confinement(self):
        good = dict(name='neutrondrive', confinement='strict', grade='devel',
                    version='0.1.0-beta1', base='core24',
                    summary='Unofficial Proton Drive client for Linux')
        publisher.validate(good)
        for key, value in [('name','other'), ('confinement','classic'),
                           ('grade','stable'), ('base','core22'),
                           ('version','0.1.0'), ('version',1), ('summary','Official client')]:
            bad=copy.deepcopy(good);bad[key]=value
            with self.subTest(field=key), self.assertRaises(ValueError):
                publisher.validate(bad)

    def test_upload_is_beta_only_and_check_only_never_uploads(self):
        import tempfile
        from unittest.mock import patch
        try:
            import yaml
        except ImportError:
            self.skipTest('PyYAML build dependency unavailable')
        metadata = "name: neutrondrive\nconfinement: strict\ngrade: devel\nbase: core24\nversion: 0.1.0-beta1\nsummary: Unofficial Proton Drive client for Linux\n"
        with tempfile.TemporaryDirectory() as temporary:
            package=Path(temporary)/'package with spaces.snap';package.touch()
            for check in [True, False]:
                argv=['publish-snap-beta',str(package)]+(['--check-only'] if check else [])
                with patch('sys.argv',argv), patch.object(publisher.subprocess,'check_output',return_value=metadata), patch.object(publisher.subprocess,'run') as upload:
                    self.assertEqual(publisher.main(),0)
                    if check:
                        upload.assert_not_called()
                    else:
                        upload.assert_called_once_with(['snapcraft','upload','--release=beta',str(package)],check=True)
            with patch('sys.argv',['publish-snap-beta',str(package)]), patch.object(publisher.subprocess,'check_output',return_value='invalid: [yaml'), patch.object(publisher.subprocess,'run') as upload:
                self.assertEqual(publisher.main(),1)
                upload.assert_not_called()

    def test_metadata_screenshots_exist_in_source(self):
        metadata=ET.parse(ROOT/'data/io.github.SigmaCodez.NeutronDrive.metainfo.xml').getroot()
        self.assertEqual(metadata.findtext('summary'),'Unofficial Proton Drive client for Linux')
        self.assertEqual(metadata.find("url[@type='homepage']").text,'https://neutrondrive.sigmacodez.dev')
        for image in metadata.findall('screenshots/screenshot/image'):
            self.assertTrue((ROOT / image.text.split('/v0.1.0-beta1/',1)[1]).is_file())

    def test_flatpak_has_no_native_host_service_permission(self):
        import json
        manifest=json.loads((ROOT/'packaging/flatpak/io.github.SigmaCodez.NeutronDrive.json').read_text())
        permissions=manifest['finish-args']
        self.assertNotIn('--talk-name=io.github.SigmaCodez.NeutronDrive.Service',permissions)
        self.assertFalse(any(p in permissions for p in ['--socket=session-bus','--talk-name=org.freedesktop.Flatpak','--device=all']))
        self.assertIn('--no-host-services',manifest['modules'][0]['build-commands'][0])

if __name__ == '__main__':
    unittest.main()
