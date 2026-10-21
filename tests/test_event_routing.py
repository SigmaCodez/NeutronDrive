"""Compare native job routing with the previous daemon policy, including queue bounds."""
import random
import unittest
from neutrondrive import rust_core


def previous(event, jobs, pending):
    result = dict(offline=event['kind']=='error', affected=[], reason='remote',
                  resolve_unknown=False, append=None, overflow=[])
    if result['offline']:
        return result
    identity, kind, reset = event['identity'], event['kind'], event['reset']
    result['reason'] = 'remote-reset' if reset else 'remote'
    known_all = set().union(*(set(j['known_uids']) | {j['remote_uid']} for j in jobs)) - {''}
    accounts = {j['account_root_uid'] for j in jobs} - {''}
    for job in jobs:
        known = set(job['known_uids']) | {job['remote_uid']}
        relevant = (reset or identity['node_uid'] in known or identity['parent_uid'] in known
                    or (identity['root_uid'] and identity['root_uid']==job['remote_uid']))
        if relevant and (kind!='ready' or reset):
            result['affected'].append(job['id'])
    unknown_parent = identity['parent_uid'] not in known_all | accounts
    unknown_node = identity['node_uid'] not in known_all
    result['resolve_unknown'] = bool(not reset and kind!='ready' and identity['node_uid'] and unknown_parent
        and (unknown_node or not str(kind).endswith('deleted')) and any(not j['paused'] for j in jobs))
    if result['resolve_unknown'] and identity not in pending:
        if len(pending)<1024:
            result['append'] = identity
        else:
            result['overflow'] = [j['id'] for j in jobs]
    return result


class EventRoutingTests(unittest.TestCase):
    def test_seeded_event_job_and_queue_decisions_match_previous_policy(self):
        rng=random.Random(6404)
        uids=['','file','folder','parent','account','outside','日本語']
        for index in range(1500):
            identity=lambda: {k:rng.choice(uids+[None]) for k in ('node_uid','parent_uid','root_uid')}
            event=dict(kind=rng.choice(['ready','error','node_deleted','node_updated','create','change','tree_remove']),
                       reset=rng.choice([False,False,True]),identity=identity())
            jobs=[dict(id=str(i),remote_uid=rng.choice(uids),account_root_uid=rng.choice(uids),
                       known_uids=rng.sample(uids,rng.randrange(len(uids)+1)),paused=rng.choice([False,True]))
                  for i in range(rng.randrange(6))]
            pending=[identity() for _ in range(rng.randrange(5))]
            if rng.randrange(3)==0:pending.append(event['identity'])
            with self.subTest(index=index):
                self.assertEqual(rust_core.call('route_event',event=event,jobs=jobs,pending=pending),previous(event,jobs,pending))

    def test_full_queue_overflow_and_duplicate_do_not_drop_existing_work(self):
        job=dict(id='job',remote_uid='folder',account_root_uid='account',known_uids=[],paused=False)
        event=dict(kind='node_created',reset=False,identity=dict(node_uid='new',parent_uid='unknown',root_uid='account'))
        pending=[dict(node_uid=str(i),parent_uid=None,root_uid='account') for i in range(1024)]
        self.assertEqual(rust_core.call('route_event',event=event,jobs=[job],pending=pending),previous(event,[job],pending))
        pending[-1]=event['identity']
        decision=rust_core.call('route_event',event=event,jobs=[job],pending=pending)
        self.assertIsNone(decision['append']);self.assertEqual(decision['overflow'],[])
        pending.append(dict(node_uid='too-many',parent_uid=None,root_uid=None))
        with self.assertRaises(rust_core.CoreError):rust_core.call('route_event',event=event,jobs=[job],pending=pending)

    def test_duplicate_jobs_and_invalid_identity_fail_closed(self):
        job=dict(id='job',remote_uid='folder',account_root_uid='account',known_uids=[],paused=False)
        event=dict(kind='change',reset=False,identity=dict(node_uid='new',parent_uid=None,root_uid=None))
        with self.assertRaises(rust_core.CoreError):rust_core.call('route_event',event=event,jobs=[job,job],pending=[])
        event['identity']['node_uid']=42
        with self.assertRaises(rust_core.CoreError):rust_core.call('route_event',event=event,jobs=[job],pending=[])
