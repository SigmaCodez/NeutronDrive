"""Rebuilding must preserve the library inode mapped by a running application."""
import importlib.util
import mmap
from pathlib import Path
import tempfile
import unittest


class RustBuildPublishTests(unittest.TestCase):
    def test_running_mapping_keeps_old_library_after_rebuild(self):
        spec = importlib.util.spec_from_file_location('rust_build_publish', Path(__file__).resolve().parents[1] / 'packaging/rust_core.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'library.so'
            source = Path(directory) / 'new.so'
            target.write_bytes(b'old library' * 1024)
            source.write_bytes(b'new library')
            with target.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as mapping:
                module.publish_library(source, target)
                self.assertEqual(mapping[:], b'old library' * 1024)
                self.assertEqual(target.read_bytes(), b'new library')
            self.assertEqual(sorted(p.name for p in Path(directory).iterdir()), ['library.so', 'new.so'])
