#!/usr/bin/env python3
"""Real GTK3 tree/filter adapter test; synthetic Nemo model, no user files."""
import ctypes,ctypes.util,json,os,sys,tempfile,time
from pathlib import Path
import gi
gi.require_version('Gtk','3.0')
from gi.repository import Gio,GLib,GObject,Gtk
ctypes.CDLL(ctypes.util.find_library('gtk-3'),mode=ctypes.RTLD_GLOBAL)
host=ctypes.util.find_library('nemo-extension')
if os.environ.get('NEUTRONDRIVE_TEST_HOST_LIBDIR'):
    host=str(next(Path(os.environ['NEUTRONDRIVE_TEST_HOST_LIBDIR']).glob('libnemo-extension.so.[0-9]')))
ctypes.CDLL(host,mode=ctypes.RTLD_GLOBAL)
plugin=ctypes.CDLL(str(Path(sys.argv[1]).resolve()),mode=ctypes.RTLD_GLOBAL)
capsule=ctypes.pythonapi.PyCapsule_GetPointer;capsule.argtypes=[ctypes.py_object,ctypes.c_char_p];capsule.restype=ctypes.c_void_p
class SidebarTestModule(GObject.TypeModule):
    resident=GObject.Property(type=bool,default=True)
    def do_load(self):return True
    def do_unload(self):pass
class NemoShortcutsModel(Gtk.TreeStore):
    __gtype_name__='NemoShortcutsModel'
original=Gio.ThemedIcon.new('folder-symbolic');desired=Gio.ThemedIcon.new('io.github.SigmaCodez.NeutronDrive')
def wait_for(check):
    deadline=time.monotonic()+6
    while time.monotonic()<deadline:
        while GLib.MainContext.default().iteration(False):pass
        if check():return
        time.sleep(.01)
    raise AssertionError('Nemo sidebar did not reach expected state')
with tempfile.TemporaryDirectory(prefix='nd-nemo-sidebar-') as tmp:
    os.environ['XDG_STATE_HOME']=tmp
    config=Path(tmp)/'neutrondrive/ondemand-service/mount.json';config.parent.mkdir(parents=True)
    config.write_text(json.dumps({'mountpoint':'/tmp/nd-sidebar-sync'}))
    model=NemoShortcutsModel(int,str,GObject.Object,GObject.Object,GObject.Object,str,Gio.Icon,int,bool,bool,bool,str,str,int,int,str,int,bool)
    def add(uri):
        it=model.append(None);model.set(it,1,uri,6,original);return it
    target=add('file:///tmp/nd-sidebar-sync');other=add('file:///tmp/other');remote=add('file://server/tmp/nd-sidebar-sync')
    window=Gtk.Window();window.add(Gtk.TreeView(model=model.filter_new()))
    module=SidebarTestModule();assert module.use()
    plugin.nemo_module_initialize.argtypes=[ctypes.c_void_p];plugin.nemo_module_initialize(capsule(module.__gpointer__,None))
    types=ctypes.POINTER(ctypes.c_size_t)();count=ctypes.c_int();plugin.nemo_module_list_types(ctypes.byref(types),ctypes.byref(count));assert count.value==1
    gobject=ctypes.CDLL(ctypes.util.find_library('gobject-2.0'));gobject.g_object_new.restype=ctypes.c_void_p
    provider=gobject.g_object_new(ctypes.c_size_t(types[0]),None);assert provider
    wait_for(lambda:model[target][6].equal(desired))
    assert model[other][6].equal(original) and model[remote][6].equal(original)
    model.set(target,6,original)
    wait_for(lambda:model[target][6].equal(desired))
    config.write_text(json.dumps({'mountpoint':'/tmp/other'}))
    wait_for(lambda:model[other][6].equal(desired) and model[target][6].equal(original))
    model.remove(other) # A deleted row must not be restored through a stale iterator.
    plugin.nemo_module_shutdown()
    gobject.g_object_unref.argtypes=[ctypes.c_void_p];gobject.g_object_unref(provider)
    window.destroy()
print('NEMO_SIDEBAR_SCOPE_RESET_RESTORE_AND_DELETED_ROW_OK')
