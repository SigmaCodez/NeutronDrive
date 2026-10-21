"""Settings portal integration on a private D-Bus, without changing desktop settings."""
import os
import subprocess
import sys
import unittest

PROBE = r'''
import time
from gi.repository import Gio, GLib
from PySide6.QtGui import QGuiApplication
from neutrondrive.appearance import SystemAppearance
app = QGuiApplication([])
connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
    'org.freedesktop.DBus', 'RequestName',
    GLib.Variant('(su)', ('org.freedesktop.portal.Desktop', 0)),
    None, Gio.DBusCallFlags.NONE, 2000, None)
xml = '<node><interface name="org.freedesktop.portal.Settings"><method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method><signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal></interface></node>'
requests = []
def read(conn, sender, path, interface, method, params, invocation):
    assert params.unpack() == ('org.freedesktop.appearance', 'color-scheme')
    requests.append(invocation)
connection.register_object('/org/freedesktop/portal/desktop', Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], read, None, None)
def pump(predicate, timeout=3):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        app.processEvents()
        context = GLib.MainContext.default()
        while context.pending(): context.iteration(False)
        if predicate(): return
        time.sleep(.005)
    raise AssertionError('Appearance event timed out')
def signal(value):
    connection.emit_signal(None, '/org/freedesktop/portal/desktop', 'org.freedesktop.portal.Settings',
        'SettingChanged', GLib.Variant('(ssv)', ('org.freedesktop.appearance','color-scheme',GLib.Variant('u',value))))
def reply(invocation, value):
    # Read has historically returned two variant layers; unpack must handle both.
    invocation.return_value(GLib.Variant('(v)', (GLib.Variant('v', GLib.Variant('u', value)),)))
appearance = SystemAppearance(app)
pump(lambda: len(requests)==1)
reply(requests.pop(), 1)
pump(lambda: appearance.dark)
signal(2)
pump(lambda: not appearance.dark)
signal(0)
pump(lambda: appearance._portal_scheme==0)
appearance.close()
# A notification received during startup must take precedence over a stale read.
appearance = SystemAppearance(app)
pump(lambda: len(requests)==1)
signal(1)
pump(lambda: appearance.dark)
reply(requests.pop(), 2)
end=time.monotonic()+.1
pump(lambda: time.monotonic()>end)
assert appearance.dark
appearance.close()
signal(2)
end=time.monotonic()+.1
pump(lambda: time.monotonic()>end)
assert appearance.dark
print('PORTAL_INITIAL_CHANGE_RESET_RACE_CLOSE_OK')
'''

@unittest.skipUnless(os.environ.get('NEUTRONDRIVE_UI_TESTS')=='1' and os.environ.get('NEUTRONDRIVE_DBUS_TEST')=='1', 'Enable Qt and isolated D-Bus tests')
class AppearancePortalTests(unittest.TestCase):
    def test_real_portal_messages_initial_read_change_and_race(self):
        result=subprocess.run(['dbus-run-session','--',sys.executable,'-c',PROBE],
                              text=True,capture_output=True,timeout=15)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertIn('PORTAL_INITIAL_CHANGE_RESET_RACE_CLOSE_OK',result.stdout)
