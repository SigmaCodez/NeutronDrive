#!/usr/bin/env python3
"""Exercise the compiled SDK's offline namespace using synthetic credentials only."""
import json,os,pathlib,subprocess,sys,tempfile,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='nd-local-identity-') as tmp:
    root=pathlib.Path(tmp);data=root/'data'/'proton-drive-cli';data.mkdir(parents=True)
    session=data/'auth-session.json'
    env=dict(os.environ,HOME=tmp,XDG_DATA_HOME=str(root/'data'),XDG_CACHE_HOME=str(root/'cache'),XDG_STATE_HOME=str(root/'state'),PROTON_DRIVE_CREDENTIALS_STORE='unsafe_file',PROTON_DRIVE_BASE_URL='127.0.0.1:1')
    def identity():
        start=time.monotonic()
        p=subprocess.run([binary,'cache','identity'],env=env,text=True,capture_output=True,check=True,timeout=5)
        assert time.monotonic()-start<3,'Local identity must not wait for cloud initialization'
        assert 'synthetic-' not in p.stdout+p.stderr,'Credential material leaked'
        assert not (root/'state').exists(),'Local identity initialized SDK logging/network session'
        return json.loads(p.stdout)['namespace']
    assert identity() is None
    credentials={'userKeyPassword':'synthetic-password','session':{'uid':'synthetic-account-a','accessToken':'synthetic-token'}}
    session.write_text(json.dumps(credentials));first=identity();assert len(first)==64
    credentials['session']['accessToken']='synthetic-refreshed-token';session.write_text(json.dumps(credentials));assert identity()==first
    credentials['session']['uid']='synthetic-account-b';session.write_text(json.dumps(credentials));assert identity()!=first
    session.write_text('broken');assert identity() is None
    session.unlink();assert identity() is None
    print('OFFLINE_LOCAL_CACHE_IDENTITY_OK')
