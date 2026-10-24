#!/usr/bin/env python3
"""Exercise Qt's actual StatusNotifierItem registration on an isolated test bus.
Requires python3-gi; run with dbus-run-session and the test session.conf.
No real account, desktop tray, preferences or cloud API is used.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from gi.repository import Gio, GLib

connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
xml = '''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
</interface></node>'''
registered = []
def method(conn, sender, path, interface, name, parameters, invocation):
    value = parameters.unpack()[0]
    registered.append((sender if value.startswith('/') else value, value if value.startswith('/') else '/StatusNotifierItem'))
    invocation.return_value(None)
def prop(conn, sender, path, interface, name):
    return {'IsStatusNotifierHostRegistered': GLib.Variant('b', True), 'RegisteredStatusNotifierItems': GLib.Variant('as', []), 'ProtocolVersion': GLib.Variant('i', 0)}[name]
connection.register_object('/StatusNotifierWatcher', Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], method, prop, None)
connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.kde.StatusNotifierWatcher', 0)), None, Gio.DBusCallFlags.NONE, 2000, None)
loop = GLib.MainLoop()
thread = threading.Thread(target=loop.run, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix='nd-tray-') as temp:
        root = Path(temp)
        sdk = root/'sdk'
        sdk.write_text('#!/bin/sh\nprintf "[]\\n"\n')
        sdk.chmod(0o700)
        env = dict(os.environ, XDG_CONFIG_HOME=str(root/'config'), XDG_STATE_HOME=str(root/'state'), XDG_CACHE_HOME=str(root/'cache'), QT_QPA_PLATFORM='xcb', QT_QUICK_BACKEND='software', LANGUAGE='en', LANG='en_US.UTF-8', LC_ALL='en_US.UTF-8')
        for key in ('SNAP','FLATPAK_ID','APPIMAGE','APPDIR'): env.pop(key, None)
        process = subprocess.Popen([sys.argv[1], '--cli', str(sdk)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic()+6
            while not registered and process.poll() is None and time.monotonic()<deadline: time.sleep(.05)
            assert registered, 'Tray did not register with the synthetic StatusNotifier host'
            destination, path = registered[0]
            response = connection.call_sync(destination, path, 'org.freedesktop.DBus.Properties', 'GetAll', GLib.Variant('(s)', ('org.kde.StatusNotifierItem',)), None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
            assert response['Status'] == 'Active', response
            assert response.get('IconName') or response.get('IconPixmap'), 'Tray has no icon'
            menu = response['Menu']
            layout = connection.call_sync(destination, menu, 'com.canonical.dbusmenu', 'GetLayout', GLib.Variant('(iias)', (0, -1, ['label'])), None, Gio.DBusCallFlags.NONE, 2000, None).unpack()
            assert 'Open NeutronDrive' in str(layout) and 'Start sync service' in str(layout), layout
            connection.call_sync(destination, path, 'org.kde.StatusNotifierItem', 'Activate', GLib.Variant('(ii)', (0,0)), None, Gio.DBusCallFlags.NONE, 2000, None)
            print('NATIVE_TRAY_REGISTRATION_MENU_AND_ACTIVATION_OK')
        finally:
            process.terminate()
            out, err = process.communicate(timeout=5)
            if not registered: print(out+err, file=sys.stderr)
finally:
    loop.quit()
    thread.join(timeout=2)
