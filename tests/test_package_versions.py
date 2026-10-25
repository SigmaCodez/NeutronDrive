"""Release metadata must agree so package managers can recognize upgrades."""
from pathlib import Path
import re
import unittest
ROOT=Path(__file__).resolve().parents[1]
RUST_VERSION=re.search(r'^version = "([^"]+)"', (ROOT/'core/Cargo.toml').read_text(), re.MULTILINE).group(1)
__version__=re.sub(r'-(alpha|beta)\.(\d+)$', lambda m: {'alpha':'a','beta':'b'}[m[1]]+m[2], RUST_VERSION)

class PackageVersionTests(unittest.TestCase):
    def test_native_and_flatpak_release_metadata_matches_application(self):
        desktop_version=re.search(r'^version = "([^"]+)"', (ROOT/'desktop/Cargo.toml').read_text(), re.MULTILINE).group(1)
        self.assertEqual(desktop_version, RUST_VERSION)
        deb=re.sub(r'-(alpha|beta)\.(\d+)$', r'~\1\2', RUST_VERSION)
        snap=re.sub(r'-(alpha|beta)\.(\d+)$', r'-\1\2', RUST_VERSION)
        checks={
            'scripts/build-deb':['Version: 1:'+deb+'-1','neutrondrive_'+deb+'-1_'],
            'scripts/build-flatpak':['neutrondrive-'+__version__+'-'],
            'scripts/build-flatpak-bundle':['neutrondrive-'+__version__+'-'],
            'packaging/rpm/neutrondrive.spec':['Version:        '+deb, 'Epoch:          1'],
            'packaging/snap/snapcraft.yaml':["version: '"+snap+"'"],
        }
        for filename,expected in checks.items():
            text=(ROOT/filename).read_text()
            for value in expected:
                with self.subTest(filename=filename,value=value):self.assertIn(value,text)
