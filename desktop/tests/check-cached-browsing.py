#!/usr/bin/env python3
"""Real FUSE regression: preload nested metadata; cached browsing during a stalled SDK."""
import hashlib,json,os,pathlib,subprocess,sys,tempfile,time
binary=pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='nd-cache-browse-') as tmp:
 r=pathlib.Path(tmp);mount=r/'mount';sdk=r/'sdk';state=r/'state';config=r/'config'
 sdk.write_text('''#!/usr/bin/python3
import sys,json,pathlib,time
root=pathlib.Path(__file__).parent
op=sys.argv[2];path=sys.argv[-1]
def folder(uid,name):return {"uid":uid,"name":{"ok":True,"value":name},"type":"folder"}
def file(uid,name):return {"uid":uid,"name":{"ok":True,"value":name},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":True}}}
if op=='list' and path.endswith('/sub'):
 (root/'waiting').touch();time.sleep(5)
if op=='list':
 value=[folder('sub','sub'),file('data','data')] if path.endswith('/root') else [folder('deep','deep')] if path.endswith('/sub') else [file('leaf','leaf')]
elif op=='download':
 (root/'downloaded').touch();pathlib.Path(path,'data').write_bytes(b'abc');value={}
else:value=folder('root','root') if path=='/my-files' else file('data','data')
print(json.dumps(value))
''');sdk.chmod(0o700)
 env=dict(os.environ,XDG_STATE_HOME=str(state),XDG_CONFIG_HOME=str(config));env.pop('DISPLAY',None);env.pop('WAYLAND_DISPLAY',None)
 with (r/'log').open('wb') as log:
  p=subprocess.Popen([str(binary),'--mount-foreground',str(mount),'--cli',str(sdk)],env=env,stdout=log,stderr=log)
  stalled=None
  try:
   deadline=time.monotonic()+15
   while not os.path.ismount(mount):
    assert p.poll() is None,(r/'log').read_text()
    assert time.monotonic()<deadline,'Mount timeout'
    time.sleep(.05)
   snapshots=list(state.rglob('metadata.json'));assert len(snapshots)==1
   while not (r/'waiting').exists():
    assert time.monotonic()<deadline,'Background prefetch did not reach the slow child'
    time.sleep(.02)
   probe_started=time.monotonic()
   probe=subprocess.run(['/usr/bin/gio','info','-a','standard::content-type',str(mount/'data')],capture_output=True,text=True,timeout=2,check=True)
   probe_elapsed=time.monotonic()-probe_started
   assert probe_elapsed<1,probe_elapsed
   assert not (r/'downloaded').exists(),'File-type probing downloaded content'
   print(f'GIO_CONTENT_TYPE_DURING_SDK_STALL_OK {probe_elapsed:.3f}s')
   started=time.monotonic()
   assert sorted(os.listdir(mount))==['data','sub']
   assert (mount/'data').stat().st_size==3
   assert not (mount/'unknown').exists()
   elapsed=time.monotonic()-started;assert elapsed<1,elapsed
   while True:
    metadata=json.loads(snapshots[0].read_text())
    if any(e['node']['name']=='leaf' for e in metadata['entries']):break
    assert time.monotonic()<deadline,'Nested metadata not prefetched'
    time.sleep(.1)
   assert os.listdir(mount/'sub'/'deep')==['leaf']
   assert not (r/'downloaded').exists(),'Structure preload downloaded contents'
   bookmark=(config/'gtk-3.0/bookmarks').read_text();assert ' Proton Drive' in bookmark
   # O_NOATIME remains valid for ordinary applications; normal GIO copies
   # must still hydrate and return the full, verified file contents.
   fd=os.open(mount/'data',os.O_RDONLY|os.O_NOATIME)
   try:assert os.read(fd,10)==b'abc'
   finally:os.close(fd)
   copied=r/'copied'
   subprocess.run(['/usr/bin/gio','copy',str(mount/'data'),str(copied)],check=True,timeout=10)
   assert copied.read_bytes()==b'abc'
   print('NORMAL_NOATIME_READ_AND_GIO_COPY_OK')
   print(f'CACHED_BROWSING_DURING_SDK_STALL_OK {elapsed:.3f}s; NESTED_METADATA_ONLY_PREFETCH_OK')
  finally:
   if stalled is not None and stalled.poll() is None:stalled.wait(timeout=15)
   p.terminate();p.wait(timeout=20)
   if os.path.ismount(mount):subprocess.run(['/usr/bin/fusermount3','-u',str(mount)],check=True)
