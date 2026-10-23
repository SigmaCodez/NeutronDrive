import QtQuick
import NeutronDrive.Native
Item {
    NativeAppearance { id: appearance }
    readonly property bool observedDark: appearance.dark
    readonly property string observedMode: appearance.mode
    function require(value, text) { if (!value) throw new Error(text) }
    Component.onCompleted: Qt.callLater(function() {
        try {
            appearance.setMode("dark"); require(observedDark, "manual dark override and change notification");
            appearance.setMode("light"); require(!observedDark, "manual light override and change notification");
            appearance.setMode("invalid"); require(observedMode === "system", "invalid mode fallback and change notification");
            appearance.setQtFallback(true); require(observedDark, "Qt dark fallback without a portal");
            appearance.setQtFallback(false); require(!observedDark, "Qt light fallback without a portal");
            appearance.shutdown(); appearance.shutdown();
            require(!appearance.watcherRunning(), "watcher shutdown");
            console.log("NATIVE_APPEARANCE_OK"); Qt.exit(0);
        } catch (error) { console.error(error); Qt.exit(1); }
    })
    Timer { interval: 10000; running: true; onTriggered: Qt.exit(2) }
}
