import QtQuick
import QtQuick.Controls.Basic
Button {
    id: control
    required property var theme
    property bool primary: false
    implicitHeight: 40; padding: 12; spacing: 8
    font.weight: Font.Medium
    contentItem: Text {
        text: control.text; textFormat: Text.PlainText; font: control.font
        color: !control.enabled ? control.theme.disabledText : control.primary ? control.theme.onAccent : control.highlighted ? control.theme.accentText : control.theme.text
        horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: 10
        color: !control.enabled ? control.theme.disabled : control.primary ? (control.down || control.hovered ? control.theme.primaryHover : control.theme.accent) : control.down || control.highlighted ? control.theme.selected : control.hovered ? control.theme.hover : control.flat ? "transparent" : control.theme.canvas
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus || control.primary ? control.theme.accent : control.flat ? "transparent" : control.theme.border
        Behavior on color { ColorAnimation { duration: 100 } }
    }
}
