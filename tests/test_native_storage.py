"""Native state I/O regression tests. Temporary files only; no account state."""
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest

from neutrondrive.backend import BackendError
from neutrondrive.rust_core import backend_value


class NativeStorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / "state.json"

    def write(self, value):
        backend_value("store_write", path=str(self.path), value=value)

    def read(self):
        return backend_value("store_read_present", path=str(self.path), limit=1024 * 1024, private=True)

    def test_roundtrip_preserves_integer_identity_and_file_mode(self):
        value = {"inode": 2**63 + 5, "mtime_ns": 1792497600123456789, "name": "Grüße", "jobs": []}
        self.assertEqual(self.read(), {"present": False, "value": None})
        self.write(value)
        self.assertEqual(self.read(), {"present": True, "value": value})
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(list(self.root.glob(".state-*")), [])
        self.write(None)
        self.assertEqual(self.read(), {"present": True, "value": None})

    def test_symlink_ancestors_are_rejected_for_reads_writes_and_creation(self):
        real = self.root / "real"
        real.mkdir(mode=0o700)
        link = self.root / "link"
        link.symlink_to(real, target_is_directory=True)
        (real / "state.json").write_text("keep me")
        self.path = link / "state.json"
        with self.assertRaises(BackendError): self.write({"changed": True})
        with self.assertRaises(BackendError): self.read()
        with self.assertRaises(BackendError): backend_value("private_directory", path=str(link / "child"))
        self.assertEqual((real / "state.json").read_text(), "keep me")
        self.assertFalse((real / "child").exists())

    def test_fifo_read_does_not_block(self):
        os.mkfifo(self.path, 0o600)
        with self.assertRaises(BackendError): self.read()

    def test_corrupt_and_oversized_files_are_not_replaced_or_defaulted(self):
        for contents in (b"{broken", b'"' + b"x" * (1024 * 1024) + b'"'):
            self.path.write_bytes(contents)
            self.path.chmod(0o600)
            with self.assertRaises(BackendError): self.read()
            self.assertEqual(self.path.read_bytes(), contents)

    def test_group_readable_file_and_directory_are_rejected(self):
        self.write({"private": True})
        self.path.chmod(0o640)
        with self.assertRaises(BackendError): self.read()
        self.root.chmod(0o750)
        self.addCleanup(self.root.chmod, 0o700)
        with self.assertRaises(BackendError): self.write({"changed": True})
        self.assertEqual(json.loads(self.path.read_text()), {"private": True})

    def test_replacement_does_not_follow_destination_symlink(self):
        target = self.root / "unrelated"
        target.write_text("preserve")
        self.path.symlink_to(target)
        self.write({"new": True})
        self.assertFalse(self.path.is_symlink())
        self.assertEqual(target.read_text(), "preserve")

    def test_concurrent_readers_see_only_complete_replacements(self):
        self.write({"sequence": 0, "payload": "0" * 8192})
        done = threading.Event()
        failures = []
        def reader():
            try:
                while not done.is_set():
                    value = self.read()["value"]
                    self.assertEqual(value["payload"], str(value["sequence"]) * 8192)
            except BaseException as error: failures.append(error)
        thread = threading.Thread(target=reader)
        thread.start()
        try:
            for number in range(1, 40): self.write({"sequence": number, "payload": str(number) * 8192})
        finally:
            done.set()
            thread.join(3)
        self.assertFalse(thread.is_alive())
        self.assertEqual(failures, [])
        self.assertEqual(list(self.root.glob(".state-*")), [])

    def test_failed_replace_keeps_existing_destination_and_cleans_temporary(self):
        self.path.mkdir(mode=0o700)
        child = self.path / "unchanged"
        child.write_text("keep")
        with self.assertRaises(BackendError): self.write({"new": True})
        self.assertEqual(child.read_text(), "keep")
        self.assertEqual(list(self.root.glob(".state-*")), [])


if __name__ == "__main__": unittest.main()
