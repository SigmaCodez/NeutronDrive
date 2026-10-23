"""Build-tool source identity and generator-artifact integrity."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('qt_tools', ROOT/'packaging/qt_tools.py')
tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tools)


class QtToolsTests(unittest.TestCase):
    def test_existing_wrong_source_archive_is_rejected_without_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'source.tar.xz'
            path.write_bytes(b'not the pinned Qt source')
            with patch.object(tools.urllib.request, 'urlopen') as network:
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    tools.source_archive(path)
                network.assert_not_called()

    def test_generator_tampering_and_wrong_source_identity_fail_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            qmake, moc = root/'qmake', root/'moc'
            qmake.write_bytes(b'qmake fixture'); moc.write_bytes(b'moc fixture')
            def inspect(path):
                return {'sha256': tools.sha(path), 'maximum_glibc': [2,34]}
            with patch.object(tools, 'inspect', side_effect=inspect), patch.object(tools.subprocess, 'check_output', return_value='6.10.2\n'):
                tools.record(qmake, moc, root/'artifact')
                tools.verify(root/'artifact')
                (root/'artifact/moc').write_bytes(b'changed')
                with self.assertRaisesRegex(ValueError, 'binary mismatch'):
                    tools.verify(root/'artifact')
                with patch.object(tools, 'source_lock', return_value=('6.10.2', {'sha256':'different'})):
                    with self.assertRaisesRegex(ValueError, 'provenance'):
                        tools.verify(root/'artifact')
