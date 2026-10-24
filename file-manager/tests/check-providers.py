#!/usr/bin/env python3
"""Real host libraries, isolated synthetic file-info URIs, no account or file mutations."""
import ctypes.util,json,os,pathlib,subprocess,sys,tempfile
root=pathlib.Path(__file__).resolve().parent
modern=pathlib.Path(sys.argv[1]).resolve();legacy=pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix='nd-menu-provider-') as t:
 p=pathlib.Path(t);config=p/'state/neutrondrive/ondemand-service';config.mkdir(parents=True)
 (config/'mount.json').write_text(json.dumps({'mountpoint':'/tmp/nd-menu-sync'}))
 flags=subprocess.check_output(['pkg-config','--cflags','--libs','gobject-2.0','gmodule-2.0'],text=True).split()
 subprocess.run(['cc',str(root/'provider.c'),'-o',str(p/'provider'),'-Wl,--export-dynamic',*flags],check=True)
 env=dict(os.environ,XDG_STATE_HOME=str(p/'state'),LANG='en_US.UTF-8',LC_ALL='C',G_DEBUG='fatal-warnings')
 for name,lib in [('nautilus','nautilus-extension'),('nemo','nemo-extension'),('caja','caja-extension'),('thunarx','thunarx-3')]:
  library=ctypes.util.find_library(lib)
  extra=os.environ.get("NEUTRONDRIVE_TEST_HOST_LIBDIR")
  if extra:
   found=sorted(pathlib.Path(extra).glob("lib"+lib+".so.[0-9]"))
   if found:library=str(found[0])
  if not library:raise SystemExit('Missing integration-test library: '+lib)
  old=name=='nautilus' and '.so.4' not in library
  subprocess.run([str(p/'provider'),'nautilus3' if old else name,library,str(legacy if old else modern)],env=env,check=True)
print('ALL_NATIVE_FILE_MANAGER_PROVIDERS_OK')
