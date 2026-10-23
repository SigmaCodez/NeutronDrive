"""Native boundary and differential migration tests; no account or network."""
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, replace
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import unittest

from core_contract import rust_core
from core_contract.sync import _Entry, SyncPlan, SyncError
import reference_policy_03 as old


def baseline(entry, remote, version=2):
    return dict(identity_version=version,kind=entry.kind,sha1=entry.sha1,size=entry.size,
                local_device=entry.device,local_inode=entry.inode,local_mtime_ns=entry.mtime_ns,
                local_ctime_ns=entry.ctime_ns,remote_uid=remote.uid,remote_revision=remote.revision,
                remote_path=remote.remote_path,subtree_complete=True)


class Oracle:
    def __init__(self, proofs): self.proofs=proofs

    def _check_ancestor_baselines(self, relative, local, remote, baseline):
        parts=relative.split('/')
        for index in range(1,len(parts)):
            parent='/'.join(parts[:index]);previous=baseline.get(parent,{})
            left,right=local.get(parent),remote.get(parent)
            if (left is None or right is None or left.kind!='folder' or right.kind!='folder'
                or not old._baseline_matches(left,previous,'local',ancestor=True)
                or not old._baseline_matches(right,previous,'remote')):
                raise SyncError('Deletion ancestor disappeared, moved, or was replaced')

    def _prove_remote_deletion(self, plan, remote, relative):
        if self.proofs.get(relative): raise SyncError(self.proofs[relative])


