import QtQuick
import QtQuick.Controls.Basic
ComboBox {
    id: control
    required property var theme
    implicitHeight: 44; leftPadding: 12; rightPadding: 32
    contentItem: Text { text: control.displayText; font: control.font; color: control.enabled ? control.theme.text : control.theme.disabledText; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
    indicator: Text { text: "⌄"; color: control.theme.muted; x: control.width-width-12; y: (control.height-height)/2 }
    background: Rectangle { radius: 10; color: control.enabled ? control.theme.input : control.theme.disabled; border.width: control.visualFocus ? 2 : 1; border.color: control.visualFocus ? control.theme.accent : control.theme.border }
    delegate: ItemDelegate {
        required property int index
        width: control.width-12; implicitHeight: 40; highlighted: control.highlightedIndex===index
        contentItem: Text { text: control.textAt(index); font: control.font; color: control.theme.text; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
        background: Rectangle { radius: 8; color: parent.highlighted ? control.theme.selected : "transparent" }
    }
    popup: Popup {
        y: control.height+6; width: control.width; padding: 6
        implicitHeight: Math.min(280,contentItem.implicitHeight+12)
        background: Rectangle { radius: 12; color: control.theme.canvas; border.color: control.theme.border }
        contentItem: ListView { clip: true; implicitHeight: contentHeight; model: control.popup.visible ? control.delegateModel : null; currentIndex: control.highlightedIndex; ScrollBar.vertical: DriveScrollBar { theme: control.theme } }
    }
}
