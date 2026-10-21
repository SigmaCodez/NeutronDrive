"""Cache policy parity and stale-metadata protection at the Rust/host boundary."""
import random
import unittest
from unittest.mock import patch
from neutrondrive import rust_core
from neutrondrive.background import RemoteTreeCache


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

    def test_failure_discards_stale_listings_and_advances_generation(self):
        cache = RemoteTreeCache(object())
        cache._listings = {'root': ()}
        with patch.object(rust_core, 'call', side_effect=rust_core.CoreError('synthetic')):
            with self.assertRaises(rust_core.CoreError):
                cache.event('node', None, ['root'])
        self.assertEqual(cache._listings, {})
        self.assertEqual(cache._generation, 1)

    def test_in_flight_listing_is_not_reinserted_after_event(self):
        class Backend:
            def list_nodes(self, path):
                cache.event(None, None, ['root'])
                return []
        cache = RemoteTreeCache(Backend())
        self.assertEqual(cache.list_nodes_fresh('root'), [])
        self.assertNotIn('root', cache._listings)

    def test_invalid_snapshot_rejected(self):
        with self.assertRaises(rust_core.CoreError):
            rust_core.call('cache_invalidate', snapshot={'roots': ['root']})
