import QtQuick
import QtQuick.Controls.Basic
Button {
    id: control
    required property var theme
    property bool primary: false
    implicitHeight: 42; leftPadding: 14; rightPadding: 14; topPadding: 10; bottomPadding: 10; spacing: 8
    hoverEnabled: true
    Accessible.name: text
    font.weight: Font.Medium
    contentItem: Text {
        text: control.text; textFormat: Text.PlainText; font: control.font
        color: !control.enabled ? control.theme.disabledText : control.primary ? control.theme.onAccent : control.highlighted ? control.theme.accentText : control.theme.text
        horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: 12
        color: !control.enabled ? (control.flat ? "transparent" : control.theme.disabled) : control.primary ? (control.down || control.hovered ? control.theme.primaryHover : control.theme.accent) : control.down || control.highlighted ? control.theme.selected : control.hovered ? control.theme.hover : control.flat ? "transparent" : control.theme.canvas
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus || control.primary ? control.theme.accent : control.flat ? "transparent" : control.theme.border
        Rectangle {anchors {fill:parent;topMargin:1;bottomMargin:-2} z:-1;radius:parent.radius;visible:control.enabled && !control.flat && !control.down;color:Qt.rgba(0,0,0,0.10)}
        Rectangle {anchors {left:parent.left;right:parent.right;top:parent.top;leftMargin:10;rightMargin:10;topMargin:1} height:1;visible:control.enabled && !control.flat;color:Qt.rgba(1,1,1,control.primary?0.22:0.12)}
        Behavior on color { ColorAnimation { duration: 100 } }
    }
}
