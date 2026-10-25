#!/usr/bin/env python3
"""Native adapter with real GTK4/GObject, synthetic sidebar rows, no user files.
Requires python3-gi, GTK4, libnautilus-extension4 and a display (Xvfb works).
"""
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, GLib, GObject, Gtk
ctypes.CDLL(ctypes.util.find_library('gtk-4'), mode=ctypes.RTLD_GLOBAL)
ctypes.CDLL(ctypes.util.find_library('nautilus-extension'), mode=ctypes.RTLD_GLOBAL)
plugin = ctypes.CDLL(str(Path(sys.argv[1]).resolve()), mode=ctypes.RTLD_GLOBAL)
capsule = ctypes.pythonapi.PyCapsule_GetPointer
capsule.argtypes = [ctypes.py_object, ctypes.c_char_p]
capsule.restype = ctypes.c_void_p
class SidebarTestModule(GObject.TypeModule):
    resident = GObject.Property(type=bool, default=True)
    def do_load(self): return True
    def do_unload(self): pass
class NautilusSidebarRow(Gtk.Box):
    __gtype_name__ = 'NautilusSidebarRow'
    uri = GObject.Property(type=str)
    start_icon = GObject.Property(type=Gio.Icon)
original = Gio.ThemedIcon.new('folder-symbolic')
def row(uri): return NautilusSidebarRow(uri=uri, start_icon=original)
def wait_for(check):
    deadline = time.monotonic() + 6
    while time.monotonic() < deadline:
        while GLib.MainContext.default().iteration(False): pass
        if check(): return
        time.sleep(.01)
    raise AssertionError('Sidebar did not reach the expected state')
def custom(item):
    return item.props.start_icon.equal(Gio.ThemedIcon.new('io.github.SigmaCodez.NeutronDrive'))
with tempfile.TemporaryDirectory(prefix='nd-sidebar-') as tmp:
    os.environ['XDG_STATE_HOME'] = tmp
    config = Path(tmp) / 'neutrondrive/ondemand-service/mount.json'
    config.parent.mkdir(parents=True)
    config.write_text(json.dumps({'mountpoint': '/tmp/nd-sidebar-sync'}))
    window = Gtk.Window()
    box = Gtk.Box()
    window.set_child(box)
    target, other, remote = (row(uri) for uri in ('file:///tmp/nd-sidebar-sync', 'file:///tmp/other', 'smb://server/tmp/nd-sidebar-sync'))
    for item in (target, other, remote): box.append(item)
    remote_file = row("file://server/tmp/nd-sidebar-sync")
    box.append(remote_file)
    module = SidebarTestModule()
    assert module.use()
    plugin.nautilus_module_initialize.argtypes = [ctypes.c_void_p]
    plugin.nautilus_module_initialize(capsule(module.__gpointer__, None))
    types = ctypes.POINTER(ctypes.c_size_t)()
    count = ctypes.c_int()
    plugin.nautilus_module_list_types(ctypes.byref(types), ctypes.byref(count))
    assert count.value == 1
    gobject = ctypes.CDLL(ctypes.util.find_library('gobject-2.0'))
    gobject.g_object_new.restype = ctypes.c_void_p
    provider = gobject.g_object_new(ctypes.c_size_t(types[0]), None)
    assert provider
    wait_for(lambda: custom(target))
    assert other.props.start_icon.equal(original)
    assert remote.props.start_icon.equal(original)
    assert remote_file.props.start_icon.equal(original)
    target.props.start_icon = original
    window2 = Gtk.Window()
    target2 = row('file:///tmp/nd-sidebar-sync')
    window2.set_child(target2)
    wait_for(lambda: custom(target) and custom(target2))
    config.write_text(json.dumps({'mountpoint': '/tmp/other'}))
    wait_for(lambda: custom(other) and target.props.start_icon.equal(original) and target2.props.start_icon.equal(original))
    window2.destroy()
    plugin.nautilus_module_shutdown()
    assert other.props.start_icon.equal(original)
    gobject.g_object_unref.argtypes = [ctypes.c_void_p]
    gobject.g_object_unref(provider)
    window.destroy()
print('NAUTILUS_SIDEBAR_ICON_RESTORE_NEW_WINDOW_AND_SCOPE_OK')
