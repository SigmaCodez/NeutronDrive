import QtQuick
import QtQuick.Controls.Basic
Menu {
    required property var theme
    padding: 6
    background: Rectangle { radius: 12; color: theme.canvas; border.color: theme.border }
}
