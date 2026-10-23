"""Native computer setup against a synthetic SDK executable; no real account."""
from dataclasses import replace
from pathlib import Path
import json
import os
import sys
import tempfile
import unittest
from unittest.mock import patch
from core_contract.backend import Backend, Node, BackendError
from core_contract.computer_backup import add_folder
from core_contract.background import JobStore

SDK_PROGRAM = r'''
import json, sys
from pathlib import Path
model=json.loads(MODEL.read_text())
args=sys.argv[1:]
def folder(uid,name,parent=None):
    value=dict(uid=uid,name=dict(ok=True,value=name),type='folder')
    if parent:value['parentUid']=parent
    return value
if args[:2]==['computers','create']:
    model['calls'].append(args)
    model['device']=dict(uid='device',rootFolderUid='root',name=args[-1])
    if model.get('replace_root'):
        root=Path(model['replace_root']);root.rename(root.with_name('old-folder'));root.mkdir()
    result=model['device']
elif args[:2]==['computers','info']:
    model['calls'].append(args);result=model['device']
elif args[:2]==['filesystem','info']:
    path=args[-1]
    if path=='/my-files':result=folder(model['account'],'My files')
    else:result=folder(model['device']['rootFolderUid'],model['device']['name'])
elif args[:2]==['filesystem','list']:
    result=model['children']
elif args[:2]==['filesystem','create-folder']:
    result=folder('child'+str(len(model['children'])),args[-1],'root');model['children'].append(result)
else:
    raise RuntimeError('Unexpected test SDK command')
MODEL.write_text(json.dumps(model))
print(json.dumps(result))
'''

class ComputerCloud(Backend):
    def __init__(self, base):
        self.model_path=base/'sdk-model.json'
        self.model_path.write_text(json.dumps(dict(account='account',device=None,children=[],calls=[])))
        script=base/'synthetic-sdk'
        script.write_text(f'#!{sys.executable}\nfrom pathlib import Path\nMODEL=Path({str(self.model_path)!r})\n'+SDK_PROGRAM)
        script.chmod(0o700)
        super().__init__(str(script))
    @property
    def model(self):return json.loads(self.model_path.read_text())
    def change(self, **fields):self.model_path.write_text(json.dumps(dict(self.model, **fields)))
    @property
    def calls(self):return self.model['calls']
    @property
    def children(self):return self.model['children']

class ComputerBackupTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name)
        self.state=self.base/'state';self.state.mkdir(mode=0o700)
        self.local=self.base/'Documents';self.local.mkdir()
        self.cloud=ComputerCloud(self.base)
        env=patch.dict(os.environ,{'XDG_STATE_HOME':str(self.base/'process-state')});env.start();self.addCleanup(env.stop)
    def test_register_and_reuse_computer_with_pinned_job(self):
        job=add_folder(self.cloud,self.state,self.local)
        self.assertFalse(job.paused)
        self.assertEqual(job.remote_uid,'child0')
        self.assertEqual(job.account_root_uid,'account')
        self.assertTrue(job.remote_root.startswith('/devices/'))
        other=self.base/'Pictures';other.mkdir()
        second=add_folder(self.cloud,self.state,other)
        self.assertNotEqual(job.id,second.id)
        self.assertEqual([call[1] for call in self.cloud.calls],['create','info'])
        self.assertEqual((self.state/'computer.json').stat().st_mode & 0o777,0o600)
    def test_account_change_fails_before_cloud_mutation(self):
        add_folder(self.cloud,self.state,self.local)
        self.cloud.change(account='another-account')
        other=self.base/'Pictures';other.mkdir()
        with self.assertRaises(BackendError):add_folder(self.cloud,self.state,other)
        self.assertEqual(len(self.cloud.calls),1)
        self.assertEqual(len(self.cloud.children),1)
    def test_duplicate_and_nested_local_folder_rejected(self):
        add_folder(self.cloud,self.state,self.local)
        child=self.local/'nested';child.mkdir()
        for path in (self.local,child):
            with self.assertRaises(BackendError):add_folder(self.cloud,self.state,path)
        self.assertEqual(len(self.cloud.calls),1)
    def test_existing_cloud_name_is_not_merged(self):
        add_folder(self.cloud,self.state,self.local)
        store=JobStore(self.state/'background');store.remove(store.list()[0].id)
        with self.assertRaises(BackendError):add_folder(self.cloud,self.state,self.local)
        self.assertEqual(len(self.cloud.children),1)
    def test_replaced_device_root_blocks_setup(self):
        add_folder(self.cloud,self.state,self.local)
        self.cloud.change(device=dict(self.cloud.model['device'],rootFolderUid='replacement'))
        other=self.base/'Pictures';other.mkdir()
        with self.assertRaises(BackendError):add_folder(self.cloud,self.state,other)
        self.assertEqual(len(self.cloud.children),1)
    def test_interruption_before_pin_leaves_job_paused(self):
        path=self.state/'background/jobs.json'
        class StopAfterJob:
            def is_set(self):
                return path.exists() and bool(json.loads(path.read_text())['jobs'])
        self.cloud.cancel_event=StopAfterJob()
        with self.assertRaises(BackendError):add_folder(self.cloud,self.state,self.local)
        jobs=JobStore(self.state/'background').list()
        self.assertEqual(len(jobs),1);self.assertTrue(jobs[0].paused)
        self.assertEqual(jobs[0].remote_uid,'')
    def test_replaced_local_folder_during_registration_is_rejected(self):
        self.cloud.change(replace_root=str(self.local))
        with self.assertRaises(BackendError):add_folder(self.cloud,self.state,self.local)
        self.assertEqual(self.cloud.children,[])
        self.assertEqual(JobStore(self.state/'background').list(),[])


class DeviceSyncTests(unittest.TestCase):
    def test_device_folder_upload_download_conflict_and_trash(self):
        from test_sync import FakeBackend
        from core_contract.sync import SyncEngine
        class DeviceCloud(FakeBackend):
            def __init__(self):
                super().__init__()
                self.nodes['']=replace(self.nodes[''],path='/devices/Linux')
            def add(self, relative, data=None, **overrides):
                node=super().add(relative,data,**overrides)
                node=replace(node,path='/devices/Linux/'+node.uid)
                self.nodes[relative]=node
                return node
        with tempfile.TemporaryDirectory() as folder:
            base=Path(folder);local=base/'local';local.mkdir()
            backend=DeviceCloud();engine=SyncEngine(backend,base/'state')
            (local/'a.txt').write_bytes(b'local')
            backend.add('b.txt',b'remote')
            result=engine.execute(engine.plan(local,'/devices/Linux'))
            self.assertTrue(result.ok,result)
            self.assertEqual(backend.contents['a.txt'],b'local')
            self.assertEqual((local/'b.txt').read_bytes(),b'remote')
            (local/'a.txt').write_bytes(b'changed local')
            backend.add('a.txt',b'changed remote')
            plan=engine.plan(local,'/devices/Linux')
            self.assertTrue(plan.conflicts)
            self.assertEqual((local/'a.txt').read_bytes(),b'changed local')
            (local/'b.txt').unlink()
            result=engine.execute(engine.plan(local,'/devices/Linux'))
            self.assertIn('b.txt',backend.trashed_nodes)
