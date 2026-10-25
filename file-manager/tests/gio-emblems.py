#!/usr/bin/env python3
"""Run only in an isolated HOME/XDG and dbus-run-session; synthetic file only."""
import gi,json,os,pathlib,subprocess,sys
from gi.repository import Gio
p=pathlib.Path(os.environ['HOME'])/'mount';p.mkdir();f=p/'test file.txt';f.write_text('synthetic')
c=pathlib.Path(os.environ['XDG_STATE_HOME'])/'neutrondrive/ondemand-service';c.mkdir(parents=True);(c/'mount.json').write_text(json.dumps({'mountpoint':str(p)}))
g=Gio.File.new_for_path(str(f));i=Gio.FileInfo.new();i.set_attribute_stringv('metadata::emblems',['emblem-favorite']);g.set_attributes_from_info(i,Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,None)
def emblems():return g.query_info('metadata::emblems',Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,None).get_attribute_stringv('metadata::emblems')
for flag,want in [('1',['emblem-favorite','neutrondrive-available']),('0',['emblem-favorite'])]:
 subprocess.run([sys.argv[1],'--apply-emblem',flag,str(f)],check=True)
 assert emblems()==want,emblems()
print('GIO_EMBLEM_ADD_REMOVE_PRESERVES_USER_EMBLEMS_OK')
