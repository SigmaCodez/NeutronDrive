import QtQuick
import QtQuick.Controls.Basic
ScrollBar {
    id: control
    required property var theme
    padding: 2; minimumSize: 0.08
    contentItem: Rectangle {
        implicitWidth: 8; implicitHeight: 8; radius: 4
        color: control.pressed ? control.theme.accent : control.theme.muted
        opacity: control.size>=1 ? 0 : control.active || control.hovered ? 0.8 : 0.3
        Behavior on opacity { NumberAnimation { duration: 120 } }
    }
    background: Rectangle { color: "transparent" }
}
