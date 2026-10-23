"""Native watcher decoding: Linux byte names, safe batching and side effects."""
import os
from pathlib import Path
import random
import struct
import unittest
import tempfile
import time
import threading
from core_contract import rust_core
from reference_daemon import InotifyWatcher


def event(watch, mask, name=b''):
    return struct.pack('iIII',watch,mask,0,len(name))+name


class NativeInotifyTests(unittest.TestCase):
    def test_seeded_classification_matches_existing_watcher_policy(self):
        rng=random.Random(571)
        for _ in range(500):
            mask=rng.choice([8,4,64,128,256,512,1024,2048,0x4000,0x8000]) | rng.choice([0,0x40000000])
            name=rng.choice([b'',b'normal\0',b'non-utf8-\xff\0',b'.neutrondrive-tmp\0'])
            result=rust_core.call('inotify_decode',data=list(event(4,mask,name)))
            if mask & 0x4000: self.assertEqual(result,[{'kind':'overflow'}])
            elif mask & 0x8000: self.assertEqual(result,[{'kind':'ignored','watch':4}])
            elif name.startswith(b'.neutrondrive-'): self.assertEqual(result,[])
            else:
                self.assertEqual(result,[dict(kind='local',watch=4,name=list(name.split(b'\0')[0]),add_tree=bool(mask & 0x40000000 and mask & (256|128)),rebuild=bool(mask & (1024|2048)))])

    def wait_for(self, predicate):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if predicate(): return
            time.sleep(0.01)
        self.fail("Native watcher did not deliver the expected event")

    def test_linux_byte_names_and_new_directories_reach_adapter(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            events = []
            watcher = InotifyWatcher(root, lambda path, reason: events.append((path, reason)))
            watcher.start()
            try:
                (root / "directory").mkdir()
                self.wait_for(lambda: ("directory", "local") in events)
                name = os.fsencode(root / "directory") + b"/bad-\xff"
                with open(name, "wb") as target: target.write(b"data")
                self.wait_for(lambda: any(path is not None and os.fsencode(path) == b"directory/bad-\xff" for path, reason in events))
            finally:
                watcher.stop(); watcher.join()
            self.assertFalse(watcher._thread.is_alive())

    def test_malformed_batch_does_not_apply_partial_effects(self):
        # Native State::consume has a separate unit test proving zero callbacks
        # and unchanged registration state for this same malformed read.
        with self.assertRaises(rust_core.CoreError):
            rust_core.call("inotify_decode", data=list(event(4,0x40000100,b'directory\0')+b'incomplete'))

    def test_stop_from_callback_is_safe_and_handle_cannot_be_reused(self):
        with tempfile.TemporaryDirectory() as temporary:
            stopped = threading.Event()
            def callback(path, reason):
                watcher.stop()
                stopped.set()
            watcher = InotifyWatcher(Path(temporary), callback)
            watcher.start()
            (Path(temporary) / "file").write_bytes(b"x")
            self.assertTrue(stopped.wait(3))
            watcher.join()
            self.assertFalse(watcher._thread.is_alive())
            watcher.stop()  # idempotent
            with self.assertRaises(rust_core.BackendError):
                rust_core.backend_value("watcher_run", id=watcher._handle)

    def test_callback_failure_closes_native_descriptor(self):
        with tempfile.TemporaryDirectory() as temporary:
            failed = threading.Event()
            def callback(path, reason):
                if reason == "watch-error": failed.set()
                else: raise rust_core.BackendError("consumer failed")
            watcher = InotifyWatcher(Path(temporary), callback)
            watcher.start()
            (Path(temporary) / "file").write_bytes(b"x")
            self.assertTrue(failed.wait(3))
            watcher.join()
            self.assertFalse(watcher._thread.is_alive())
            with self.assertRaises(rust_core.BackendError):
                rust_core.backend_value("watcher_run", id=watcher._handle)
