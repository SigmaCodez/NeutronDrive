import QtQuick
import NeutronDrive.Native
Item {
    id: test
    property int stage: 0
    property double workerStarted: 0
    property int fileNotifications: 0
    property int savedNotifications: 0
    property var lastDialog: ({})
    property var state: JSON.parse(files.stateJson)
    property var entries: JSON.parse(files.filesJson)
    readonly property string observedPreference: preferences.languagePreference
    readonly property string observedAppearance: preferences.appearance
    property string catalog: Qt.application.arguments[2]
    NativePreferences { id: preferences }
    NativeFiles { id: files }
    NativeFiles { id: failure }
    NativeFiles { id: cancelled }
    Connections {
        target: files
        function onFilesJsonChanged() { test.fileNotifications++ }
        function onDialogRequested(value) { test.lastDialog = JSON.parse(value) }
    }
    function require(value, message) { if (!value) throw new Error(message) }
    function send(method, args) { files.dispatch(method, JSON.stringify(args)) }
    function finish(code) { files.shutdown(); failure.shutdown(); cancelled.shutdown(); Qt.exit(code) }
    Component.onCompleted: Qt.callLater(function() {
        try {
            require(preferences.initialize(catalog), "catalog initialization");
            require(JSON.parse(preferences.languageOptionsJson).length === 26, "all language choices");
            const language = preferences.language;
            require(preferences.setLanguagePreference("de"), "save language");
            require(observedPreference === "de", "language notification");
            require(preferences.language === language, "language applies only at restart");
            require(!preferences.setLanguagePreference("missing-locale"), "invalid locale rejected");
            require(preferences.setAppearancePreference("dark"), "save appearance");
            require(observedAppearance === "dark", "appearance notification");
            require(!preferences.setAppearancePreference("invalid"), "invalid mode rejected");
            require(files.configure(catalog, "en", "/not-used-in-demo", true), "configure files");
            require(!files.configure(catalog, "de", "/not-used-in-demo", true), "reject reconfiguration");
            send("action", ["refresh"]);
            require(state.busy, "worker begins asynchronously");
            stage = 1;
        } catch (error) { console.error(error); finish(1); }
    })
    Timer {
        interval: 10; running: true; repeat: true
        onTriggered: {
            try {
                if (stage === 1 && !state.busy) {
                    require(entries.length === 6, "native demo listing");
                    savedNotifications = fileNotifications;
                    send("select_row", [0, false]);
                    require(state.selection.length === 1, "selection state notification");
                    require(fileNotifications === savedNotifications, "stable file delegate model");
                    send("edit", ["search", "NOTES"]);
                    require(entries.length === 1 && entries[0].name === "Notes.md", "filter and file notification");
                    send("select_row", [0, false]);
                    send("action", ["Umbenennen"]);
                    require(lastDialog.kind === "prompt", "native dialog signal");
                    send("answer_dialog", [lastDialog.token, true, "new.md"]);
                    require(state.busy, "mutation worker started");
                    stage = 2;
                } else if (stage === 2 && !state.busy) {
                    require(state.noticeError, "demo mutation reports failure");
                    require(entries[0].name === "Notes.md", "failed mutation retains files");
                    files.shutdown(); files.shutdown();
                    require(!files.workerRunning(), "joined file worker");
                    const before = files.stateJson;
                    send("action", ["refresh"]);
                    require(files.stateJson === before, "closed controller rejects work");
                    require(failure.configure(catalog, "en", "/missing-proton-sdk", false), "real backend controller");
                    failure.dispatch("action", '["refresh"]');
                    stage = 3;
                } else if (stage === 3 && !JSON.parse(failure.stateJson).busy) {
                    require(JSON.parse(failure.stateJson).noticeError, "native SDK error propagated");
                    failure.shutdown();
                    require(!failure.workerRunning(), "joined failed worker");
                    require(cancelled.configure(catalog, "en", Qt.application.arguments[3], false), "long-running backend");
                    cancelled.dispatch("action", '["login"]');
                    workerStarted = Date.now(); stage = 4;
                } else if (stage === 4 && Date.now() - workerStarted >= 1500) {
                    require(JSON.parse(cancelled.stateJson).busy, "backend remains active while Qt processes timers");
                    require(JSON.parse(cancelled.stateJson).fields.login_button.visible, "login event delivered on Qt thread while busy");
                    require(cancelled.stateJson.indexOf("test=synthetic") < 0, "login URL excluded from state snapshots");
                    cancelled.dispatch("action", '["cancel"]'); stage = 5;
                } else if (stage === 5 && !JSON.parse(cancelled.stateJson).busy) {
                    require(JSON.parse(cancelled.stateJson).noticeError, "cancellation is not success");
                    require(!JSON.parse(cancelled.stateJson).fields.login_button.visible, "cancelled login removes action");
                    cancelled.dispatch("action", '["refresh"]');
                    workerStarted = Date.now(); stage = 6;
                } else if (stage === 6 && Date.now() - workerStarted >= 1500) {
                    require(JSON.parse(cancelled.stateJson).busy, "slow listing before navigation");
                    let navigationStarted=Date.now();
                    cancelled.dispatch("open_section", '["Photos","photos"]');
                    require(JSON.parse(cancelled.stateJson).page === "photos", "photo section changes immediately during slow listing");
                    cancelled.dispatch("open_section", '["Shared","/shared-with-me"]');
                    require(JSON.parse(cancelled.stateJson).path === "/shared-with-me", "new destination wins while cancelling");
                    cancelled.dispatch("show_page", '["settings"]');
                    require(JSON.parse(cancelled.stateJson).page === "settings", "account settings override pending sidebar destination");
                    require(!JSON.parse(cancelled.stateJson).busy, "settings immediately available before old read finishes");
                    require(Date.now()-navigationStarted<250, "menu switches never join a slow worker");
                    cancelled.dispatch("action", '["cancel"]');
                    stage = 7;
                } else if (stage === 7 && !JSON.parse(cancelled.stateJson).busy) {
                    require(JSON.parse(cancelled.stateJson).page === "settings", "cancel and late results cannot restore old page");
                    require(!JSON.parse(cancelled.stateJson).noticeError, "obsolete read cancellation stays silent");
                    cancelled.dispatch("action", '["refresh"]');
                    workerStarted = Date.now(); stage = 8;
                } else if (stage === 8 && Date.now() - workerStarted >= 1500) {
                    require(JSON.parse(cancelled.stateJson).busy, "live backend before shutdown");
                    cancelled.shutdown(); cancelled.shutdown();
                    require(!cancelled.workerRunning(), "shutdown joins and reaps in-flight backend");
                    console.log("NATIVE_CONTROLLER_OK"); finish(0);
                }
            } catch (error) { console.error(error); finish(1); }
        }
    }
    Timer { interval: 15000; running: true; onTriggered: finish(2) }
}
