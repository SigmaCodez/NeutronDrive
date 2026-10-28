import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: share
    required property var host
    required property var controller
    property var request: ({})
    property bool pending: false
    property var node: ({})
    objectName: "shareWindow"
    modality: Qt.ApplicationModal
    width: 600; height: 700; minimumWidth: 480; minimumHeight: 400
    title: request.title || host.tr("Freigabe")
    color: host.theme.canvas
    font.family: host.font.family; font.pixelSize: 14
    palette: host.palette
    function loading() { request={}; node={}; pending=false; email.text=""; role.currentIndex=0; show(); raise(); requestActivate(); }
    function present(value) { if(value.node && value.node.path!==node.path) { email.text=""; role.currentIndex=0; } request=value; node=value.node || node; let settings=value.linkSettings || {}; linkRole.currentIndex=settings.role==="editor"?1:0; linkPassword.text=settings.password || ""; linkExpiration.text=(settings.expiration || "").substring(0,10); editorsShare.checked=Boolean(settings.editorsCanShare); pending=true; show(); raise(); requestActivate(); }
    function expirationValue() {
        const date=linkExpiration.text.trim();
        const original=(request.linkSettings || {}).expiration || "";
        if(date===original.substring(0,10))return original;
        return /^\d{4}-\d{2}-\d{2}$/.test(date) ? date+"T23:59:59.000Z" : date;
    }
    function answer(value) { let token=request.token; pending=false; controller.answer_dialog(token,true,value); }
    onClosing: function(event) {
        event.accepted=false;
        if(controller.state.busy || host.confirmationOpen)return;
        if(pending)controller.answer_dialog(request.token,false,"");
        pending=false; hide();
    }
    ScrollView {
        anchors.fill: parent; anchors.margins: 24; clip: true
        contentWidth: availableWidth
    ColumnLayout {
        width: share.width - 48; spacing: 12
        Label { text: share.title; textFormat: Text.PlainText; color: host.theme.text; font.pixelSize: 22; font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Label { text: share.request.description || host.tr("Freigabestatus laden"); color: host.theme.muted; textFormat: Text.PlainText; wrapMode: Text.Wrap; Layout.fillWidth: true }
        TextArea {
            objectName: "shareLink"; Layout.fillWidth: true; visible: Boolean(share.request.url)
            text: share.request.url || ""; textFormat: Text.PlainText; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere
            color: host.theme.accentText; padding: 12
            background: Rectangle { radius: 10; color: host.theme.input; border.color: host.theme.border }
        }
        RowLayout {
            Layout.fillWidth: true
            DriveField { id: email; theme: host.theme; objectName: "shareEmail"; Layout.fillWidth: true; placeholderText: host.tr("Email address"); inputMethodHints: Qt.ImhEmailCharactersOnly; maximumLength: 254 }
            DriveCombo { id: role; theme: host.theme; model: [host.tr("Viewer"),host.tr("Editor")] }
        }
        Flow {
            Layout.fillWidth: true; spacing: 8
            DriveButton { theme: host.theme; text: host.tr("Invite by email"); enabled: share.pending && !controller.state.busy && email.text.trim().length>0; onClicked: share.answer({action:"invite",email:email.text,role:role.currentIndex===0?"viewer":"editor"}) }
            DriveButton { objectName: "copyShareLink"; theme: host.theme; text: host.tr("Link kopieren"); visible: Boolean(share.request.url); enabled: share.pending && !controller.state.busy; onClicked: share.answer("copy") }
            DriveButton { theme: host.theme; text: share.request.url ? host.tr("Link deaktivieren") : host.tr("Öffentlichen Leselink erstellen"); enabled: share.pending && !controller.state.busy; onClicked: share.answer(share.request.url ? "revoke" : "create") }
        }
        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: host.theme.border }
        Label { text: host.tr("Link settings"); color: host.theme.text; font.bold: true }
        GridLayout {
            Layout.fillWidth: true; columns: 2; columnSpacing: 12; rowSpacing: 8
            Label { text: host.tr("Link access"); color: host.theme.muted }
            Label { text: host.tr("Password (empty: no password)"); color: host.theme.muted; Layout.fillWidth: true; wrapMode: Text.Wrap }
            DriveCombo { id: linkRole; objectName: "shareLinkRole"; theme: host.theme; Layout.fillWidth: true; model: [host.tr("Viewer"),host.tr("Editor")] }
            DriveField { id: linkPassword; objectName: "sharePassword"; theme: host.theme; Layout.fillWidth: true; echoMode: TextInput.Password; maximumLength: 256 }
            Label { text: host.tr("Expiration date, end of day UTC (empty: no expiration)"); color: host.theme.muted; wrapMode: Text.Wrap; Layout.fillWidth: true; Layout.columnSpan: 2 }
            DriveField { id: linkExpiration; objectName: "shareExpiration"; theme: host.theme; Layout.fillWidth: true; Layout.columnSpan: 2; placeholderText: "2030-12-31"; maximumLength: 10 }
            DriveCheck { theme: host.theme; id: editorsShare; objectName: "shareEditorsCanShare"; text: host.tr("Editors can share"); Layout.fillWidth: true; Layout.columnSpan: 2 }
        }
        DriveButton { objectName: "saveShareSettings"; theme: host.theme; text: host.tr("Save link settings"); enabled: share.pending && !controller.state.busy; onClicked: share.answer({action:"settings",role:linkRole.currentIndex===0?"viewer":"editor",password:linkPassword.text,expiration:share.expirationValue(),editorsCanShare:editorsShare.checked}) }
        Label { Layout.fillWidth: true; text: controller.state.fields.notice_label.value; visible: text.length>0; wrapMode: Text.Wrap; textFormat: Text.PlainText; color: host.theme.text }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.fillWidth: true
            BusyIndicator { running: controller.state.busy; visible: running; implicitWidth: 28; implicitHeight: 28 }
            DriveButton { theme: host.theme; text: host.tr("Aktualisieren"); visible: !share.pending && Boolean(share.node.path); enabled: !controller.state.busy && !host.confirmationOpen; onClicked: controller.send("share_open",[share.node]) }
            Item { Layout.fillWidth: true }
            DriveButton { theme: host.theme; text: host.tr("Abbrechen / Schließen"); enabled: !controller.state.busy && !host.confirmationOpen; onClicked: share.close() }
        }
    }
    }
}
