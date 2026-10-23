import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQuick.Dialogs

ApplicationWindow {
    id: window
    readonly property bool darkMode: controller.appearance.dark
    readonly property var theme: darkMode ? {
        "canvas": "#191822",
        "text": "#f1eef9",
        "accent": "#aa91ff",
        "sidebar": "#211f2d",
        "muted": "#b3adc4",
        "border": "#4a445c",
        "disabledText": "#817b91",
        "disabled": "#292635",
        "hover": "#373047",
        "primaryHover": "#bba7ff",
        "input": "#23202e",
        "cardBorder": "#393446",
        "accentText": "#c4b3ff",
        "secondaryText": "#c3bdd2",
        "selected": "#3a2e58",
        "navHover": "#2e293c",
        "badge": "#352a4d",
        "warning": "#4b3723",
        "rowSelected": "#3b3056",
        "rowHover": "#2a2537",
        "fileTile": "#282d3b",
        "fileInk": "#acbadc",
        "body": "#d1cbdf",
        "dialogBorder": "#514763",
        "onAccent": "#211631"
} : {
        "canvas": "#ffffff",
        "text": "#252236",
        "accent": "#6d4aff",
        "sidebar": "#f7f6fb",
        "muted": "#706b80",
        "border": "#dedbe7",
        "disabledText": "#9691a2",
        "disabled": "#f2f0f6",
        "hover": "#f2efff",
        "primaryHover": "#5935df",
        "input": "#faf9fc",
        "cardBorder": "#e7e4ed",
        "accentText": "#6140db",
        "secondaryText": "#575269",
        "selected": "#ebe5ff",
        "navHover": "#efedf5",
        "badge": "#f0ebff",
        "warning": "#fff1db",
        "rowSelected": "#eee8ff",
        "rowHover": "#f7f5fc",
        "fileTile": "#f1f3f8",
        "fileInk": "#6b7794",
        "body": "#403b51",
        "dialogBorder": "#d6d0e5",
        "onAccent": "#ffffff"
}
    objectName: "mainWindow"
    width: 1220; height: 820
    minimumWidth: 900; minimumHeight: 640
    visible: true
    title: "NeutronDrive" + (s.demo ? " · DEMO" : "")
    color: window.theme.canvas
    font.family: "Sans Serif"; font.pixelSize: 14
    palette.text: window.theme.text; palette.windowText: window.theme.text
    palette.buttonText: window.theme.text; palette.highlight: window.theme.accent
    palette.base: window.theme.canvas; palette.button: window.theme.canvas
    palette.window: window.theme.canvas; palette.highlightedText: window.theme.onAccent
    palette.light: window.theme.sidebar; palette.mid: window.theme.border; palette.dark: window.theme.muted
    property var s: controller.state
    property color muted: window.theme.muted
    property var titles: ({"files":s.fields.files_title.value,"background":window.tr("Automatischer Sync"),"ondemand":window.tr("Online-Laufwerk"),"local-trash":window.tr("Lokaler Papierkorb"),"activity":window.tr("Aktivität"),"settings":window.tr("Einstellungen")})
    function tr(source) { return controller.translate(source) }
    locale: Qt.locale(controller.language)
    function f(name) { return s.fields[name] }
    onClosing: function(event) { event.accepted = controller.request_close() }

    component Copy: Label {
        color: window.muted; wrapMode: Text.Wrap; textFormat: Text.PlainText
        Layout.fillWidth: true
    }
    component Heading: Label {
        color: window.theme.text; font.pixelSize: 18; font.bold: true; textFormat: Text.PlainText
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
            color: !actionButton.enabled ? window.theme.disabledText : actionButton.primary ? window.theme.onAccent : window.theme.text
            horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 9
            color: !actionButton.enabled ? window.theme.disabled : actionButton.primary ? (actionButton.hovered ? window.theme.primaryHover : window.theme.accent) : actionButton.hovered ? window.theme.hover : window.theme.canvas
            border.width: actionButton.activeFocus ? 2 : 1
            border.color: actionButton.primary || actionButton.activeFocus ? window.theme.accent : window.theme.border
            Behavior on color { ColorAnimation { duration: 100 } }
        }
    }
    component Input: TextField {
        id: input
        property string field
        text: window.f(field).value
        enabled: window.f(field).enabled && !s.busy
        selectByMouse: true
        color: window.theme.text; placeholderTextColor: window.muted
        Layout.fillWidth: true; implicitHeight: 44; padding: 12
        onTextEdited: controller.edit(field, text)
        background: Rectangle { color: window.theme.input; radius: 9; border.color: input.activeFocus ? window.theme.accent : window.theme.border }
    }
    component Card: Rectangle {
        color: window.theme.canvas; radius: 14; border.color: window.theme.cardBorder
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
        contentItem: Text { text: check.text; textFormat: Text.PlainText; color: window.theme.text; leftPadding: check.indicator.width + check.spacing; verticalAlignment: Text.AlignVCenter }
    }

    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 238; Layout.fillHeight: true; color: window.theme.sidebar
            ScrollView {
                id: sidebarScroll
                anchors.fill: parent; anchors.margins: 18; clip: true; contentWidth: availableWidth
            ColumnLayout {
                width: sidebarScroll.availableWidth; height: Math.max(implicitHeight,sidebarScroll.availableHeight); spacing: 8
                RowLayout {
                    Layout.topMargin: 10; spacing: 10
                    Rectangle { width: 34; height: 34; radius: 10; color: window.theme.accent
                        DriveIcon { anchors.centerIn: parent; symbol: "brand"; ink: window.theme.onAccent }
                    }
                    Label { text: "NeutronDrive"; font.pixelSize: 21; font.bold: true; color: window.theme.text }
                }
                Copy { text: window.tr("Deine Dateien. Dein Speicher."); font.pixelSize: 12; Layout.bottomMargin: 12 }
                Repeater {
                    model: [{name:window.tr("Meine Dateien"),key:"/my-files",icon:"folder"},{name:window.tr("Mit mir geteilt"),key:"/shared-with-me",icon:"shared"},{name:window.tr("Computer"),key:"/devices",icon:"computer"},{name:window.tr("Cloud-Papierkorb"),key:"/trash",icon:"trash"},
                        {name:window.tr("Automatischer Sync"),key:"background",icon:"refresh"},{name:window.tr("Online-Laufwerk"),key:"ondemand",icon:"cloud"},{name:window.tr("Lokaler Papierkorb"),key:"local-trash",icon:"trash"},{name:window.tr("Aktivität"),key:"activity",icon:"activity"},{name:window.tr("Einstellungen"),key:"settings",icon:"settings"}]
                    delegate: ColumnLayout {
                        id: navigationEntry
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true; spacing: 4
                        Copy {
                            visible: navigationEntry.index===0 || navigationEntry.index===4 || navigationEntry.index===7
                            text: navigationEntry.index===0 ? window.tr("DATEIEN") : navigationEntry.index===4 ? window.tr("AUF DIESEM GERÄT") : window.tr("VERWALTEN")
                            font.pixelSize: 10; font.letterSpacing: 1; font.bold: true
                            Layout.topMargin: navigationEntry.index===0 ? 0 : 12; Layout.leftMargin: 12; Layout.bottomMargin: 4
                        }
                        Button {
                        id: nav
                        property var modelData: navigationEntry.modelData
                        property bool selected: modelData.key.charAt(0)==="/" ? s.page==="files" && ("/"+s.path.split("/")[1])===modelData.key : s.page===modelData.key
                        objectName: "nav-"+modelData.key
                        Layout.fillWidth: true; implicitHeight: 42
                        text: modelData.name
                        onClicked: modelData.key.charAt(0)==="/" ? controller.navigate(modelData.name,modelData.key) : controller.show_page(modelData.key)
                        Accessible.name: text
                        contentItem: RowLayout {
                            spacing: 12
                            DriveIcon { symbol: nav.modelData.icon; ink: nav.selected ? window.theme.accentText : window.theme.muted; Layout.leftMargin: 10; Layout.preferredWidth: 20; Layout.preferredHeight: 20 }
                            Label { text: nav.text; elide: Text.ElideRight; color: nav.selected ? window.theme.accentText : window.theme.secondaryText; font.pixelSize: 13; font.bold: nav.selected; Layout.fillWidth: true }
                        }
                        background: Rectangle { radius: 9; color: nav.selected ? window.theme.selected : nav.hovered ? window.theme.navHover : "transparent"; border.color: nav.activeFocus ? window.theme.accent : "transparent" }
                        }
                    }
                }
                Item { Layout.fillHeight: true }
                Action { text: window.tr("Bei Proton anmelden"); command: "login"; Layout.fillWidth: true }
                Action { text: window.tr("Proton Drive im Web ↗"); command: "web"; Layout.fillWidth: true }
                Copy { text: window.tr("Unabhängige Open-Source-App\n")+s.version+(s.demo ? " · DEMO" : ""); font.pixelSize: 11; Layout.topMargin: 10 }
            }
        }
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.margins: 28; spacing: 16
            RowLayout {
                Layout.fillWidth: true
                ColumnLayout {
                    Layout.fillWidth: true; spacing: 5
                    Copy { text: s.page==="files" ? window.tr("Dateien & Ordner") : window.tr("NeutronDrive für Linux"); font.pixelSize: 12 }
                    Label { text: window.titles[s.page]; textFormat: Text.PlainText; font.pixelSize: 28; font.bold: true; color: window.theme.text; elide: Text.ElideRight; Layout.fillWidth: true }
                }
                Rectangle { implicitWidth: modeLabel.implicitWidth+24; implicitHeight: 30; radius: 15; color: window.theme.badge
                    Label { id: modeLabel; anchors.centerIn: parent; text: s.demo ? "Demo" : "Proton SDK"; color: window.theme.accentText; font.pixelSize: 12 }
                }
            }
            Rectangle {
                visible: window.f("notice_label").visible; Layout.fillWidth: true; implicitHeight: notice.implicitHeight+24
                radius: 10; color: s.noticeError ? window.theme.warning : window.theme.badge
                Copy { id: notice; anchors { left: parent.left; right: parent.right; margins: 12; verticalCenter: parent.verticalCenter } text: window.f("notice_label").value; color: window.theme.text }
            }
            Action { visible: window.f("login_button").visible; text: window.tr("Anmeldung im Browser öffnen"); command: "openLogin" }
            Loader { id: pageLoader; objectName: "pageLoader"; Layout.fillWidth: true; Layout.fillHeight: true
                sourceComponent: s.page==="files" ? filesPage : s.page==="background" ? backgroundPage : s.page==="ondemand" ? mountPage : s.page==="local-trash" ? trashPage : s.page==="activity" ? activityPage : settingsPage
            }
            RowLayout {
                Layout.fillWidth: true
                BusyIndicator { running: s.busy; visible: s.busy; implicitWidth: 24; implicitHeight: 24 }
                Copy { text: window.f("status_label").value; font.pixelSize: 12 }
                Action { visible: s.busy; text: window.tr("Abbrechen"); command: "cancel"; enabled: true }
            }
        }
    }

    Component {
        id: filesPage
        ColumnLayout {
            objectName: "page-files"; spacing: 14
            RowLayout {
                Layout.fillWidth: true; spacing: 12
                Input { field: "search"; placeholderText: window.tr("In diesem Ordner suchen …"); objectName: "searchInput"; leftPadding: 42
                    DriveIcon { x: 13; anchors.verticalCenter: parent.verticalCenter; symbol: "search"; ink: window.muted; width: 18; height: 18 }
                }
            }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action { text: window.tr("Dateien hochladen"); command: "upload"; field: "upload_button"; primary: true }
                Action { text: window.tr("Ordner hochladen"); command: "uploadFolder"; field: "folder_upload_button" }
                Action { text: window.tr("Neuer Ordner"); command: "mkdir"; field: "mkdir_button" }
                Action { text: window.tr("Aktualisieren"); command: "refresh"; field: "refresh_button" }
            }
            Flow { Layout.fillWidth: true; spacing: 4
                Repeater { model: s.crumbs; delegate: Button { required property string modelData; required property int index; text: modelData+"  ›"; flat: true; onClicked: controller.crumb(index) } }
            }
            Rectangle {
                Layout.fillHeight: true; Layout.fillWidth: true; radius: 12; color: window.theme.canvas; border.color: window.theme.cardBorder
                ColumnLayout {
                    anchors.fill: parent; anchors.margins: 12; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; Layout.margins: 10
                        Copy { text: window.tr("Name"); font.pixelSize: 12 }
                        Label { text: window.tr("Größe"); color: window.muted; font.pixelSize: 10; Layout.preferredWidth: 80 }
                        Label { text: window.tr("Geändert"); color: window.muted; font.pixelSize: 10; Layout.preferredWidth: 90 }
                    }
                    ListView {
                        id: fileList; objectName: "fileList"; Layout.fillWidth: true; Layout.fillHeight: true
                        clip: true; model: controller.fileEntries; spacing: 4;
                        activeFocusOnTab: true; currentIndex: s.focusIndex; keyNavigationEnabled: false
                        Keys.onPressed: function(event) {
                            if (event.matches(StandardKey.SelectAll)) { controller.select_all(); event.accepted=true }
                            else if (event.key===Qt.Key_Down || event.key===Qt.Key_Up) { controller.move_selection(event.key===Qt.Key_Down ? 1 : -1,Boolean(event.modifiers & Qt.ShiftModifier)); event.accepted=true }
                            else if (event.key===Qt.Key_Return || event.key===Qt.Key_Enter) { controller.row_activated(s.focusIndex); event.accepted=true }
                            else if (event.key===Qt.Key_Space) { controller.select_row(s.focusIndex,true); event.accepted=true }
                        }
                         boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Rectangle {
                            id: fileRow
                            required property var modelData; required property int index
                            width: fileList.width; height: 64; radius: 8
                            objectName: "fileRow-"+index
                            color: s.selection.indexOf(index)>=0 ? window.theme.rowSelected : rowMouse.containsMouse ? window.theme.rowHover : "transparent"
                            border.color: fileList.activeFocus && index===s.focusIndex ? window.theme.accent : "transparent"
                            Accessible.role: Accessible.ListItem; Accessible.name: modelData.name; Accessible.selected: s.selection.indexOf(index)>=0
                            Keys.onSpacePressed: controller.select_row(index,true)
                            Keys.onReturnPressed: controller.row_activated(index)
                            RowLayout {
                                anchors.fill: parent; anchors.margins: 10; spacing: 12
                                Rectangle {
                                    Layout.preferredWidth: 36; Layout.preferredHeight: 36; radius: 9
                                    color: fileRow.modelData.isFolder ? window.theme.rowSelected : window.theme.fileTile
                                    DriveIcon { anchors.centerIn: parent; symbol: fileRow.modelData.isFolder ? "folder" : "file"; ink: fileRow.modelData.isFolder ? window.theme.accent : window.theme.fileInk }
                                }
                                ColumnLayout { Layout.fillWidth: true; spacing: 3
                                    Label { text: fileRow.modelData.name; textFormat: Text.PlainText; color: window.theme.text; elide: Text.ElideRight; Layout.fillWidth: true }
                                    Copy { text: fileRow.modelData.safe ? fileRow.modelData.kind : window.tr("Nicht eindeutig · gesperrt"); font.pixelSize: 11 }
                                }
                                Label { text: fileRow.modelData.size; color: window.muted; Layout.preferredWidth: 80; font.pixelSize: 12 }
                                Label { text: fileRow.modelData.modified; color: window.muted; Layout.preferredWidth: 90; font.pixelSize: 12 }
                            }
                            MouseArea { id: rowMouse; anchors.fill: parent; hoverEnabled: true; enabled: !s.busy
                                onClicked: function(mouse) { fileList.forceActiveFocus(); controller.select_row(fileRow.index,Boolean(mouse.modifiers & (Qt.ControlModifier|Qt.ShiftModifier))) }
                                onDoubleClicked: controller.row_activated(fileRow.index)
                            }
                        }
                        Label { anchors.centerIn: parent; visible: fileList.count===0; text: window.f("empty_label").value; color: window.muted }
                    }
                }
            }
            Flow { Layout.fillWidth: true; spacing: 8
                Repeater { model: ["Download","Umbenennen","Verschieben","Freigabe","Papierkorb","Wiederherstellen"]
                    delegate: Action { required property string modelData; text: window.tr(modelData); command: modelData; enabled: s.actions[modelData].enabled; visible: s.actions[modelData].visible }
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
        id: backgroundPage
        PageScroll {
            objectName: "page-background"
            Copy { text: window.tr("Änderungen automatisch abgleichen – auch nach dem Schließen des Fensters.") }
            Card {
                Heading { text: window.tr("Computersicherung") }
                Copy { text: window.tr("Sichere ausgewählte Ordner unter Computer in Proton Drive. Die Ordner werden automatisch in beide Richtungen abgeglichen.") }
                Copy { text: window.f("background_local_label").value }
                Action { text: window.tr("Lokalen Ordner wählen"); command: "chooseBackground" }
                Action { objectName: "computerBackupButton"; text: window.tr("Ordner zur Computersicherung hinzufügen"); command: "addComputerFolder"; field: "background_add_button"; primary: true }
            }
            Card {
                Heading { text: window.tr("Ordner automatisch synchronisieren") }
                Copy { text: window.f("background_local_label").value }
                Action { text: window.tr("Lokalen Ordner wählen"); command: "chooseBackground" }
                Copy { text: window.tr("Cloud-Ordner") }
                Input { field: "background_remote" }
                Action { text: window.tr("Automatischen Sync aktivieren"); command: "addJob"; field: "background_add_button"; primary: true }
            }
            Card {
                Heading { text: window.f("background_status").value }
                RowLayout { Action { text: window.tr("Dienst starten"); command: "start" } Action { text: window.tr("Dienst stoppen"); command: "stop" } }
            }
            Repeater { model: s.jobs
                delegate: Card { required property var modelData
                    Heading { text: modelData.name }
                    Copy { text: modelData.detail }
                    Copy { text: modelData.status }
                    Flow { Layout.fillWidth: true; spacing: 8
                        Action { text: modelData.paused ? window.tr("Fortsetzen") : window.tr("Pausieren"); onClicked: controller.job_action(modelData.paused ? "resume" : "pause",modelData.id) }
                        Action { text: window.tr("Jetzt abgleichen"); onClicked: controller.job_action("sync",modelData.id) }
                        Action { text: window.tr("Auftrag entfernen"); onClicked: controller.job_action("remove",modelData.id) }
                    }
                }
            }
            Copy { text: window.tr("Dateilöschungen nur bei unverändertem Gegenstück; ganze Ordnerlöschungen bleiben Konflikte. Anmeldung und entsperrter Schlüsselbund erforderlich.") }
        }
    }
    Component {
        id: mountPage
        PageScroll {
            objectName: "page-ondemand"
            Copy { text: window.tr("Dateien beim Öffnen laden, offline vorhalten oder lokalen Speicher freigeben.") }
            Card {
                Heading { text: window.tr("Online-Laufwerk verbinden") }
                Copy { text: window.f("mount_label").value }
                Action { text: window.tr("Leeren Einhängeordner wählen"); command: "chooseMount" }
                Copy { text: window.tr("Cloud-Ordner") }
                Input { field: "mount_remote" }
                Toggle { field: "mount_writable"; text: window.tr("Dateien im Laufwerk bearbeiten und hochladen") }
                Toggle { field: "mount_offline"; text: window.tr("Offline aus vorhandenem Cache öffnen") }
                Flow { Layout.fillWidth: true; spacing: 8
                    Action { text: window.tr("Laufwerk verbinden"); command: "mount"; field: "mount_button"; primary: true }
                    Action { text: window.tr("Trennen"); command: "unmount" }
                    Action { text: window.tr("Aktualisieren"); command: "refreshMount" }
                }
                Copy { text: window.f("mount_status").value }
            }
            Card {
                Heading { text: window.tr("Offline-Verfügbarkeit") }
                Input { field: "cache_path"; placeholderText: window.tr("Dateipfad, z. B. Dokumente/Notizen.txt") }
                RowLayout { Action { text: window.tr("Offline vorhalten"); command: "pin" } Action { text: window.tr("Speicher freigeben"); command: "evict" } }
                Action { text: window.tr("Laufwerkshelfer bei Proton anmelden"); command: "helperLogin" }
            }
            Copy { text: window.tr("Für Flatpak zusätzlich den nativen Laufwerkshelfer installieren. Online-Laufwerke sind im Snap derzeit nicht verfügbar. Nicht hochgeladene Änderungen bleiben im Wiederherstellungsspeicher.") }
        }
    }
    Component {
        id: trashPage
        PageScroll {
            objectName: "page-local-trash"
            Copy { text: window.tr("Nach Cloud-Löschung lokal gesicherte Dateien. Wiederherstellen überschreibt keine vorhandenen Dateien.") }
            Action { text: window.tr("Aktualisieren"); command: "refreshTrash" }
            Copy { text: window.f("local_trash_status").value }
            Copy { text: window.f("local_trash_empty").value; visible: s.trash.length===0 }
            Repeater { model: s.trash
                delegate: Card { required property var modelData; required property int index
                    Heading { text: modelData.name }
                    Copy { text: modelData.detail }
                    Copy { text: modelData.error+" "+modelData.restorePath }
                    Action { text: window.tr("Wiederherstellen / Wiederherstellung prüfen"); enabled: modelData.enabled && !s.busy; onClicked: controller.restore_entry(index) }
                }
            }
            Copy { text: window.tr("Bei aktivem Sync wird eine Wiederherstellung erneut synchronisiert. NeutronDrive leert den Papierkorb nicht automatisch.") }
        }
    }
    Component {
        id: activityPage
        PageScroll {
            objectName: "page-activity"
            Copy { text: window.tr("Vorgänge dieser Sitzung · keine dauerhafte Speicherung von Dateinamen") }
            Copy { visible: s.activity.length===0; text: window.tr("Noch keine Vorgänge in dieser Sitzung.") }
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
                Heading { text: window.tr("Konto & Verbindung") }
                Copy { text: window.f("backend_label").value }
                RowLayout { Action { text: window.tr("Backend prüfen"); command: "version" } Action { text: window.tr("Anmelden"); command: "login" } Action { text: window.tr("Abmelden"); command: "logout" } }
                Copy { text: window.tr("Anmeldung ausschließlich auf account.proton.me im Browser. Die Sitzung liegt im Schlüsselbund oder in pass. Flatpak kann einen eigenen Kontospeicher verwenden.") }
            }
            Card {
                Heading { text: window.tr("Erscheinungsbild") }
                Copy { text: window.tr("Erscheinungsbild") }
                ComboBox {
                    objectName: "appearanceSelector"
                    Layout.fillWidth: true
                    model: [window.tr("Systemeinstellung"), window.tr("Hell"), window.tr("Dunkel")]
                    currentIndex: ["system", "light", "dark"].indexOf(controller.appearance.mode)
                    onActivated: controller.setAppearancePreference(["system", "light", "dark"][currentIndex])
                    Accessible.name: window.tr("Erscheinungsbild")
                }
                Copy { text: window.tr("Sprache") }
                ComboBox {
                    objectName: "languageSelector"
                    Layout.fillWidth: true
                    model: controller.languageOptions
                    textRole: "name"
                    valueRole: "code"
                    currentIndex: {
                        for (var i = 0; i < model.length; ++i)
                            if (model[i].code === controller.languagePreference) return i
                        return 0
                    }
                    onActivated: controller.setLanguagePreference(currentValue)
                    Accessible.name: window.tr("Sprache")
                }
                Copy { text: window.tr("Die Sprache wird beim nächsten App-Start übernommen.") }
            }
            Card {
                Heading { text: window.tr("Beim Anmelden starten") }
                Copy { text: window.tr("Halte deine automatischen Sync-Aufträge nach dem Desktop-Start bereit.") }
                RowLayout { Action { text: window.tr("Autostart aktivieren"); command: "autostart" } Action { text: window.tr("Deaktivieren"); command: "noAutostart" } }
            }
            Card {
                Heading { text: window.tr("Updates") }
                CheckBox {
                    text: window.tr("Automatisch täglich nach Updates suchen")
                    checked: controller.updates.automatic
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("automatic", checked)
                }
                CheckBox {
                    text: window.tr("Vorabversionen einschließen")
                    checked: controller.updates.previews
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("previews", checked)
                }
                CheckBox {
                    text: window.tr("Updates automatisch herunterladen")
                    visible: controller.updates.packageFormat === "deb"
                    checked: controller.updates.autoDownload
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("download", checked)
                }
                Copy { text: controller.updates.status }
                Copy { text: window.tr("Die Prüfung kontaktiert GitHub. Es werden keine Konto- oder Dateidaten übertragen.") }
                Copy {
                    visible: controller.updates.packageFormat !== "deb"
                    text: window.tr("Dieses Installationsformat wird über den Paketmanager oder das Paket auf der Release-Seite aktualisiert.")
                }
                Flow {
                    Layout.fillWidth: true; spacing: 8
                    Action { text: window.tr("Nach Updates suchen"); enabled: !controller.updates.busy && !s.demo; onClicked: controller.updates.checkNow() }
                    Action { text: window.tr("Update herunterladen"); visible: controller.updates.downloadable && !controller.updates.ready; enabled: !controller.updates.busy; onClicked: controller.updates.downloadUpdate() }
                    Action { text: window.tr("Update installieren"); visible: controller.updates.ready; enabled: !controller.updates.busy && !s.busy; onClicked: controller.updates.installUpdate() }
                    Action { text: window.tr("Release-Seite öffnen"); visible: controller.updates.available; onClicked: controller.updates.openRelease() }
                }
                Copy { visible: controller.updates.packageFormat === "deb"; text: window.tr("Die Installation benötigt eine Systembestätigung. Laufende Synchronisationen vor dem Neustart abschließen.") }
            }
            Card {
                Heading { text: "NeutronDrive "+s.version }
                Copy { text: "Qt Quick · QML · Rust · Open Source" }
                Action { text: window.tr("Projekt & Installation ↗"); command: "project" }
                Action { text: window.tr("Offizielle Proton CLI ↗"); command: "cli" }
                Copy { text: window.tr("Alpha: keine vollständige Windows/macOS-Funktionsparität. Versionsverwaltung und Proton Docs/Sheets sind nicht integriert. Das Proton SDK übernimmt Konto, Verschlüsselung und Übertragungen.") }
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
        id: folderPicker; objectName: "folderPicker"; title: window.request.title || window.tr("Ordner wählen")
        onAccepted: controller.answer_dialog(window.request.token,true,[selectedFolder.toString()])
        onRejected: controller.answer_dialog(window.request.token,false,[])
    }
    FileDialog {
        id: filePicker; objectName: "filePicker"; title: window.request.title || window.tr("Dateien wählen")
        onAccepted: controller.answer_dialog(window.request.token,true,selectedFiles.map(function(url) { return url.toString() }))
        onRejected: controller.answer_dialog(window.request.token,false,[])
    }
    Dialog {
        id: modal; objectName: "confirmationDialog"
        anchors.centerIn: parent; width: Math.min(560,window.width-60)
        modal: true; closePolicy: Popup.CloseOnEscape
        onRejected: controller.answer_dialog(window.request.token,false,"")
        background: Rectangle { color: window.theme.canvas; radius: 16; border.color: window.theme.dialogBorder }
        header: Label { text: window.request.title || ""; textFormat: Text.PlainText; wrapMode: Text.Wrap; color: window.theme.text; font.pixelSize: 20; font.bold: true; padding: 20 }
        contentItem: ColumnLayout {
            spacing: 16
            Copy { text: window.request.description || ""; color: window.theme.secondaryText }
            TextField { id: promptInput; objectName: "promptInput"; visible: window.request.kind==="prompt"; Layout.fillWidth: true; selectByMouse: true; color: window.theme.text }
            TextArea { visible: window.request.kind==="share" && Boolean(window.request.url); text: window.request.url || ""; textFormat: Text.PlainText; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true; color: window.theme.accentText }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action { id: cancelDialog; objectName: "dialogCancel"; text: window.tr("Abbrechen / Schließen"); enabled: true
                    onClicked: modal.reject()
                }
                Action { objectName: "dialogAccept"; text: window.request.kind==="prompt" ? window.tr("Übernehmen") : window.tr("Bestätigen"); enabled: true; primary: true; visible: window.request.kind!=="share"
                    onClicked: { let token=window.request.token; let value=promptInput.text; modal.close(); controller.answer_dialog(token,true,value) }
                }
                Action { objectName: "copyShareLink"; text: window.tr("Link kopieren"); enabled: true; visible: window.request.kind==="share" && Boolean(window.request.url)
                    onClicked: { let token=window.request.token; modal.close(); controller.answer_dialog(token,true,"copy") }
                }
                Action { text: window.request.url ? window.tr("Link deaktivieren") : window.tr("Öffentlichen Leselink erstellen"); enabled: true; visible: window.request.kind==="share"
                    onClicked: { let token=window.request.token; let value=window.request.url ? "revoke" : "create"; modal.close(); controller.answer_dialog(token,true,value) }
                }
            }
        }
    }
}
