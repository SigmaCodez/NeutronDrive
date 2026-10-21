"""Native watcher decoding: Linux byte names, safe batching and side effects."""
import os
from pathlib import Path
import random
import struct
import unittest
from unittest.mock import Mock
from neutrondrive import rust_core
from neutrondrive.background import InotifyWatcher


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

    def watcher(self):
        watcher=object.__new__(InotifyWatcher)
        watcher.root=Path('/synthetic')
        watcher._watches={4:watcher.root}
        watcher.callback=Mock()
        watcher._add_tree=Mock()
        return watcher

    def test_linux_byte_names_and_new_directories_reach_adapter(self):
        w=self.watcher()
        w._consume(event(4,8,b'bad-\xff\0')+event(4,0x40000100,b'directory\0'))
        self.assertEqual(os.fsencode(w.callback.call_args_list[0].args[0]),b'bad-\xff')
        w._add_tree.assert_called_once_with(Path('/synthetic/directory'))
        self.assertEqual(w.callback.call_args_list[1].args,('directory','local'))

    def test_malformed_batch_does_not_apply_partial_effects(self):
        w=self.watcher()
        with self.assertRaises(rust_core.CoreError): w._consume(event(4,0x40000100,b'directory\0')+b'incomplete')
        w._add_tree.assert_not_called();w.callback.assert_not_called()

    def test_overflow_and_watch_removal(self):
        w=self.watcher()
        w._consume(event(-1,0x4000)+event(4,0x8000)+event(4,8,b'later\0'))
        w.callback.assert_called_once_with(None,'inotify-overflow')
        self.assertEqual(w._watches,{})
