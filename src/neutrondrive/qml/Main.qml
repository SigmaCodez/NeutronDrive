import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQuick.Dialogs

ApplicationWindow {
    id: window
    readonly property bool darkMode: controller.appearance.dark
    readonly property var theme: darkMode ? {
        "canvas": "#191b24",
        "text": "#f1eef9",
        "accent": "#aa91ff",
        "sidebar": "#12141c",
        "muted": "#b0b6ca",
        "border": "#44495b",
        "disabledText": "#817b91",
        "disabled": "#292635",
        "hover": "#2c3040",
        "primaryHover": "#bba7ff",
        "input": "#20232e",
        "cardBorder": "#303442",
        "accentText": "#c4b3ff",
        "secondaryText": "#c3bdd2",
        "selected": "#3a2e58",
        "navHover": "#252936",
        "badge": "#352a4d",
        "warning": "#4b3723",
        "rowSelected": "#3b3056",
        "rowHover": "#232735",
        "fileTile": "#282d3b",
        "fileInk": "#acbadc",
        "body": "#d1cbdf",
        "dialogBorder": "#514763",
        "onAccent": "#211631"
} : {
        "canvas": "#ffffff",
        "text": "#252236",
        "accent": "#6d4aff",
        "sidebar": "#f4f5fa",
        "muted": "#657087",
        "border": "#d7dce8",
        "disabledText": "#9691a2",
        "disabled": "#f2f0f6",
        "hover": "#f2efff",
        "primaryHover": "#5935df",
        "input": "#f7f8fc",
        "cardBorder": "#e3e6ef",
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
    color: window.theme.sidebar
    font.family: "Sans Serif"; font.pixelSize: 14
    palette.text: window.theme.text; palette.windowText: window.theme.text
    palette.buttonText: window.theme.text; palette.highlight: window.theme.accent
    palette.base: window.theme.canvas; palette.button: window.theme.canvas
    palette.toolTipBase: window.theme.sidebar; palette.toolTipText: window.theme.text
    palette.placeholderText: window.theme.muted
    palette.window: window.theme.canvas; palette.highlightedText: window.theme.onAccent
    palette.light: window.theme.sidebar; palette.mid: window.theme.border; palette.dark: window.theme.muted
    readonly property bool confirmationOpen: modal.visible
    readonly property var sharingWindow: shareWindow
    function openShareLoading() { shareWindow.loading(); }
    readonly property bool dialogOpen: shareWindow.visible || modal.visible || folderPicker.visible || filePicker.visible || setupWizard.visible
    property bool gridView: false
    Timer { interval: s.storage ? 60000 : 5000; repeat: true; running: Boolean(s.authenticated); triggeredOnStart: true; onTriggered: if(!s.busy)controller.send("account_storage",[]) }
    property var appController: controller
    property var s: controller.state
    property color muted: window.theme.muted
    property var titles: ({"files":s.fields.files_title.value,"photos":window.tr("Photos"),"computer-backup":window.tr("Computersicherung"),"ondemand":window.tr("Online-Laufwerk"),"local-trash":window.tr("Lokaler Papierkorb"),"activity":window.tr("Aktivität"),"settings":window.tr("Einstellungen")})
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
    component Action: DriveButton {
        theme: window.theme
        property string command
        property string field: ""
        enabled: field ? window.f(field).enabled : !s.busy
        onClicked: controller.action(command)
    }
    component Field: DriveField { theme: window.theme }
    component Choice: DriveCombo { theme: window.theme }
    component Check: DriveCheck { theme: window.theme }
    component ViewToggle: Button {
        id: viewToggle
        checkable: true; implicitHeight: 40; padding: 12
        Accessible.name: text
        contentItem: Text { text: viewToggle.text; font: viewToggle.font; color: viewToggle.checked ? window.theme.accentText : window.theme.secondaryText; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
        background: Rectangle { radius: 8; color: viewToggle.checked ? window.theme.selected : viewToggle.hovered ? window.theme.navHover : "transparent"; border.color: viewToggle.activeFocus ? window.theme.accent : "transparent" }
    }
    component Input: Field {
        property string field
        text: window.f(field).value
        enabled: window.f(field).enabled && (!s.busy || (field==="search" && Boolean(s.reading)))
        Layout.fillWidth: true
        onTextEdited: controller.edit(field, text)
    }
    component Card: Rectangle {
        color: window.theme.canvas; radius: 14; border.color: window.theme.cardBorder
        Layout.fillWidth: true
        implicitHeight: cardContents.implicitHeight + 40
        default property alias contents: cardContents.data
        ColumnLayout { id: cardContents; anchors { left: parent.left; right: parent.right; top: parent.top; margins: 20 } spacing: 12 }
    }
    component Toggle: Check {
        property string field
        checked: window.f(field).value
        enabled: !s.busy
        onToggled: controller.edit(field, checked)
    }

    Dialog {
        id: setupWizard; objectName: "setupWizard"
        property int step: 0
        visible: Boolean(s.setupNeeded); modal: true; closePolicy: Popup.NoAutoClose
        onOpened: step = 0
        anchors.centerIn: parent; width: Math.min(580, window.width - 48)
        height: Math.min(implicitHeight, window.height - 48)
        padding: 24
        background: Rectangle { radius: 18; color: window.theme.canvas; border.color: window.theme.cardBorder }
        header: Item { implicitHeight: 0 }
        Overlay.modal: Rectangle { color: window.theme.canvas }
        contentItem: ColumnLayout {
            spacing: 22
            ScrollView {
                id: setupScroll; objectName: "setupScroll"
                Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 0
                implicitHeight: setupBody.implicitHeight
                contentWidth: availableWidth; clip: true
                ScrollBar.vertical: DriveScrollBar { theme: window.theme }
                ColumnLayout {
                    id: setupBody; width: setupScroll.availableWidth; spacing: 22
            ColumnLayout {
                Layout.fillWidth: true; spacing: 16
                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: 56; Layout.preferredHeight: 56; radius: 16; color: window.theme.accent
                    DriveIcon { anchors.centerIn: parent; symbol: "brand"; ink: window.theme.onAccent }
                }
                ColumnLayout {
                    spacing: 3
                    Heading { text: window.tr("Welcome to NeutronDrive"); font.pixelSize: 26; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter }
                    Copy { text: window.tr("Your files, ready when you need them"); horizontalAlignment: Text.AlignHCenter }
                }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 12
                Repeater {
                    model: [window.tr("Sign in to Proton"), window.tr("Choose sync folder")]
                    delegate: Rectangle {
                        required property int index
                        required property string modelData
                        Layout.fillWidth: true; implicitHeight: steps.implicitHeight + 24
                        radius: 10; color: setupWizard.step === index ? window.theme.selected : window.theme.sidebar
                        RowLayout {
                            id: steps
                            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 12 }
                            spacing: 8
                            Label { text: (index + 1).toString(); font.bold: true; color: setupWizard.step === index ? window.theme.accentText : window.theme.muted }
                            Label { text: modelData; Layout.fillWidth: true; wrapMode: Text.Wrap; font.bold: setupWizard.step === index; color: setupWizard.step === index ? window.theme.accentText : window.theme.muted }
                        }
                    }
                }
            }
            Card {
                visible: setupWizard.step === 0
                color: window.theme.sidebar
                Copy { text: window.tr("Die Anmeldung erfolgt bei Proton im Browser. Diese App ist unabhängig und wird nicht von Proton unterstützt.") }
                Action {
                    objectName: "setupSignIn"; Layout.fillWidth: true
                    text: s.authenticated ? window.tr("Verbunden") : window.tr("Sign in to Proton")
                    command: "login"; primary: !s.authenticated; enabled: !s.busy && !s.authenticated
                }
            }
            ColumnLayout {
                visible: setupWizard.step === 1; Layout.fillWidth: true; spacing: 16
                Card {
                    color: window.theme.sidebar
                    Heading { text: window.tr("Choose sync folder") }
                    Field {
                        id: setupPath; objectName: "setupPath"
                        Layout.fillWidth: true; implicitHeight: 44; padding: 12
                        text: s.setupFolder || ""; enabled: !s.busy; selectByMouse: true
                        color: window.theme.text; selectionColor: window.theme.accent; selectedTextColor: window.theme.onAccent
                        Accessible.name: window.tr("Choose sync folder")
                        onTextEdited: controller.send("setup_folder", [text])
                        background: Rectangle { radius: 9; color: window.theme.input; border.color: setupPath.activeFocus ? window.theme.accent : window.theme.border }
                    }
                    Action { text: window.tr("Choose sync folder"); onClicked: setupFolderPicker.open() }
                }
                Copy { text: s.sandboxed ? window.tr("This store edition downloads all files from My files into the selected empty folder and synchronizes changes in both directions. Make sure enough disk space is available. Online placeholders and file-manager extensions are not available in this edition.") : window.tr("Finishing starts the sync service and enables automatic startup at login. Existing files in the selected folder will never be hidden or moved.") }
                Copy { visible: Boolean(s.sandboxed); text: window.tr("Finishing enables background sync and automatic startup at login. You can configure individual sync folders in Settings instead by choosing Later.") }
            }
            Copy { text: s.fields.notice_label.value || ""; visible: Boolean(text); color: window.theme.accentText }
                }
            }
            Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: window.theme.cardBorder }
            GridLayout {
                columns: 2; columnSpacing: 10; rowSpacing: 10
                Action { Layout.fillWidth: true; Layout.minimumWidth: 0; text: window.tr("Later"); onClicked: controller.send("setup_later", []) }
                Action { Layout.fillWidth: true; Layout.minimumWidth: 0; objectName: "setupBack"; visible: setupWizard.step === 1; text: window.tr("Previous"); onClicked: setupWizard.step = 0 }
                Action {
                    objectName: "setupNext"; Layout.fillWidth: true; Layout.minimumWidth: 0
                    visible: setupWizard.step === 0; text: window.tr("Next"); primary: true
                    enabled: !s.busy && Boolean(s.authenticated)
                    onClicked: setupWizard.step = 1
                }
                Action {
                    objectName: "setupFinish"; Layout.columnSpan: 2; Layout.fillWidth: true; Layout.minimumWidth: 0
                    visible: setupWizard.step === 1; text: window.tr("Finish and start sync"); primary: true
                    enabled: !s.busy && Boolean(s.authenticated) && setupPath.text.trim().length > 0
                    onClicked: { controller.send("setup_folder", [setupPath.text]); controller.send("setup_finish", []); }
                }
            }
        }
    }
    FolderDialog {
        id: setupFolderPicker; title: window.tr("Choose an empty sync folder")
        onAccepted: { let url=selectedFolder.toString(); controller.send("setup_folder_url",[url]); }
    }

    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 248; Layout.fillHeight: true; color: window.theme.sidebar
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 18; spacing: 8
                RowLayout {
                    Layout.topMargin: 10; spacing: 10
                    Rectangle { width: 34; height: 34; radius: 10; color: window.theme.accent
                        DriveIcon { anchors.centerIn: parent; symbol: "brand"; ink: window.theme.onAccent }
                    }
                    Label { text: "NeutronDrive"; font.pixelSize: 21; font.bold: true; color: window.theme.text }
                }
                Item { Layout.preferredHeight: 16 }
                ScrollView {
                    id: sidebarScroll
                    Layout.fillWidth: true; Layout.fillHeight: true
                    clip: true; contentWidth: availableWidth
                    ColumnLayout {
                        width: sidebarScroll.availableWidth; spacing: 4
                Repeater {
                    model: [{name:window.tr("Meine Dateien"),key:"/my-files",icon:"folder"},{name:window.tr("Photos"),key:"photos",icon:"photo"},{name:window.tr("Mit mir geteilt"),key:"/shared-with-me",icon:"shared"},{name:window.tr("Shared by me"),key:"/shared-by-me",icon:"shared"},{name:window.tr("Computer"),key:"/devices",icon:"computer"},{name:window.tr("Cloud-Papierkorb"),key:"/trash",icon:"trash"},
                        {name:window.tr("Aktivität"),key:"activity",icon:"activity"},{name:window.tr("Einstellungen"),key:"settings",icon:"settings"}]
                    delegate: ColumnLayout {
                        id: navigationEntry
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true; spacing: 4
                        Button {
                        id: nav
                        property var modelData: navigationEntry.modelData
                        property bool selected: modelData.key.charAt(0)==="/" ? (s.page==="files" && ("/"+s.path.split("/")[1])===modelData.key || s.page==="computer-backup" && modelData.key==="/devices" || s.page==="local-trash" && modelData.key==="/trash") : s.page===modelData.key
                        objectName: "nav-"+modelData.key
                        Layout.fillWidth: true; implicitHeight: 42
                        text: modelData.name
                        onClicked: controller.send("open_section",[modelData.name,modelData.key])
                        Accessible.name: text
                        contentItem: RowLayout {
                            spacing: 12
                            DriveIcon { symbol: nav.modelData.icon; ink: nav.selected ? window.theme.accentText : window.theme.muted; Layout.leftMargin: 10; Layout.preferredWidth: 20; Layout.preferredHeight: 20 }
                            Label { text: nav.text; elide: Text.ElideRight; color: nav.selected ? window.theme.accentText : window.theme.secondaryText; font.pixelSize: 14; font.bold: nav.selected; Layout.fillWidth: true }
                        }
                        background: Rectangle { radius: 9; color: nav.selected ? window.theme.selected : nav.hovered ? window.theme.navHover : "transparent"; border.color: nav.activeFocus ? window.theme.accent : "transparent" }
                        }
                    }
                }
                    }
                }
                Action { text: window.tr("Bei Proton anmelden"); command: "login"; visible: !s.authenticated; Layout.fillWidth: true }
                ColumnLayout {
                    visible: Boolean(s.authenticated); Layout.fillWidth: true
                    RowLayout {
                        Layout.fillWidth: true
                        Copy { text: window.tr("Account storage"); font.bold: true }
                        Action { text: "↻"; flat: true; padding: 4; font.pixelSize: 20; implicitWidth: 30; implicitHeight: 30; enabled: !s.busy; onClicked: controller.send("account_storage",[]); Accessible.name: window.tr("Refresh storage"); ToolTip.visible: hovered; ToolTip.text: window.tr("Refresh storage") }
                    }
                    ProgressBar {
                        id: storageProgress
                        Layout.fillWidth: true; from: 0; to: 100; value: s.storage ? Math.min(100,s.storage.percent) : 0; visible: Boolean(s.storage)
                        background: Rectangle { implicitHeight: 6; radius: 3; color: window.theme.border }
                        contentItem: Item { implicitHeight: 6; Rectangle { width: storageProgress.visualPosition*parent.width; height: parent.height; radius: 3; color: window.theme.accent } }
                    }
                    Copy { text: s.storage ? s.storage.usedText+" "+window.tr("used")+" · "+Math.round(s.storage.percent)+"%" : window.tr("Storage information unavailable"); font.pixelSize: 12 }
                    Copy { text: s.storage ? s.storage.freeText+" "+window.tr("free") : ""; visible: Boolean(s.storage); font.pixelSize: 12 }
                }
                Button {
                    id: accountButton; objectName: "accountMenuButton"
                    property alias popup: accountMenu
                    visible: Boolean(s.authenticated); Layout.fillWidth: true; implicitHeight: 58; padding: 10
                    readonly property string accountName: (s.account && (s.account.name || s.account.email)) || window.tr("Signed in")
                    Accessible.name: window.tr("Account menu")+": "+accountName
                    onClicked: accountMenu.open()
                    background: Rectangle { radius: 10; color: accountButton.hovered || accountMenu.opened ? window.theme.navHover : "transparent"; border.color: accountButton.activeFocus ? window.theme.accent : window.theme.border }
                    contentItem: RowLayout {
                        spacing: 9
                        Rectangle {
                            Layout.preferredWidth: 32; Layout.preferredHeight: 32; radius: 16; color: window.theme.selected
                            Label { anchors.centerIn: parent; text: accountButton.accountName.slice(0,1).toUpperCase(); textFormat: Text.PlainText; font.bold: true; color: window.theme.accentText }
                        }
                        ColumnLayout {
                            Layout.fillWidth: true; spacing: 2
                            Label { objectName: "accountDisplayName"; text: accountButton.accountName; textFormat: Text.PlainText; elide: Text.ElideRight; color: window.theme.text; font.bold: true; Layout.fillWidth: true }
                            Label { text: (s.account && s.account.email) || window.tr("Signed in"); textFormat: Text.PlainText; elide: Text.ElideRight; color: window.theme.muted; font.pixelSize: 11; Layout.fillWidth: true }
                        }
                        Label { text: "⌃"; color: window.theme.muted }
                    }
                    Menu {
                        id: accountMenu; objectName: "accountMenu"; y: -height-8; width: 240; padding: 6
                        background: Rectangle { radius: 12; color: window.theme.canvas; border.color: window.theme.border }
                        component AccountItem: DriveMenuItem { theme: window.theme }
                        AccountItem { objectName: "accountSettings"; text: window.tr("Einstellungen"); onTriggered: controller.show_page("settings") }
                        AccountItem { objectName: "accountSubscription"; text: window.tr("Subscription"); onTriggered: controller.action("subscription") }
                        AccountItem { objectName: "accountDriveOnline"; text: window.tr("Proton Drive online"); onTriggered: controller.action("web") }
                        MenuSeparator { contentItem: Rectangle { implicitHeight: 1; color: window.theme.border } }
                        AccountItem { objectName: "accountSignOut"; text: window.tr("Abmelden"); enabled: !s.busy; onTriggered: controller.action("logout") }
                    }
                }
                Copy { text: window.tr("Unabhängige Open-Source-App\n")+s.version+(s.demo ? " · DEMO" : ""); font.pixelSize: 11; Layout.topMargin: 10 }
                Action { objectName: "donationButton"; text: "♡  "+window.tr("Support on Ko-fi"); command: "donate"; flat: true; font.pixelSize: 12; Layout.fillWidth: true; Accessible.description: window.tr("Opens Ko-fi in your browser. Donations are optional.") }
            }
        }
        Rectangle {
            Layout.fillWidth: true; Layout.fillHeight: true
            Layout.topMargin: 0; Layout.bottomMargin: 0; Layout.rightMargin: 0
            color: window.theme.canvas
        ColumnLayout {
            anchors.fill: parent; anchors.margins: window.width < 1050 ? 16 : 28; spacing: window.width < 1050 ? 12 : 16
            RowLayout {
                Layout.fillWidth: true
                ColumnLayout {
                    Layout.fillWidth: true; spacing: 5
                    Label { text: window.titles[s.page]; textFormat: Text.PlainText; font.pixelSize: window.width < 1050 ? 24 : 28; font.bold: true; color: window.theme.text; elide: Text.ElideRight; Layout.fillWidth: true }
                }
                Rectangle { visible: s.demo; implicitWidth: modeLabel.implicitWidth+24; implicitHeight: 30; radius: 15; color: window.theme.badge
                    Label { id: modeLabel; anchors.centerIn: parent; text: "Demo"; color: window.theme.accentText; font.pixelSize: 12 }
                }
            }
            Rectangle {
                visible: window.f("notice_label").visible; Layout.fillWidth: true; implicitHeight: notice.implicitHeight+24
                radius: 10; color: s.noticeError ? window.theme.warning : window.theme.badge
                Copy { id: notice; anchors { left: parent.left; right: parent.right; margins: 12; verticalCenter: parent.verticalCenter } text: window.f("notice_label").value; color: window.theme.text }
            }
            Action { visible: window.f("login_button").visible; text: window.tr("Anmeldung im Browser öffnen"); command: "openLogin" }
            Item { id: pageLoader; objectName: "pageLoader"; Layout.fillWidth: true; Layout.fillHeight: true
                // Keep the gallery's model, scroll position and decoded previews
                // alive between sections. Logout destroys the retained page.
                property bool photosVisited: false
                readonly property var item: s.page==="photos" ? photoLoader.item : sectionLoader.item
                Connections {
                    target: controller
                    function onStateChanged() {
                        if(!s.authenticated && !s.demo)pageLoader.photosVisited=false;
                        else if(s.page==="photos")pageLoader.photosVisited=true;
                    }
                }
                Loader { id: photoLoader; anchors.fill: parent
                    active: s.page==="photos" || pageLoader.photosVisited
                    visible: s.page==="photos"
                    sourceComponent: photosPage
                    onLoaded: pageLoader.photosVisited=true
                }
                Loader { id: sectionLoader; anchors.fill: parent
                    active: s.page!=="photos"; visible: active
                    sourceComponent: s.page==="files" ? filesPage : s.page==="computer-backup" ? computerBackupPage : s.page==="ondemand" ? mountPage : s.page==="local-trash" ? trashPage : s.page==="activity" ? activityPage : settingsPage
                }
            }
            RowLayout {
                Layout.fillWidth: true
                BusyIndicator { running: s.busy; visible: s.busy; implicitWidth: 24; implicitHeight: 24 }
                Copy { text: window.f("status_label").value; font.pixelSize: 12 }
                Action { visible: s.busy; text: window.tr("Abbrechen"); command: "cancel"; enabled: true }
            }
        }
        }
    }

    Component {
        id: filesPage
        ColumnLayout {
            objectName: "page-files"; spacing: window.width < 1050 ? 10 : 14
            Shortcut {
                sequences: [StandardKey.Find]
                enabled: s.page==="files" && (!s.busy || Boolean(s.reading)) && !window.dialogOpen
                onActivated: { folderSearch.forceActiveFocus(); folderSearch.selectAll(); }
            }
            Card {
                visible: s.path==="/devices"
                Heading { text: window.tr("Back up this computer") }
                Copy { text: window.tr("Choose local folders to keep in Proton Drive. Manage this computer's backup separately from browsing files saved by your computers.") }
                Action { objectName: "configureComputerBackup"; text: window.tr("Configure computer backup"); primary: true; onClicked: controller.show_page("computer-backup") }
            }
            Heading { visible: s.path==="/devices"; text: window.tr("Saved computers") }
            RowLayout {
                Layout.fillWidth: true; spacing: 12
                Input { field: "search"; placeholderText: window.tr("In diesem Ordner suchen …"); objectName: "searchInput"; leftPadding: 42
                    id: folderSearch
                    Accessible.description: placeholderText+" · Ctrl+F · Esc"
                    Keys.onEscapePressed: {
                        if(text.length)controller.edit("search", "");
                        else (window.gridView ? fileGrid : fileList).forceActiveFocus();
                    }
                    Keys.onDownPressed: (window.gridView ? fileGrid : fileList).forceActiveFocus()
                    DriveIcon { x: 13; anchors.verticalCenter: parent.verticalCenter; symbol: "search"; ink: window.muted; width: 18; height: 18 }
                }
                ViewToggle {
                    objectName: "listViewButton"
                    text: window.tr("Listenansicht"); checkable: true; checked: !window.gridView
                    Accessible.name: text
                    onClicked: window.gridView = false
                }
                ViewToggle {
                    objectName: "gridViewButton"
                    text: window.tr("Kachelansicht"); checkable: true; checked: window.gridView
                    Accessible.name: text
                    onClicked: window.gridView = true
                }
            }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action {
                    objectName: "addFilesButton"; visible: s.path!=="/devices" && s.path!=="/trash"
                    text: "+ " + window.tr("Add files"); primary: true
                    enabled: window.f("upload_button").enabled || window.f("folder_upload_button").enabled || window.f("mkdir_button").enabled
                    onClicked: addFilesMenu.popup()
                    DriveMenu {
                        id: addFilesMenu; objectName: "addFilesMenu"; theme: window.theme
                        DriveMenuItem { theme: window.theme; text: window.tr("Dateien hochladen"); enabled: window.f("upload_button").enabled; onTriggered: controller.action("upload") }
                        DriveMenuItem { theme: window.theme; text: window.tr("Ordner hochladen"); enabled: window.f("folder_upload_button").enabled; onTriggered: controller.action("uploadFolder") }
                        DriveMenuItem { theme: window.theme; text: window.tr("Neuer Ordner"); enabled: window.f("mkdir_button").enabled; onTriggered: controller.action("mkdir") }
                    }
                }
                Action { objectName: "localTrashButton"; visible: s.path==="/trash"; text: window.tr("Lokaler Papierkorb"); onClicked: controller.show_page("local-trash") }
                Action { text: window.tr("Aktualisieren"); command: "refresh"; field: "refresh_button" }
            }
            Flow { Layout.fillWidth: true; spacing: 4
                Repeater { model: s.crumbs; delegate: Action { required property string modelData; required property int index; text: modelData+"  ›"; implicitHeight: window.width < 1050 ? 32 : 40; flat: true; onClicked: controller.crumb(index) } }
            }
            Rectangle {
                Layout.fillHeight: true; Layout.fillWidth: true; radius: 12; color: window.theme.canvas; border.color: window.theme.cardBorder
                ColumnLayout {
                    anchors.fill: parent; anchors.margins: 12; spacing: 4
                    RowLayout {
                        visible: !window.gridView
                        Layout.fillWidth: true; Layout.margins: 10
                        Copy { text: window.tr("Name"); font.pixelSize: 12 }
                        Label { text: window.tr("Größe"); color: window.muted; font.pixelSize: 12; Layout.preferredWidth: 80 }
                        Label { text: window.tr("Geändert"); color: window.muted; font.pixelSize: 12; Layout.preferredWidth: 90 }
                    }
                    ListView {
                        id: fileList; objectName: "fileList"; visible: !window.gridView; Layout.fillWidth: true; Layout.fillHeight: true
                        clip: true; model: controller.fileEntries; spacing: 4;
                        activeFocusOnTab: true; currentIndex: s.focusIndex; keyNavigationEnabled: false
                        Keys.onPressed: function(event) {
                            if (event.matches(StandardKey.SelectAll)) { controller.select_all(); event.accepted=true }
                            else if (event.key===Qt.Key_Down || event.key===Qt.Key_Up) { controller.move_selection(event.key===Qt.Key_Down ? 1 : -1,Boolean(event.modifiers & Qt.ShiftModifier)); event.accepted=true }
                            else if (event.key===Qt.Key_Home || event.key===Qt.Key_End) {
                                let target=event.key===Qt.Key_Home ? 0 : controller.fileEntries.length-1;
                                if(event.modifiers & Qt.ShiftModifier)controller.select_range(target);else controller.select_row(target,false);
                                event.accepted=true;
                            }
                            else if (event.key===Qt.Key_Escape) { controller.send("clear_selection",[]); event.accepted=true }
                            else if (event.key===Qt.Key_Return || event.key===Qt.Key_Enter) { controller.row_activated(s.focusIndex); event.accepted=true }
                            else if (event.key===Qt.Key_Space) { controller.select_row(s.focusIndex,true); event.accepted=true }
                        }
                         boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: DriveScrollBar { theme: window.theme }
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
                            MouseArea { id: rowMouse; anchors.fill: parent; hoverEnabled: true; enabled: !s.busy || Boolean(s.reading)
                                onClicked: function(mouse) {
                                    fileList.forceActiveFocus();
                                    if(mouse.modifiers & Qt.ShiftModifier)controller.select_range(fileRow.index);
                                    else controller.select_row(fileRow.index,Boolean(mouse.modifiers & Qt.ControlModifier));
                                }
                                onDoubleClicked: controller.row_activated(fileRow.index)
                            }
                        }
                        Label { anchors.centerIn: parent; visible: fileList.count===0; text: window.f("empty_label").value; color: window.muted }
                    }
                    GridView {
                        id: fileGrid; objectName: "fileGrid"
                        visible: window.gridView
                        Layout.fillWidth: true; Layout.fillHeight: true
                        clip: true; model: controller.fileEntries
                        cellWidth: width / Math.max(1, Math.floor(width / 148)); cellHeight: 144
                        currentIndex: s.focusIndex; keyNavigationEnabled: false; activeFocusOnTab: true
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: DriveScrollBar { theme: window.theme }
                        Keys.onPressed: function(event) {
                            if (s.busy) return;
                            if (event.matches(StandardKey.SelectAll)) { controller.select_all(); event.accepted=true }
                            else if ([Qt.Key_Left,Qt.Key_Right,Qt.Key_Up,Qt.Key_Down].indexOf(event.key)>=0) {
                                let columns=Math.max(1,Math.floor(width/cellWidth));
                                let delta=event.key===Qt.Key_Left ? -1 : event.key===Qt.Key_Right ? 1 : event.key===Qt.Key_Up ? -columns : columns;
                                controller.move_selection(delta,Boolean(event.modifiers & Qt.ShiftModifier)); event.accepted=true;
                            }
                            else if (event.key===Qt.Key_Home || event.key===Qt.Key_End) {
                                let target=event.key===Qt.Key_Home ? 0 : controller.fileEntries.length-1;
                                if(event.modifiers & Qt.ShiftModifier)controller.select_range(target);else controller.select_row(target,false);
                                event.accepted=true;
                            }
                            else if (event.key===Qt.Key_Escape) { controller.send("clear_selection",[]); event.accepted=true }
                            else if (event.key===Qt.Key_Return || event.key===Qt.Key_Enter) { controller.row_activated(s.focusIndex); event.accepted=true }
                            else if (event.key===Qt.Key_Space) { controller.select_row(s.focusIndex,true); event.accepted=true }
                        }
                        delegate: Item {
                            id: tile
                            required property var modelData; required property int index
                            objectName: "fileTile-"+index
                            width: fileGrid.cellWidth; height: fileGrid.cellHeight
                            Accessible.role: Accessible.ListItem; Accessible.name: modelData.name
                            Accessible.selected: s.selection.indexOf(index)>=0
                            Rectangle {
                                anchors.fill: parent; anchors.margins: 4; radius: 10
                                color: s.selection.indexOf(tile.index)>=0 ? window.theme.rowSelected : tileMouse.containsMouse ? window.theme.rowHover : "transparent"
                                border.color: fileGrid.activeFocus && tile.index===s.focusIndex ? window.theme.accent : "transparent"
                                ColumnLayout {
                                    anchors.fill: parent; anchors.margins: 10; spacing: 6
                                    DriveIcon { Layout.alignment: Qt.AlignHCenter; Layout.preferredWidth: 48; Layout.preferredHeight: 48; Layout.minimumHeight: 48; Layout.maximumHeight: 48; symbol: tile.modelData.isFolder ? "folderTile" : "file"; ink: tile.modelData.isFolder ? window.theme.accent : window.theme.fileInk }
                                    Label { text: tile.modelData.name; textFormat: Text.PlainText; color: window.theme.text; Layout.fillWidth: true; Layout.preferredHeight: 36; Layout.minimumHeight: 36; Layout.maximumHeight: 36; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignTop; wrapMode: Text.Wrap; maximumLineCount: 2; elide: Text.ElideRight }
                                    Label { text: tile.modelData.safe ? tile.modelData.size : window.tr("Nicht eindeutig · gesperrt"); textFormat: Text.PlainText; color: window.muted; Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; elide: Text.ElideRight; font.pixelSize: 11 }
                                }
                                MouseArea {
                                    id: tileMouse; anchors.fill: parent; hoverEnabled: true; enabled: !s.busy || Boolean(s.reading)
                                    onClicked: function(mouse) {
                                    fileGrid.forceActiveFocus();
                                    if(mouse.modifiers & Qt.ShiftModifier)controller.select_range(tile.index);
                                    else controller.select_row(tile.index,Boolean(mouse.modifiers & Qt.ControlModifier));
                                }
                                    onDoubleClicked: controller.row_activated(tile.index)
                                }
                                ToolTip.visible: tileMouse.containsMouse
                                ToolTip.delay: 700
                                ToolTip.text: tile.modelData.name
                            }
                        }
                        Label { anchors.centerIn: parent; visible: fileGrid.count===0; text: window.f("empty_label").value; color: window.muted }
                    }
                }
            }
            Rectangle {
                visible: s.selection.length > 0
                Layout.fillWidth: true; implicitHeight: fileActions.implicitHeight + 16
                radius: 12; color: window.theme.sidebar; border.color: window.theme.cardBorder
                Flow {
                    id: fileActions; objectName: "fileActionBar"
                    anchors { left: parent.left; right: parent.right; top: parent.top; margins: 8 }
                    spacing: 6
                    Repeater { model: ["Download","Keep on device","Free up space","Umbenennen","Verschieben","Freigabe","Papierkorb","Wiederherstellen"]
                        delegate: Action { required property string modelData; text: window.tr(modelData); command: modelData; flat: true; implicitHeight: window.width < 1050 ? 34 : 40; enabled: s.actions[modelData].enabled; visible: s.actions[modelData].visible }
                    }
                    Action { text: window.tr("Version history"); command: "Versions"; flat: true; implicitHeight: window.width < 1050 ? 34 : 40; enabled: s.actions.Versions.enabled; visible: s.actions.Versions.visible }
                }
            }
            Copy { text: window.f("count_label").value; font.pixelSize: 12 }
        }
    }
    Component {
        id: photosPage
        PhotoLibraryPanel { host: window; controller: window.appController; viewState: s }
    }

    Dialog {
        id: historyDialog; objectName: "historyDialog"
        visible: Boolean(s.history && s.history.visible)
        title: window.tr("Version history"); modal: true
        anchors.centerIn: parent; width: Math.min(680,window.width-60); height: Math.min(500,window.height-80)
        footer: Item {
            implicitHeight: 64
            Action { objectName: "historyClose"; text: window.tr("Abbrechen / Schließen"); enabled: true; anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: 16; onClicked: historyDialog.reject() }
        }
        padding: 20
        background: Rectangle { radius: 14; color: window.theme.canvas; border.color: window.theme.dialogBorder }
        header: Label { text: window.tr("Version history"); color: window.theme.text; font.pixelSize: 20; font.bold: true; padding: 20 }
        onRejected: controller.send("history_close",[])
        contentItem: ListView {
            clip: true; model: s.history ? s.history.revisions : []; spacing: 10
            ScrollBar.vertical: DriveScrollBar { theme: window.theme }
            delegate: RowLayout {
                required property var modelData; required property int index
                width: ListView.view.width; spacing: 12
                ColumnLayout {
                    Layout.fillWidth: true
                    Label { text: modelData.created ? new Date(modelData.created).toLocaleString(window.locale,"dd MMM yyyy · HH:mm") : "—"; textFormat: Text.PlainText; color: window.theme.text }
                    Label { text: modelData.size===undefined ? "—" : Number(modelData.size).toLocaleString(window.locale,"f",0)+" B"; color: window.muted }
                }
                Action { text: modelData.state==="active" ? window.tr("Current version") : window.tr("Wiederherstellen"); enabled: !s.busy && modelData.state==="superseded"; onClicked: {controller.send("version_restore",[index]);controller.send("history_close",[]);} }
            }
        }
    }
    component PageScroll: ScrollView {
        ScrollBar.vertical: DriveScrollBar { theme: window.theme; parent: scroll; x: scroll.width-width; y: scroll.topPadding; height: scroll.availableHeight }
        id: scroll
        rightPadding: 14
        clip: true; contentWidth: availableWidth
        default property alias contents: pageColumn.data
        ColumnLayout { id: pageColumn; width: scroll.availableWidth; spacing: 16 }
    }
    component SyncSettings: ColumnLayout {
            id: syncSettingsPanel
            property bool additionalFolders: false
            property bool advanced: false
            Layout.fillWidth: true; spacing: 16; objectName: "syncSettings"
            Card {
                Heading { text: window.tr("Your sync folder") }
                Copy { visible: !s.sandboxed; text: window.f("mount_label").value }
                Copy { visible: !s.sandboxed; text: window.f("mount_status").value }
                Copy { text: s.sandboxed ? window.tr("This store edition synchronizes downloaded files. Online placeholders and native file-manager extensions are unavailable.") : window.tr("Files appear as online placeholders. Use Keep on device in the file menu to keep selected files or folders available offline.") }
                Flow { Layout.fillWidth: true; spacing: 8
                    Action { text: window.tr("Set up sync folder"); onClicked: controller.send("setup_open",[]); primary: true }
                    Action { visible: syncSettingsPanel.advanced && !s.sandboxed; text: window.tr("Sync settings / online drive only"); onClicked: controller.show_page("ondemand") }
                    Action { visible: syncSettingsPanel.advanced && !s.sandboxed; text: window.tr("Refresh sync status"); command: "mountStatus" }
                }
            }
            Card {
                Heading { text: window.f("background_status").value }
                RowLayout { Action { visible: syncSettingsPanel.advanced; text: window.tr("Dienst starten"); command: "start" } Action { visible: syncSettingsPanel.advanced; text: window.tr("Dienst stoppen"); command: "stop" } Action { objectName: "restartSyncButton"; text: window.tr("Restart sync service"); command: "restart"; enabled: !s.busy && !(s.syncSummary && s.syncSummary.syncing>0) } }
            }
            Action { objectName: "advancedSyncButton"; text: (syncSettingsPanel.advanced ? "▾ " : "▸ ") + window.tr("Advanced settings"); enabled: true; onClicked: syncSettingsPanel.advanced = !syncSettingsPanel.advanced }
            Card {
                objectName: "syncOverview"; visible: syncSettingsPanel.advanced
                Heading { text: window.tr("Sync overview") }
                Copy { text: window.f("background_status").value }
                GridLayout {
                    Layout.fillWidth: true; columns: width<650 ? 2 : 4; columnSpacing: 24; rowSpacing: 12
                    Repeater {
                        model: [{key:"total",label:window.tr("Folders")},{key:"syncing",label:window.tr("Syncing")},{key:"paused",label:window.tr("Pausiert")},{key:"attention",label:window.tr("Needs attention")}]
                        delegate: ColumnLayout {
                            required property var modelData
                            Layout.fillWidth: true; Layout.preferredWidth: 1; Layout.minimumWidth: 120
                            Label { text: s.syncSummary ? s.syncSummary[modelData.key] : "—"; font.pixelSize: 26; font.bold: true; color: window.theme.accentText }
                            Copy { text: modelData.label }
                        }
                    }
                }
            }
            Copy { text: window.tr("Änderungen automatisch abgleichen – auch nach dem Schließen des Fensters.") }
            Action { visible: syncSettingsPanel.advanced; text: (syncSettingsPanel.additionalFolders ? "▾ " : "▸ ") + window.tr("Additional sync folders"); enabled: true; onClicked: syncSettingsPanel.additionalFolders = !syncSettingsPanel.additionalFolders }
            Card {
                visible: syncSettingsPanel.advanced && syncSettingsPanel.additionalFolders
                Heading { text: window.tr("Ordner automatisch synchronisieren") }
                Copy { text: window.f("background_local_label").value }
                Action { text: window.tr("Lokalen Ordner wählen"); command: "chooseBackground" }
                Copy { text: window.tr("Cloud-Ordner") }
                Input { field: "background_remote" }
                Action { text: window.tr("Automatischen Sync aktivieren"); command: "addJob"; field: "background_add_button"; primary: true }
            }
            Repeater { model: (s.jobs || []).filter(job=>!job.computer)
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
    Component {
        id: computerBackupPage
        PageScroll {
            objectName: "page-computer-backup"
            Action { text: window.tr("Computer"); onClicked: controller.navigate(window.tr("Computer"),"/devices") }
            Card {
                Heading { text: window.tr("Computersicherung") }
                Copy { text: window.tr("Choose folders from this computer. They appear under its name in Computer, separately from your ProtonDrive sync folder.") }
                Copy { text: window.tr("This is two-way folder synchronization, not a system image. File changes and supported deletions can propagate in both directions. Keep a separate backup for recovery.") }
                Copy { text: window.f("background_local_label").value }
                Action { text: window.tr("Lokalen Ordner wählen"); command: "chooseBackground" }
                Action { objectName: "computerBackupButton"; text: window.tr("Ordner zur Computersicherung hinzufügen"); command: "addComputerFolder"; field: "background_add_button"; primary: true }
            }
            Repeater { model: (s.jobs || []).filter(job=>job.computer)
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
        }
    }
    Component {
        id: mountPage
        PageScroll {
            objectName: "page-ondemand"
            Copy { text: window.tr("Dateien beim Öffnen laden, offline vorhalten oder lokalen Speicher freigeben.") }
            Card {
                Heading { text: window.tr("Online-Laufwerk verbinden") }
                Copy { visible: !s.sandboxed; text: window.f("mount_label").value }
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
                Copy { visible: !s.sandboxed; text: window.f("mount_status").value }
            }
            Card {
                Heading { text: window.tr("Offline-Verfügbarkeit") }
                Input { field: "cache_path"; placeholderText: window.tr("Dateipfad, z. B. Dokumente/Notizen.txt") }
                RowLayout { Action { text: window.tr("Offline vorhalten"); command: "pin" } Action { text: window.tr("Speicher freigeben"); command: "evict" } }
                Action { text: window.tr("Laufwerkshelfer bei Proton anmelden"); command: "helperLogin" }
            }
            Copy { text: window.tr("Online placeholders require the native edition. Unsynced changes remain in recovery storage.") }
        }
    }
    Component {
        id: trashPage
        PageScroll {
            objectName: "page-local-trash"
            Action { text: "‹ " + window.tr("Cloud-Papierkorb"); onClicked: controller.navigate(window.tr("Cloud-Papierkorb"),"/trash") }
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
            id: settingsScroll; objectName: "page-settings"
            property bool syncTab: s.settingsSync===true
            property bool advanced: false
            RowLayout {
                Action { text: window.tr("Einstellungen"); primary: !settingsScroll.syncTab; onClicked: settingsScroll.syncTab=false }
                Action { text: window.tr("Automatischer Sync"); primary: settingsScroll.syncTab; onClicked: settingsScroll.syncTab=true }
            }
            SyncSettings { visible: settingsScroll.syncTab }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 16
                visible: !settingsScroll.syncTab

            Card {
                Heading { text: window.tr("Erscheinungsbild") }
                Copy { text: window.tr("Erscheinungsbild") }
                Choice {
                    objectName: "appearanceSelector"
                    Layout.fillWidth: true
                    model: [window.tr("Systemeinstellung"), window.tr("Hell"), window.tr("Dunkel")]
                    currentIndex: ["system", "light", "dark"].indexOf(controller.appearance.mode)
                    onActivated: controller.setAppearancePreference(["system", "light", "dark"][currentIndex])
                    Accessible.name: window.tr("Erscheinungsbild")
                }
                Copy { text: window.tr("Sprache") }
                Choice {
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
                Check {
                    text: window.tr("Automatisch täglich nach Updates suchen")
                    checked: controller.updates.automatic
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("automatic", checked)
                }
                Check {
                    text: window.tr("Updates automatisch herunterladen")
                    visible: ["deb", "rpm-fedora", "rpm-opensuse"].indexOf(controller.updates.packageFormat) >= 0
                    checked: controller.updates.autoDownload
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("download", checked)
                }
                Copy { text: controller.updates.status }
                Copy { text: window.tr("Die Prüfung kontaktiert GitHub. Es werden keine Konto- oder Dateidaten übertragen.") }
                Copy {
                    visible: ["deb", "rpm-fedora", "rpm-opensuse"].indexOf(controller.updates.packageFormat) < 0
                    text: window.tr("Dieses Installationsformat wird über den Paketmanager oder das Paket auf der Release-Seite aktualisiert.")
                }
                Flow {
                    Layout.fillWidth: true; spacing: 8
                    Action { text: window.tr("Nach Updates suchen"); enabled: !controller.updates.state.restartPending && !controller.updates.busy && !s.demo; onClicked: controller.updates.checkNow() }
                    Action { text: window.tr("Update herunterladen"); visible: controller.updates.downloadable && !controller.updates.ready; enabled: !controller.updates.busy; onClicked: controller.updates.downloadUpdate() }
                    Action { text: window.tr("Update installieren"); visible: controller.updates.ready; enabled: !controller.updates.busy && !s.busy; onClicked: controller.updates.installUpdate() }
                    Action { text: window.tr("Release-Seite öffnen"); visible: controller.updates.available; onClicked: controller.updates.openRelease() }
                }
                Action { text: window.tr("App und Dienste neu starten"); visible: controller.updates.state.restartPending === true; enabled: !controller.updates.busy && !s.busy; onClicked: controller.updates.restartUpdate() }
                Copy { visible: ["deb", "rpm-fedora", "rpm-opensuse"].indexOf(controller.updates.packageFormat) >= 0; text: window.tr("Nach erfolgreicher Installation werden App und Dienste automatisch neu gestartet. Offene Dateien und laufende Synchronisationen vorher abschließen.") }
            }
            Action { objectName: "advancedSettingsButton"; text: (settingsScroll.advanced ? "▾ " : "▸ ") + window.tr("Advanced settings"); enabled: true; onClicked: settingsScroll.advanced = !settingsScroll.advanced }
            Card {
                objectName: "connectionDiagnostics"; visible: settingsScroll.advanced
                Check {
                    visible: settingsScroll.advanced
                    text: window.tr("Vorabversionen einschließen")
                    checked: controller.updates.previews
                    enabled: !controller.updates.busy
                    onToggled: controller.updates.setOption("previews", checked)
                }
                Heading { text: window.tr("Konto & Verbindung") }
                Copy { text: window.f("backend_label").value }
                RowLayout { Action { text: window.tr("Backend prüfen"); command: "version" } Action { text: window.tr("Anmelden"); command: "login" } Action { text: window.tr("Abmelden"); command: "logout" } }
                Copy { text: window.tr("Anmeldung ausschließlich auf account.proton.me im Browser. Die Sitzung liegt im Schlüsselbund oder in pass. Flatpak kann einen eigenen Kontospeicher verwenden.") }
            }
            Card {
                Heading { text: "NeutronDrive "+s.version }
                Copy { visible: settingsScroll.advanced; text: "Qt Quick · QML · Rust · Open Source" }
                Action { text: window.tr("Projekt & Installation ↗"); command: "project" }
                Action { visible: settingsScroll.advanced; text: window.tr("Offizielle Proton CLI ↗"); command: "cli" }
                Copy { text: window.tr("Independent, open-source Proton Drive client for Linux. Not affiliated with Proton.") }
            }
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
            else if(value.kind==="share")shareWindow.present(value)
            else { promptInput.text=value.initial || ""; modal.open(); cancelDialog.forceActiveFocus() }
        }
    }
    ShareWindow { id: shareWindow; host: window; controller: window.appController }
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
        id: modal; objectName: "confirmationDialog"; padding: 24
        parent: shareWindow.visible ? shareWindow.contentItem : window.contentItem
        Overlay.modal: Rectangle { color: window.darkMode ? "#99000000" : "#66252236" }
        anchors.centerIn: parent; width: Math.min(560,window.width-60)
        modal: true; closePolicy: Popup.CloseOnEscape
        onRejected: controller.answer_dialog(window.request.token,false,"")
        background: Rectangle { color: window.theme.canvas; radius: 16; border.color: window.theme.dialogBorder }
        header: Label { text: window.request.title || ""; textFormat: Text.PlainText; wrapMode: Text.Wrap; color: window.theme.text; font.pixelSize: 20; font.bold: true; padding: 20 }
        contentItem: ColumnLayout {
            spacing: 16
            Copy { text: window.request.description || ""; color: window.theme.secondaryText }
            Field { id: promptInput; objectName: "promptInput"; visible: window.request.kind==="prompt"; Layout.fillWidth: true; selectByMouse: true; color: window.theme.text }
            Flow {
                Layout.fillWidth: true; spacing: 8
                Action { id: cancelDialog; objectName: "dialogCancel"; text: window.tr("Abbrechen / Schließen"); enabled: true
                    onClicked: modal.reject()
                }
                Action { objectName: "dialogAccept"; text: window.request.kind==="prompt" ? window.tr("Übernehmen") : window.tr("Bestätigen"); enabled: true; primary: true; visible: window.request.kind!=="share"
                    onClicked: { let token=window.request.token; let value=promptInput.text; modal.close(); controller.answer_dialog(token,true,value) }
                }
            }
        }
    }
}
