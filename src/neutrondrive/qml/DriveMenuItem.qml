import QtQuick
import QtQuick.Controls.Basic
MenuItem {
    id: control
    required property var theme
    implicitHeight: 40
    contentItem: Text { text: control.text; textFormat: Text.PlainText; font: control.font; color: control.enabled ? control.theme.text : control.theme.disabledText; verticalAlignment: Text.AlignVCenter; leftPadding: 10; rightPadding: 10; elide: Text.ElideRight }
    background: Rectangle { radius: 8; color: control.highlighted ? control.theme.selected : "transparent"; border.color: control.visualFocus ? control.theme.accent : "transparent" }
}
