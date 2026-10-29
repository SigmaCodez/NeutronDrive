#!/usr/bin/env python3
"""Real FUSE: unlink cached files/folders while a cloud trash request is stalled."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='nd-local-delete-') as tmp:
    root = Path(tmp)
    mount = root / 'mount'
    sdk = root / 'sdk'
    sdk.write_text('''#!/usr/bin/python3
import json,sys,time
from pathlib import Path
root=Path(__file__).parent
op=sys.argv[2];path=sys.argv[-1];uid=path.rsplit('/',1)[-1]
def node(uid):
 if uid in ('root','folder'):return {"uid":uid,"name":{"ok":True,"value":uid},"type":"folder"}
 return {"uid":uid,"name":{"ok":True,"value":uid},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":True}}}
def gone(uid):return (root/('gone-'+uid)).exists()
if op=='list':
 children=['one','two','folder'] if uid in ('root','my-files') else ['child'] if uid=='folder' else []
 value=[node(i) for i in children if not gone(i)]
elif op=='download':
 source=sys.argv[-2].rsplit('/',1)[-1]
 Path(path,source).write_bytes(b'abc');value={}
elif op=='trash':
 if uid=='one':(root/'waiting').touch();time.sleep(3)
 (root/('gone-'+uid)).touch();value={}
else:
 if gone(uid):sys.exit(1)
 value=node('root' if uid=='my-files' else uid)
print(json.dumps(value))
''')
    sdk.chmod(0o700)
    env = dict(os.environ, XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), XDG_CACHE_HOME=str(root/'cache'))
    env.pop('DISPLAY',None)
    env.pop('WAYLAND_DISPLAY',None)
    def until(predicate, seconds=15):
        deadline=time.monotonic()+seconds
        while not predicate():
            assert process.poll() is None, (root/'log').read_text()
            assert time.monotonic()<deadline, 'Synthetic mount test timed out'
            time.sleep(.03)
    with (root/'log').open('wb') as log:
        process=subprocess.Popen([str(binary),'--mount-foreground',str(mount),'--cli',str(sdk),'--read-write'],env=env,stdout=log,stderr=log)
        try:
            until(lambda:os.path.ismount(mount))
            for path in [mount/'one',mount/'two',mount/'folder'/'child']:
                assert path.read_bytes()==b'abc'
            started=time.monotonic()
            (mount/'one').unlink()
            assert time.monotonic()-started<1, 'Local deletion waited for cloud'
            until(lambda:(root/'waiting').exists())
            started=time.monotonic()
            (mount/'two').unlink()
            (mount/'folder'/'child').unlink()
            (mount/'folder').rmdir()
            elapsed=time.monotonic()-started
            assert elapsed<1, f'Deletion blocked by cloud: {elapsed:.3f}s'
            assert os.listdir(mount)==[], 'Deleted entries remain visible'
            until(lambda:all((root/('gone-'+i)).exists() for i in ['one','two','child','folder']))
            print(f'LOCAL_DELETE_DURING_CLOUD_STALL_OK {elapsed:.3f}s; FILE_AND_FOLDER_CLOUD_TRASH_OK')
        finally:
            process.terminate()
            process.wait(timeout=20)
            if os.path.ismount(mount):
                subprocess.run(['/usr/bin/fusermount3','-u',str(mount)],check=True)
