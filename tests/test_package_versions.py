"""Release metadata must agree so package managers can recognize upgrades."""
from pathlib import Path
import re
import unittest
from neutrondrive import __version__

ROOT=Path(__file__).resolve().parents[1]

class PackageVersionTests(unittest.TestCase):
    def test_native_and_flatpak_release_metadata_matches_application(self):
        deb=re.sub(r'a(\d+)$',r'~alpha\1',__version__)
        snap=re.sub(r'a(\d+)$',r'-alpha\1',__version__)
        checks={
            'scripts/build-deb':['Version: '+deb+'-1','neutrondrive_'+deb+'-1_'],
            'scripts/build-flatpak':['neutrondrive-'+__version__+'-'],
            'scripts/build-flatpak-bundle':['neutrondrive-'+__version__+'-'],
            'packaging/rpm/neutrondrive.spec':['Version:        '+deb],
            'packaging/snap/snapcraft.yaml':["version: '"+snap+"'"],
        }
        for filename,expected in checks.items():
            text=(ROOT/filename).read_text()
            for value in expected:
                with self.subTest(filename=filename,value=value):self.assertIn(value,text)
