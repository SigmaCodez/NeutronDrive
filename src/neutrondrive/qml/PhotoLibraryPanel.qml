import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: panel
    required property var host
    required property var controller
    required property var viewState
    property bool albumsVisible: false
    property var selected: []
    property int albumIndex: 0
    property real savedScroll: 0
    property bool restoring: false
    objectName: "page-photos"
    spacing: 12
    component PhotoButton: Button {
        id: control; implicitHeight: 36; padding: 10
        contentItem: Text { text: control.text; font: control.font; color: control.enabled ? host.theme.text : host.theme.disabledText; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
        background: Rectangle { radius: 8; color: control.highlighted ? host.theme.selected : control.hovered ? host.theme.hover : host.theme.canvas; border.color: control.activeFocus ? host.theme.accent : host.theme.border }
    }
    function request(value) { controller.send("photo_library", [value]); }
    function openAlbum(uid) {
        selected=[]; savedScroll=0; albumsVisible=false;
        request({action:"index",album:uid});
    }
    function toggle(index) {
        let items=selected.slice(); let at=items.indexOf(index);
        if(at>=0)items.splice(at,1);else if(items.length<48)items.push(index);
        selected=items;
    }
    function selectedAlbum() { return (panel.viewState.photoAlbums || [])[albumIndex]; }
    function changeAlbum(action) {
        let album=selectedAlbum();
        if(album)request({action:action,album:album.uid,indices:selected});
    }
    RowLayout {
        PhotoButton { text: host.tr("Photos"); highlighted: !panel.albumsVisible; enabled: !panel.viewState.busy; onClicked: panel.openAlbum("") }
        PhotoButton { objectName: "photoAlbumsButton"; text: host.tr("Albums"); highlighted: panel.albumsVisible; enabled: !panel.viewState.busy; onClicked: { panel.albumsVisible=true; panel.request({action:"albums"}); } }
        Item { Layout.fillWidth: true }
        Label { text: panel.viewState.photoTotal===null ? "" : host.tr("Total photos")+": "+panel.viewState.photoTotal; color: host.theme.text }
        PhotoButton { text: host.tr("Refresh"); enabled: !panel.viewState.busy; onClicked: panel.albumsVisible ? panel.request({action:"albums"}) : panel.openAlbum(panel.viewState.photoAlbum || "") }
    }
    RowLayout {
        visible: panel.albumsVisible
        PhotoButton { text: host.tr("New album"); enabled: !panel.viewState.busy; onClicked: panel.request({action:"create"}) }
        Label { text: host.tr("Deleting an album keeps its photos in the timeline."); wrapMode: Text.Wrap; color: host.theme.text; Layout.fillWidth: true }
    }
    Flow {
        Layout.fillWidth: true; spacing: 8
        visible: !panel.albumsVisible && panel.selected.length>0
        Label { text: host.tr("Selected")+": "+panel.selected.length+" / 48"; color: host.theme.text }
        ComboBox { width: 200; model: panel.viewState.photoAlbums || []; textRole: "name"; currentIndex: panel.albumIndex; onActivated: panel.albumIndex=currentIndex }
        PhotoButton { text: host.tr("Load albums"); enabled: !panel.viewState.busy; onClicked: panel.request({action:"albums"}) }
        PhotoButton { text: host.tr("Add to album"); enabled: !panel.viewState.busy && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("add") }
        PhotoButton { text: host.tr("Remove from album"); enabled: !panel.viewState.busy && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("remove") }
        PhotoButton { text: host.tr("Set album cover"); enabled: !panel.viewState.busy && panel.selected.length===1 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("cover") }
    }
    GridView {
        id: albums; objectName: "photoAlbumsGrid"
        visible: panel.albumsVisible; Layout.fillWidth: true; Layout.fillHeight: true
        clip: true; model: panel.viewState.photoAlbums || []; cellWidth: width/Math.max(1,Math.floor(width/240)); cellHeight: 170
        ScrollBar.vertical: ScrollBar {}
        delegate: Rectangle {
            required property var modelData
            width: albums.cellWidth-12; height: albums.cellHeight-12
            radius: 12; color: host.theme.fileTile
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 12
                Label { text: modelData.name; textFormat: Text.PlainText; color: host.theme.text; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                Label { text: host.tr("Photos")+": "+modelData.count; color: host.theme.text }
                RowLayout {
                    PhotoButton { text: host.tr("Open"); enabled: !panel.viewState.busy && modelData.safe; onClicked: panel.openAlbum(modelData.uid) }
                    PhotoButton { text: "⋯"; Accessible.name: host.tr("Album actions"); enabled: !panel.viewState.busy && modelData.safe; onClicked: albumMenu.popup() }
                }
            }
            Menu {
                id: albumMenu
                MenuItem { text: host.tr("Rename album"); onTriggered: panel.request({action:"rename",album:modelData.uid}) }
                MenuItem { text: host.tr("Delete album"); onTriggered: panel.request({action:"delete",album:modelData.uid}) }
            }
        }
        Label { anchors.centerIn: parent; visible: !panel.viewState.busy && albums.count===0; text: host.tr("No albums yet"); color: host.theme.text }
    }
    GridView {
        id: grid
        objectName: "photoGrid"; visible: !panel.albumsVisible
        Layout.fillWidth: true; Layout.fillHeight: true
        clip: true; model: panel.viewState.photos || []; cellWidth: width/Math.max(1,Math.floor(width/200)); cellHeight: 250
        cacheBuffer: 250
        ScrollBar.vertical: ScrollBar {}
        onContentYChanged: if(!panel.restoring) { panel.savedScroll=Math.max(0,contentY); loadTimer.restart(); }
        onModelChanged: {
            panel.restoring=true;
            Qt.callLater(function() { grid.contentY=Math.min(panel.savedScroll,Math.max(0,grid.contentHeight-grid.height)); panel.restoring=false; });
        }
        delegate: Item {
            id: photo; required property var modelData; required property int index
            width: grid.cellWidth; height: grid.cellHeight
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 8; spacing: 6
                Rectangle {
                    Layout.fillWidth: true; Layout.preferredHeight: 140; color: host.theme.fileTile; radius: 10; clip: true
                    border.width: panel.selected.indexOf(photo.index)>=0 ? 2 : 0; border.color: host.theme.text
                    DriveIcon { anchors.centerIn: parent; symbol: "photo"; ink: host.theme.fileInk; visible: preview.status!==Image.Ready }
                    Image { id: preview; objectName: "photoPreview-"+photo.index; anchors.fill: parent; anchors.margins: 3; source: photo.modelData.thumbnail || ""; sourceSize.width: 400; sourceSize.height: 280; fillMode: Image.PreserveAspectFit; asynchronous: true; cache: false }
                    MouseArea { anchors.fill: parent; acceptedButtons: Qt.LeftButton | Qt.RightButton; onClicked: function(mouse) { if(mouse.button===Qt.RightButton){if(panel.selected.indexOf(photo.index)<0)panel.selected=[photo.index];photoMenu.popup();}else panel.toggle(photo.index); } }
                }
                Label { text: photo.modelData.name; textFormat: Text.PlainText; color: host.theme.text; Layout.fillWidth: true; elide: Text.ElideMiddle }
                CheckBox { text: host.tr("Select photo"); checked: panel.selected.indexOf(photo.index)>=0; enabled: photo.modelData.safe; onClicked: panel.toggle(photo.index) }
                RowLayout {
                    PhotoButton { text: host.tr("Download"); enabled: !panel.viewState.busy && photo.modelData.safe; onClicked: controller.send("photo_download",[photo.index]) }
                    PhotoButton { text: host.tr("Map"); visible: Boolean(photo.modelData.location); ToolTip.text: host.tr("Open this location on OpenStreetMap in your browser"); ToolTip.visible: hovered; onClicked: panel.request({action:"map",index:photo.index}) }
                }
            }
            Menu {
                id: photoMenu
                MenuItem { text: host.tr("Download"); enabled: !panel.viewState.busy && photo.modelData.safe; onTriggered: controller.send("photo_download",[photo.index]) }
                MenuItem { text: host.tr("Show location on OpenStreetMap"); enabled: Boolean(photo.modelData.location); onTriggered: panel.request({action:"map",index:photo.index}) }
                MenuItem { text: host.tr("Add to album"); enabled: !panel.viewState.busy; onTriggered: panel.request({action:"albums"}) }
            }
        }
        Label { anchors.centerIn: parent; visible: !panel.viewState.busy && grid.count===0; text: host.tr("No photos"); color: host.theme.text }
    }
    PhotoButton { visible: !panel.albumsVisible && Boolean(panel.viewState.photosMore); Layout.alignment: Qt.AlignHCenter; text: host.tr("Load more photos"); enabled: !panel.viewState.busy; onClicked: panel.request({action:"more"}) }
    Timer {
        id: loadTimer; interval: 250
        onTriggered: {
            if(panel.viewState.busy || panel.albumsVisible || panel.restoring)return;
            let first=Math.max(0,Math.floor(grid.contentY/grid.cellHeight)*Math.max(1,Math.floor(grid.width/200)));
            if(first<grid.count && !panel.viewState.photos[first].previewLoaded) { panel.request({action:"previews",start:first}); return; }
            if(panel.viewState.photosMore && grid.contentY+grid.height>=grid.contentHeight-grid.cellHeight)panel.request({action:"more"});
        }
    }
}
