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

# Install a private copy with the same prefix layout as native packages.
import shutil
prefix=pathlib.Path(os.environ['HOME'])/'install';helper=prefix/'lib/neutrondrive/bin/neutrondrive-file-manager'
helper.parent.mkdir(parents=True);shutil.copy2(sys.argv[1],helper)
icon=prefix/'share/icons/hicolor/scalable/apps/io.github.SigmaCodez.NeutronDrive.svg'
icon.parent.mkdir(parents=True);icon.write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
subprocess.run([str(helper),'--apply-root-icon'],check=True)
folder=Gio.File.new_for_path(str(p))
def custom():return folder.query_info('metadata::*',Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,None)
info=custom();assert info.get_attribute_string('metadata::custom-icon')==icon.as_uri()
assert info.get_attribute_string('metadata::thunar-custom-icon-name')=='io.github.SigmaCodez.NeutronDrive'
assert not g.query_info('metadata::*',Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,None).has_attribute('metadata::custom-icon')
# A user's independently chosen folder icon survives a later refresh.
folder.set_attribute_string('metadata::custom-icon','file:///tmp/user-chosen.svg',Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,None)
subprocess.run([str(helper),'--apply-root-icon'],check=True)
assert custom().get_attribute_string('metadata::custom-icon')=='file:///tmp/user-chosen.svg'
print('GIO_ROOT_ICON_SCOPED_AND_USER_CUSTOMIZATION_PRESERVED_OK')