class RustCoreTests(unittest.TestCase):
    def test_native_core_identity_and_versioned_protocol(self):
        info=rust_core.call('info')
        self.assertEqual(info['implementation'],'rust')
        self.assertEqual(info['protocol'],1)
        self.assertIn('daemon-scheduler',info['components'])
        self.assertIn('sync-planner',info['components'])
        with self.assertRaises(rust_core.CoreError): rust_core.call('not_an_operation')
        with self.assertRaises(rust_core.CoreError): rust_core.call('select',jobs=[])

    def test_missing_library_never_silently_uses_python(self):
        env=dict(os.environ,NEUTRONDRIVE_CORE_LIBRARY='/missing/neutrondrive-core-test.so')
        result=subprocess.run([sys.executable,'-c','from core_contract.rust_core import call; call("info")'],env=env,capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('Rust-Kern fehlt',result.stderr)

    def test_planner_matches_released_policy_across_1200_seeded_snapshots(self):
        rng=random.Random(20261020)
        paths=('a.txt','b.txt','folder','folder/child.txt','folder/empty')
        for iteration in range(1200):
            local={};remote={};saved={};proofs={};pending={};blocked={}
            for index,path in enumerate(paths):
                kind='folder' if path in ('folder','folder/empty') else 'file'
                l=_Entry(kind,sha1='' if kind=='folder' else 'a'*40,size=0 if kind=='folder' else 10,
                         device=3,inode=index+1,mtime_ns=1792454400000000001,ctime_ns=1792454400000000011)
                r=_Entry(kind,sha1=l.sha1,size=l.size,uid=f'uid-{index}',revision='' if kind=='folder' else 'rev-1',remote_path=f'/my-files/uid-{index}')
                choice=rng.randrange(10)
                if rng.randrange(5): saved[path]=baseline(l,r,version=rng.choice((1,2,2,2)))
                if choice!=0: local[path]=l
                if choice!=1: remote[path]=r
                if choice==2: local[path]=replace(l,sha1='b'*40)
                if choice==3: remote[path]=replace(r,sha1='b'*40,revision='rev-2')
                if choice==4:
                    local[path]=replace(l,sha1='b'*40);remote[path]=replace(r,sha1='c'*40,revision='rev-2')
                if choice==5: local[path]=replace(l,inode=100+index,ctime_ns=l.ctime_ns+1)
                if choice==6: remote[path]=replace(r,uid=f'replaced-{index}')
                if choice==7: local[path]=replace(l,kind='folder' if kind=='file' else 'file')
                if choice==8: local.pop(path,None);remote.pop(path,None)
                if rng.randrange(10)==0: blocked[path]='unreadable fixture'
                if rng.randrange(8)==0: pending[path]=dict(direction=rng.choice(('local','remote')),operation_id='1'*32)
                proofs[path]=rng.choice((None,None,'Remote UID is still active elsewhere; a move is not deletion proof'))
            if iteration%9==0 and 'a.txt' in saved:
                if 'a.txt' in local: local['renamed.txt']=local.pop('a.txt')
                if iteration%18==0 and 'a.txt' in remote: remote['renamed.txt']=remote.pop('a.txt')
            plan=SyncPlan(Path('/tmp/oracle'),'/my-files','root',_blocked=set(blocked))
            expected=old.plan_policy(Oracle(proofs),plan,local,remote,saved,pending,blocked,None)
            actual=rust_core.call('plan',local=local,remote=remote,baseline=saved,pending=pending,blocked=blocked,proofs=proofs)
            with self.subTest(iteration=iteration):
                self.assertFalse(actual['checks'])
                self.assertEqual(actual['actions'],[asdict(a) for a in expected.actions])
                self.assertEqual(sorted(actual['conflicts'],key=lambda c:(c['relative_path'],c['reason'])),
                                 sorted([asdict(c) for c in expected.conflicts],key=lambda c:(c['relative_path'],c['reason'])))
                self.assertEqual(set(actual['blocked']),expected._blocked)

    def test_remote_delete_requires_explicit_observation(self):
        l=_Entry('file',sha1='a'*40,size=1,device=1,inode=1)
        r=replace(l,uid='uid',revision='rev')
        request=dict(local={'a':l},remote={},baseline={'a':baseline(l,r)},pending={},blocked={})
        value=rust_core.call('plan',**request,proofs={})
        self.assertEqual(value['checks'],['a']);self.assertEqual(value['actions'],[])
        self.assertEqual(rust_core.call('plan',**request,proofs={'a':None})['actions'][0]['kind'],'trash_local')
        self.assertFalse(rust_core.call('plan',**request,proofs={'a':'Uncertain trash'})['actions'])

    def test_scheduler_matches_previous_debounce_retry_and_paused_policy(self):
        rng=random.Random(17)
        for _ in range(600):
            state=dict(paused=rng.choice((False,False,True)),state=rng.choice(('idle','pending','syncing','offline','blocked')),
                       pending=rng.choice((True,False)),due=100.0,first_dirty=rng.choice((0.0,98.0)),
                       paths=set(rng.sample(['a','b',None],rng.randrange(4))),reasons={'startup'},rebuild_watch=False)
            reason=rng.choice(('local','watch-rebuild','watch-error','inotify-overflow','remote','retry'))
            immediate=rng.choice((True,False));now=103.0;debounce=1.5
            expected=dict(state,paths=set(state['paths']),reasons=set(state['reasons']))
            if not expected['paused'] and expected['state']!='blocked':
                if not expected['pending']: expected['first_dirty']=now
                expected['pending']=True;expected['reasons'].add(reason)
                if reason.startswith(('local','watch')) or reason=='inotify-overflow': expected['paths'].add('changed')
                if reason in {'watch-rebuild','watch-error','inotify-overflow'}: expected['rebuild_watch']=True
                if expected['state']!='offline' or immediate: expected['due']=now if immediate else min(now+debounce,(expected['first_dirty'] or now)+10)
            actual=rust_core.call('schedule',state=state,reason=reason,relative='changed',immediate=immediate,now=now,debounce=debounce)
            actual['paths']=set(actual['paths']);actual['reasons']=set(actual['reasons'])
            self.assertEqual(actual,expected)
        for failures in (0,1,5,10,50):
            for jitter in (0.9,1.0,1.1):
                actual=rust_core.call('retry',failures=failures,now=200.0,base=30.0,maximum=900.0,jitter=jitter)
                self.assertEqual(actual,{'failures':failures+1,'due':200+min(900,min(900,30*2**min(failures,10))*jitter)})

    def test_selection_keeps_first_tie_and_excludes_paused_blocked(self):
        jobs=[dict(id='paused',pending=True,paused=True,state='pending',due=0),
              dict(id='blocked',pending=True,paused=False,state='blocked',due=0),
              dict(id='first',pending=True,paused=False,state='pending',due=20),
              dict(id='second',pending=True,paused=False,state='pending',due=20)]
        self.assertEqual(rust_core.call('select',jobs=jobs,now=10),{'id':'first','delay':10})
        self.assertIsNone(rust_core.call('select',jobs=jobs[:2],now=10))

    def test_baseline_preserves_nanoseconds_and_unverified_absence(self):
        l=_Entry('file',sha1='a'*40,size=1,device=2,inode=2**60+3,mtime_ns=1792454400000000001,ctime_ns=1792454400000000007)
        r=replace(l,uid='u',revision='r')
        saved={'gone':baseline(l,r)}
        args=dict(local={'present':l},remote={'present':r},baseline=saved,blocked=set(),pending=set())
        value=rust_core.call('baseline',**args,absent=[])
        self.assertEqual(value['checks'],['gone']);self.assertIn('gone',value['baseline'])
        self.assertEqual(value['baseline']['present']['local_inode'],l.inode)
        self.assertEqual(value['baseline']['present']['local_ctime_ns'],l.ctime_ns)
        confirmed=rust_core.call('baseline',**args,absent=['gone'])
        self.assertNotIn('gone',confirmed['baseline'])

    def test_ffi_concurrent_owned_results_and_malformed_requests(self):
        def run(index):
            self.assertEqual(rust_core.call('retry',failures=index,now=0,base=1,maximum=20,jitter=1)['failures'],index+1)
        with ThreadPoolExecutor(max_workers=8) as pool: list(pool.map(run,range(300)))
        with self.assertRaises(rust_core.CoreError):
            rust_core.call('plan',local={'../unsafe':_Entry('file')},remote={},baseline={},pending={},blocked={},proofs={})
        with self.assertRaises(rust_core.CoreError):
            rust_core.call('plan',local={'a':{'kind':'link'}},remote={},baseline={},pending={},blocked={},proofs={})
        with self.assertRaises(ValueError): rust_core.call('select',jobs=[],now=float('nan'))


if __name__=='__main__': unittest.main()
