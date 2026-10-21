import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQuick.Dialogs

ApplicationWindow {
    id: window
    objectName: "mainWindow"
    width: 1220; height: 820
    minimumWidth: 900; minimumHeight: 640
    visible: true
    title: "NeutronDrive" + (s.demo ? " · DEMO" : "")
    color: "#0d131c"
    font.family: "Sans Serif"; font.pixelSize: 14
    palette.text: "#e8edf5"; palette.windowText: "#e8edf5"
    palette.buttonText: "#e8edf5"; palette.highlight: "#71e0bf"
    palette.base: "#18232f"; palette.button: "#223341"
    property var s: controller.state
    property color muted: "#9caec2"
    property var titles: ({"files":s.fields.files_title.value,"sync":"Synchronisierung","background":"Automatischer Sync","ondemand":"Online-Laufwerk","local-trash":"Lokaler Papierkorb","activity":"Aktivität","settings":"Einstellungen"})
    function f(name) { return s.fields[name] }
    onClosing: function(event) { event.accepted = controller.request_close() }

    component Copy: Label {
        color: window.muted; wrapMode: Text.Wrap; textFormat: Text.PlainText
        Layout.fillWidth: true
    }
    component Heading: Label {
        color: "#e8edf5"; font.pixelSize: 18; font.bold: true; textFormat: Text.PlainText
        Layout.fillWidth: true
    }
    component Action: Button {
        id: actionButton
        property string command
        property string field: ""
        property bool primary: false
        enabled: field ? window.f(field).enabled : !s.busy
        onClicked: controller.action(command)
        implicitHeight: 40
        padding: 12
        contentItem: Text {
            text: actionButton.text; textFormat: Text.PlainText; font: actionButton.font
            color: !actionButton.enabled ? "#81909f" : actionButton.primary ? "#0b2c24" : "#e8edf5"
            horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 9
            color: !actionButton.enabled ? "#18222d" : actionButton.primary ? "#71e0bf" : actionButton.hovered ? "#304354" : "#223341"
            border.color: actionButton.activeFocus ? "#71e0bf" : "#344655"
            Behavior on color { ColorAnimation { duration: 100 } }
        }
    }
    component Input: TextField {
        id: input
        property string field
        text: window.f(field).value
        enabled: window.f(field).enabled && !s.busy
        selectByMouse: true
        color: "#e8edf5"; placeholderTextColor: window.muted
        Layout.fillWidth: true; implicitHeight: 44; padding: 12
        onTextEdited: controller.edit(field, text)
        background: Rectangle { color: "#101b28"; radius: 9; border.color: input.activeFocus ? "#71e0bf" : "#344655" }
    }
    component Card: Rectangle {
        color: "#151f2b"; radius: 14; border.color: "#273544"
        Layout.fillWidth: true
        implicitHeight: cardContents.implicitHeight + 36
        default property alias contents: cardContents.data
        ColumnLayout { id: cardContents; anchors { left: parent.left; right: parent.right; top: parent.top; margins: 18 } spacing: 12 }
    }
    component Toggle: CheckBox {
        id: check
        property string field
        checked: window.f(field).value
        enabled: !s.busy
        onToggled: controller.edit(field, checked)
        contentItem: Text { text: check.text; textFormat: Text.PlainText; color: "#e8edf5"; leftPadding: check.indicator.width + check.spacing; verticalAlignment: Text.AlignVCenter }
    }

    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 230; Layout.fillHeight: true; color: "#121c28"
            ScrollView {
                id: sidebarScroll
                anchors.fill: parent; anchors.margins: 18; clip: true; contentWidth: availableWidth
            ColumnLayout {
                width: sidebarScroll.availableWidth; height: Math.max(implicitHeight,sidebarScroll.availableHeight); spacing: 8
                Label { text: "◈  NeutronDrive"; font.pixelSize: 21; font.bold: true; color: "#e8edf5"; Layout.topMargin: 10 }
                Copy { text: "Deine Dateien. Dein Speicher."; font.pixelSize: 12; Layout.bottomMargin: 24 }
                Repeater {
                    model: [{name:"Meine Dateien",key:"/my-files",icon:"▤"},{name:"Mit mir geteilt",key:"/shared-with-me",icon:"◇"},{name:"Computer",key:"/devices",icon:"▣"},{name:"Cloud-Papierkorb",key:"/trash",icon:"♲"},
                        {name:"Synchronisierung",key:"sync",icon:"⇄"},{name:"Automatischer Sync",key:"background",icon:"↻"},{name:"Online-Laufwerk",key:"ondemand",icon:"☁"},{name:"Lokaler Papierkorb",key:"local-trash",icon:"♲"},{name:"Aktivität",key:"activity",icon:"≋"},{name:"Einstellungen",key:"settings",icon:"⚙"}]
                    delegate: Button {
                        id: nav
                        required property var modelData
                        property bool selected: modelData.key.charAt(0)==="/" ? s.page==="files" && ("/"+s.path.split("/")[1])===modelData.key : s.page===modelData.key
                        objectName: "nav-"+modelData.key
                        Layout.fillWidth: true; implicitHeight: 42
                        text: modelData.name
                        onClicked: modelData.key.charAt(0)==="/" ? controller.navigate(modelData.name,modelData.key) : controller.show_page(modelData.key)
                        contentItem: Text { text: nav.modelData.icon+"   "+nav.text; color: nav.selected ? "#91f1d1" : "#b9c6d5"; verticalAlignment: Text.AlignVCenter; leftPadding: 12; font.pixelSize: 13 }
                        background: Rectangle { radius: 9; color: nav.selected ? "#213f3c" : nav.hovered ? "#1b2b3b" : "transparent"; border.color: nav.activeFocus ? "#71e0bf" : "transparent" }
                    }
                }
                Item { Layout.fillHeight: true }
                Action { text: "Bei Proton anmelden"; command: "login"; Layout.fillWidth: true }
                Action { text: "Proton Drive im Web ↗"; command: "web"; Layout.fillWidth: true }
                Copy { text: "Unabhängige Open-Source-App\n"+s.version+(s.demo ? " · DEMO" : ""); font.pixelSize: 11; Layout.topMargin: 10 }
            }
        }
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.margins: 28; spacing: 16
            RowLayout {
                Layout.fillWidth: true
                ColumnLayout {
                    Layout.fillWidth: true; spacing: 5
                    Copy { text: "DEIN PERSÖNLICHER SPEICHER"; font.pixelSize: 10; font.letterSpacing: 1.5 }
                    Label { text: window.titles[s.page]; textFormat: Text.PlainText; font.pixelSize: 29; font.bold: true; color: "#f0f4fa"; elide: Text.ElideRight; Layout.fillWidth: true }
                }
                Rectangle { implicitWidth: modeLabel.implicitWidth+24; implicitHeight: 30; radius: 15; color: "#213c36"
                    Label { id: modeLabel; anchors.centerIn: parent; text: s.demo ? "Demo" : "Proton SDK"; color: "#91f1d1"; font.pixelSize: 12 }
                }
            }
            Rectangle {
                visible: window.f("notice_label").visible; Layout.fillWidth: true; implicitHeight: notice.implicitHeight+24
                radius: 10; color: s.noticeError ? "#423323" : "#203a3c"
                Copy { id: notice; anchors { left: parent.left; right: parent.right; margins: 12; verticalCenter: parent.verticalCenter } text: window.f("notice_label").value; color: "#e8edf5" }
            }
            Action { visible: window.f("login_button").visible; text: "Anmeldung im Browser öffnen"; command: "openLogin" }
            Loader { id: pageLoader; objectName: "pageLoader"; Layout.fillWidth: true; Layout.fillHeight: true
                sourceComponent: s.page==="files" ? filesPage : s.page==="sync" ? syncPage : s.page==="background" ? backgroundPage : s.page==="ondemand" ? mountPage : s.page==="local-trash" ? trashPage : s.page==="activity" ? activityPage : settingsPage
            }
            RowLayout {
                Layout.fillWidth: true
                BusyIndicator { running: s.busy; visible: s.busy; implicitWidth: 24; implicitHeight: 24 }
                Copy { text: window.f("status_label").value; font.pixelSize: 12 }
                Action { visible: s.busy; text: "Abbrechen"; command: "cancel"; enabled: true }
            }
        }
    }

    Component {
        id: filesPage
        ColumnLayout {
            objectName: "page-files"; spacing: 14
            Input { field: "search"; placeholderText: "In diesem Ordner suchen …"; objectName: "searchInput" }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action { text: "Dateien hochladen"; command: "upload"; field: "upload_button"; primary: true }
                Action { text: "Ordner hochladen"; command: "uploadFolder"; field: "folder_upload_button" }
                Action { text: "Neuer Ordner"; command: "mkdir"; field: "mkdir_button" }
                Action { text: "Aktualisieren"; command: "refresh"; field: "refresh_button" }
            }
            Flow { Layout.fillWidth: true; spacing: 4
                Repeater { model: s.crumbs; delegate: Button { required property string modelData; required property int index; text: modelData+"  ›"; flat: true; onClicked: controller.crumb(index) } }
            }
            Rectangle {
                Layout.fillHeight: true; Layout.fillWidth: true; radius: 12; color: "#151f2b"; border.color: "#273544"
                ColumnLayout {
                    anchors.fill: parent; anchors.margins: 12; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; Layout.margins: 10
                        Copy { text: "NAME"; font.pixelSize: 10; font.letterSpacing: 1 }
                        Label { text: "GRÖSSE"; color: window.muted; font.pixelSize: 10; Layout.preferredWidth: 80 }
                        Label { text: "GEÄNDERT"; color: window.muted; font.pixelSize: 10; Layout.preferredWidth: 90 }
                    }
                    ListView {
                        id: fileList; objectName: "fileList"; Layout.fillWidth: true; Layout.fillHeight: true
                        clip: true; model: s.files; spacing: 4; boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Rectangle {
                            id: fileRow
                            required property var modelData; required property int index
                            width: fileList.width; height: 62; radius: 8
                            color: modelData.selected ? "#25473f" : rowMouse.containsMouse ? "#1d2d3d" : "transparent"
                            border.color: activeFocus ? "#71e0bf" : "transparent"
                            activeFocusOnTab: true
                            Accessible.role: Accessible.ListItem; Accessible.name: modelData.name; Accessible.selected: modelData.selected
                            Keys.onSpacePressed: controller.select_row(index,true)
                            Keys.onReturnPressed: controller.row_activated(index)
                            RowLayout {
                                anchors.fill: parent; anchors.margins: 10; spacing: 12
                                Label { text: fileRow.modelData.kind==="Ordner" ? "▰" : "▤"; color: "#71e0bf"; font.pixelSize: 25; Layout.preferredWidth: 30 }
                                ColumnLayout { Layout.fillWidth: true; spacing: 3
                                    Label { text: fileRow.modelData.name; textFormat: Text.PlainText; color: "#e8edf5"; elide: Text.ElideRight; Layout.fillWidth: true }
                                    Copy { text: fileRow.modelData.safe ? fileRow.modelData.kind : "Nicht eindeutig · gesperrt"; font.pixelSize: 11 }
                                }
                                Label { text: fileRow.modelData.size; color: window.muted; Layout.preferredWidth: 80; font.pixelSize: 12 }
                                Label { text: fileRow.modelData.modified; color: window.muted; Layout.preferredWidth: 90; font.pixelSize: 12 }
                            }
                            MouseArea { id: rowMouse; anchors.fill: parent; hoverEnabled: true; enabled: !s.busy
                                onClicked: function(mouse) { fileRow.forceActiveFocus(); controller.select_row(fileRow.index,Boolean(mouse.modifiers & (Qt.ControlModifier|Qt.ShiftModifier))) }
                                onDoubleClicked: controller.row_activated(fileRow.index)
                            }
                        }
                        Label { anchors.centerIn: parent; visible: s.files.length===0; text: window.f("empty_label").value; color: window.muted }
                    }
                }
            }
            Flow { Layout.fillWidth: true; spacing: 8
                Repeater { model: ["Download","Umbenennen","Verschieben","Freigabe","Papierkorb","Wiederherstellen"]
                    delegate: Action { required property string modelData; text: modelData; command: modelData; enabled: s.actions[modelData].enabled; visible: s.actions[modelData].visible }
                }
            }
            Copy { text: window.f("count_label").value; font.pixelSize: 12 }
        }
    }
    component PageScroll: ScrollView {
        id: scroll
        clip: true; contentWidth: availableWidth
        default property alias contents: pageColumn.data
        ColumnLayout { id: pageColumn; width: scroll.availableWidth; spacing: 16 }
    }
    Component {
        id: syncPage
        PageScroll {
            objectName: "page-sync"
            Copy { text: "Erst Vorschau prüfen, dann abgleichen. Konflikte bleiben erhalten." }
            Card {
                Heading { text: "Ordner verbinden" }
                Copy { text: window.f("local_label").value }
                Action { text: "Lokalen Ordner wählen"; command: "chooseSync"; field: "local_choose" }
                Copy { text: "Cloud-Ordner" }
                Input { field: "remote_entry" }
                RowLayout {
                    Action { text: "Vorschau erstellen"; command: "plan"; field: "plan_button"; primary: true }
                    Action { text: "Geprüften Plan ausführen"; command: "execute"; field: "execute_button" }
                }
            }
            Card {
                Heading { text: "Sync-Vorschau" }
                TextArea { text: window.f("sync_text").value; textFormat: Text.PlainText; readOnly: true; selectByMouse: true; wrapMode: Text.Wrap; color: "#d3deeb"; Layout.fillWidth: true; background: null }
            }
            Copy { text: "Dateilöschungen werden in den Papierkorb übertragen. Ganze Ordnerlöschungen bleiben als Konflikt erhalten. Ersetzte lokale Versionen werden gesichert." }
        }
    }
    Component {
        id: backgroundPage
        PageScroll {
            objectName: "page-background"
            Copy { text: "Änderungen automatisch abgleichen – auch nach dem Schließen des Fensters." }
            Card {
                Heading { text: "Neuen Auftrag einrichten" }
                Copy { text: window.f("background_local_label").value }
                Action { text: "Lokalen Ordner wählen"; command: "chooseBackground" }
                Copy { text: "Cloud-Ordner" }
                Input { field: "background_remote" }
                Action { text: "Automatischen Sync aktivieren"; command: "addJob"; field: "background_add_button"; primary: true }
            }
            Card {
                Heading { text: window.f("background_status").value }
                RowLayout { Action { text: "Dienst starten"; command: "start" } Action { text: "Dienst stoppen"; command: "stop" } }
            }
            Repeater { model: s.jobs
                delegate: Card { required property var modelData
                    Heading { text: modelData.name }
                    Copy { text: modelData.detail }
                    Copy { text: modelData.status }
                    Flow { Layout.fillWidth: true; spacing: 8
                        Action { text: modelData.paused ? "Fortsetzen" : "Pausieren"; onClicked: controller.job_action(modelData.paused ? "resume" : "pause",modelData.id) }
                        Action { text: "Jetzt abgleichen"; onClicked: controller.job_action("sync",modelData.id) }
                        Action { text: "Auftrag entfernen"; onClicked: controller.job_action("remove",modelData.id) }
                    }
                }
            }
            Copy { text: "Dateilöschungen nur bei unverändertem Gegenstück; ganze Ordnerlöschungen bleiben Konflikte. Anmeldung und entsperrter Schlüsselbund erforderlich." }
        }
    }
    Component {
        id: mountPage
        PageScroll {
            objectName: "page-ondemand"
            Copy { text: "Dateien beim Öffnen laden, offline vorhalten oder lokalen Speicher freigeben." }
            Card {
                Heading { text: "Online-Laufwerk verbinden" }
                Copy { text: window.f("mount_label").value }
                Action { text: "Leeren Einhängeordner wählen"; command: "chooseMount" }
                Copy { text: "Cloud-Ordner" }
                Input { field: "mount_remote" }
                Toggle { field: "mount_writable"; text: "Dateien im Laufwerk bearbeiten und hochladen" }
                Toggle { field: "mount_offline"; text: "Offline aus vorhandenem Cache öffnen" }
                Flow { Layout.fillWidth: true; spacing: 8
                    Action { text: "Laufwerk verbinden"; command: "mount"; field: "mount_button"; primary: true }
                    Action { text: "Trennen"; command: "unmount" }
                    Action { text: "Aktualisieren"; command: "refreshMount" }
                }
                Copy { text: window.f("mount_status").value }
            }
            Card {
                Heading { text: "Offline-Verfügbarkeit" }
                Input { field: "cache_path"; placeholderText: "Dateipfad, z. B. Dokumente/Notizen.txt" }
                RowLayout { Action { text: "Offline vorhalten"; command: "pin" } Action { text: "Speicher freigeben"; command: "evict" } }
                Action { text: "Laufwerkshelfer bei Proton anmelden"; command: "helperLogin" }
            }
            Copy { text: "Für Flatpak zusätzlich den nativen Laufwerkshelfer installieren. Online-Laufwerke sind im Snap derzeit nicht verfügbar. Nicht hochgeladene Änderungen bleiben im Wiederherstellungsspeicher." }
        }
    }
    Component {
        id: trashPage
        PageScroll {
            objectName: "page-local-trash"
            Copy { text: "Nach Cloud-Löschung lokal gesicherte Dateien. Wiederherstellen überschreibt keine vorhandenen Dateien." }
            Action { text: "Aktualisieren"; command: "refreshTrash" }
            Copy { text: window.f("local_trash_status").value }
            Copy { text: window.f("local_trash_empty").value; visible: s.trash.length===0 }
            Repeater { model: s.trash
                delegate: Card { required property var modelData; required property int index
                    Heading { text: modelData.name }
                    Copy { text: modelData.detail }
                    Copy { text: modelData.error+" "+modelData.restorePath }
                    Action { text: "Wiederherstellen / Wiederherstellung prüfen"; enabled: modelData.enabled && !s.busy; onClicked: controller.restore_entry(index) }
                }
            }
            Copy { text: "Bei aktivem Sync wird eine Wiederherstellung erneut synchronisiert. NeutronDrive leert den Papierkorb nicht automatisch." }
        }
    }
    Component {
        id: activityPage
        PageScroll {
            objectName: "page-activity"
            Copy { text: "Vorgänge dieser Sitzung · keine dauerhafte Speicherung von Dateinamen" }
            Copy { visible: s.activity.length===0; text: "Noch keine Vorgänge in dieser Sitzung." }
            Repeater { model: s.activity
                delegate: Card { required property var modelData
                    Heading { text: modelData.title }
                    Copy { text: modelData.time+" · "+modelData.status }
                }
            }
        }
    }
    Component {
        id: settingsPage
        PageScroll {
            objectName: "page-settings"
            Card {
                Heading { text: "Konto & Verbindung" }
                Copy { text: window.f("backend_label").value }
                RowLayout { Action { text: "Backend prüfen"; command: "version" } Action { text: "Anmelden"; command: "login" } Action { text: "Abmelden"; command: "logout" } }
                Copy { text: "Anmeldung ausschließlich auf account.proton.me im Browser. Die Sitzung liegt im Schlüsselbund oder in pass. Flatpak kann einen eigenen Kontospeicher verwenden." }
            }
            Card {
                Heading { text: "Beim Anmelden starten" }
                Copy { text: "Halte deine automatischen Sync-Aufträge nach dem Desktop-Start bereit." }
                RowLayout { Action { text: "Autostart aktivieren"; command: "autostart" } Action { text: "Deaktivieren"; command: "noAutostart" } }
            }
            Card {
                Heading { text: "NeutronDrive "+s.version }
                Copy { text: "Qt Quick · QML · Rust · Open Source" }
                Action { text: "Projekt & Installation ↗"; command: "project" }
                Action { text: "Offizielle Proton CLI ↗"; command: "cli" }
                Copy { text: "Alpha: keine vollständige Windows/macOS-Funktionsparität. Versionsverwaltung und Proton Docs/Sheets sind nicht integriert. Das Proton SDK übernimmt Konto, Verschlüsselung und Übertragungen." }
            }
        }
    }

    property var request: ({})
    Connections {
        target: controller
        function onDialogRequested(value) {
            window.request=value
            if (value.kind==="folder") folderPicker.open()
            else if (value.kind==="files") { filePicker.fileMode=value.multiple ? FileDialog.OpenFiles : FileDialog.OpenFile; filePicker.open() }
            else { promptInput.text=value.initial || ""; modal.open(); cancelDialog.forceActiveFocus() }
        }
    }
    FolderDialog {
        id: folderPicker; objectName: "folderPicker"; title: window.request.title || "Ordner wählen"
        onAccepted: controller.answer_dialog(window.request.token,true,[selectedFolder.toString()])
        onRejected: controller.answer_dialog(window.request.token,false,[])
    }
    FileDialog {
        id: filePicker; objectName: "filePicker"; title: window.request.title || "Dateien wählen"
        onAccepted: controller.answer_dialog(window.request.token,true,selectedFiles.map(function(url) { return url.toString() }))
        onRejected: controller.answer_dialog(window.request.token,false,[])
    }
    Dialog {
        id: modal; objectName: "confirmationDialog"
        anchors.centerIn: parent; width: Math.min(560,window.width-60)
        modal: true; closePolicy: Popup.CloseOnEscape
        onRejected: controller.answer_dialog(window.request.token,false,"")
        background: Rectangle { color: "#182532"; radius: 16; border.color: "#496074" }
        header: Label { text: window.request.title || ""; textFormat: Text.PlainText; wrapMode: Text.Wrap; color: "#f0f4fa"; font.pixelSize: 20; font.bold: true; padding: 20 }
        contentItem: ColumnLayout {
            spacing: 16
            Copy { text: window.request.description || ""; color: "#d0dce9" }
            TextField { id: promptInput; objectName: "promptInput"; visible: window.request.kind==="prompt"; Layout.fillWidth: true; selectByMouse: true; color: "#e8edf5" }
            TextArea { visible: window.request.kind==="share" && Boolean(window.request.url); text: window.request.url || ""; textFormat: Text.PlainText; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true; color: "#91f1d1" }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action { id: cancelDialog; objectName: "dialogCancel"; text: "Abbrechen / Schließen"; enabled: true
                    onClicked: modal.reject()
                }
                Action { objectName: "dialogAccept"; text: window.request.kind==="prompt" ? "Übernehmen" : "Bestätigen"; enabled: true; primary: true; visible: window.request.kind!=="share"
                    onClicked: { let token=window.request.token; let value=promptInput.text; modal.close(); controller.answer_dialog(token,true,value) }
                }
                Action { text: "Link kopieren"; enabled: true; visible: window.request.kind==="share" && Boolean(window.request.url)
                    onClicked: { let token=window.request.token; modal.close(); controller.answer_dialog(token,true,"copy") }
                }
                Action { text: window.request.url ? "Link deaktivieren" : "Öffentlichen Leselink erstellen"; enabled: true; visible: window.request.kind==="share"
                    onClicked: { let token=window.request.token; let value=window.request.url ? "revoke" : "create"; modal.close(); controller.answer_dialog(token,true,value) }
                }
            }
        }
    }
}
