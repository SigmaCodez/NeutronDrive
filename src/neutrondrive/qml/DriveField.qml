import QtQuick
import QtQuick.Controls.Basic
TextField {
    id: control
    required property var theme
    implicitHeight: 44; padding: 12; selectByMouse: true
    color: enabled ? theme.text : theme.disabledText; placeholderTextColor: theme.muted
    selectionColor: theme.accent; selectedTextColor: theme.onAccent
    background: Rectangle {
        radius: 10; color: control.enabled ? control.theme.input : control.theme.disabled
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? control.theme.accent : control.theme.border
    }
}
