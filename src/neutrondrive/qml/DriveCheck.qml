import QtQuick
import QtQuick.Controls.Basic
CheckBox {
    id: control
    required property var theme
    implicitHeight: Math.max(36,contentItem.implicitHeight+12); padding: 6; spacing: 10
    indicator: Rectangle {
        x: control.leftPadding; y: (control.height-height)/2; width: 22; height: 22; radius: 6
        color: !control.enabled ? control.theme.disabled : control.checked ? control.theme.accent : control.theme.input
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus || control.checked ? control.theme.accent : control.theme.border
        Text { anchors.centerIn: parent; text: control.checkState===Qt.PartiallyChecked ? "−" : control.checked ? "✓" : ""; color: control.enabled ? control.theme.onAccent : control.theme.disabledText; font.pixelSize: 15; font.bold: true }
    }
    contentItem: Text {
        text: control.text; textFormat: Text.PlainText; font: control.font
        color: control.enabled ? control.theme.text : control.theme.disabledText
        leftPadding: control.indicator.width+control.spacing; verticalAlignment: Text.AlignVCenter; wrapMode: Text.Wrap
    }
}
