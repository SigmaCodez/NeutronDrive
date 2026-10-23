"""Cache policy parity and stale-metadata protection at the Rust/host boundary."""
import random
import unittest
from core_contract import rust_core
from core_contract.background import RemoteTreeCache
from core_contract.backend import BackendError


def previous_policy(snapshot):
    aliases = snapshot['aliases']
    result = set()
    def tree(path):
        pending, seen = [aliases.get(path, path)], set()
        while pending:
            current = pending.pop()
            if current in seen:
                continue
            seen.add(current)
            pending.extend(snapshot['children'].get(current, ()))
        result.update(seen)
    if snapshot['reset']:
        for root in snapshot['roots']:
            tree(root)
        return sorted(result)
    node, parent = snapshot['node_uid'], snapshot['parent_uid']
    affected = set(snapshot['parents'].get(node, ()))
    paths = snapshot['uid_paths']
    if parent in paths:
        affected.add(paths[parent])
    if node in paths:
        tree(paths[node])
        affected.add(paths[node])
    if affected:
        result.update(aliases.get(path, path) for path in affected)
    else:
        for root in snapshot['roots']:
            tree(root)
    return sorted(result)


class CachePolicyTests(unittest.TestCase):
    def test_seeded_graphs_match_previous_policy(self):
        rng = random.Random(6202)
        paths = ['root', 'alias', 'a', 'a/b', 'a-other', '日本語', 'x', 'y']
        uids = ['one', 'two', 'three']
        for case in range(500):
            sample = lambda values: rng.sample(values, rng.randrange(len(values)+1))
            snapshot = dict(roots=sample(paths), children={p: sample(paths) for p in paths},
                            aliases={p: rng.choice(paths) for p in sample(paths)},
                            parents={uid: sample(paths) for uid in sample(uids)},
                            uid_paths={uid: rng.choice(paths) for uid in sample(uids)},
                            node_uid=rng.choice(uids+[None,'unknown']),
                            parent_uid=rng.choice(uids+[None,'unknown']), reset=rng.choice([False,True]))
            with self.subTest(case=case):
                self.assertEqual(rust_core.call('cache_invalidate', snapshot=snapshot), previous_policy(snapshot))

    def test_failed_write_discards_stale_listing(self):
        class Backend:
            calls = 0
            def list_nodes(self, path):
                self.calls += 1
                return []
            def mkdir(self, parent, name):
                raise BackendError('synthetic lost acknowledgement')
        backend = Backend()
        cache = RemoteTreeCache(backend)
        self.assertEqual(cache.list_nodes('root'), [])
        self.assertEqual(cache.list_nodes('root'), [])
        self.assertEqual(backend.calls, 1)
        with self.assertRaises(BackendError):
            cache.mkdir('root', 'new')
        self.assertEqual(cache.list_nodes('root'), [])
        self.assertEqual(backend.calls, 2)

    def test_in_flight_listing_is_not_reinserted_after_event(self):
        class Backend:
            calls = 0
            def list_nodes(self, path):
                self.calls += 1
                if self.calls == 1:
                    cache.event(None, None, ['root'])
                return []
        backend = Backend()
        cache = RemoteTreeCache(backend)
        self.assertEqual(cache.list_nodes_fresh('root'), [])
        self.assertEqual(cache.list_nodes('root'), [])
        self.assertEqual(backend.calls, 2)
        self.assertEqual(cache.list_nodes('root'), [])
        self.assertEqual(backend.calls, 2)

    def test_invalid_snapshot_rejected(self):
        with self.assertRaises(rust_core.CoreError):
            rust_core.call('cache_invalidate', snapshot={'roots': ['root']})
