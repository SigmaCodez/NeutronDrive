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
    width: 600; height: 430; minimumWidth: 480; minimumHeight: 400
    title: request.title || host.tr("Freigabe")
    color: host.theme.canvas
    font.family: host.font.family; font.pixelSize: 14
    palette: host.palette
    function loading() { request={}; node={}; pending=false; email.text=""; role.currentIndex=0; show(); raise(); requestActivate(); }
    function present(value) { if(value.node && value.node.path!==node.path) { email.text=""; role.currentIndex=0; } request=value; node=value.node || node; pending=true; show(); raise(); requestActivate(); }
    function answer(value) { let token=request.token; pending=false; controller.answer_dialog(token,true,value); }
    onClosing: function(event) {
        event.accepted=false;
        if(controller.state.busy || host.confirmationOpen)return;
        if(pending)controller.answer_dialog(request.token,false,"");
        pending=false; hide();
    }
    ColumnLayout {
        anchors.fill: parent; anchors.margins: 24; spacing: 18
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
