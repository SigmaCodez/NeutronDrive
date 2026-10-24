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
    property bool selectionMode: false
    property int tileSize: 200
    property int viewerIndex: -1
    property real previewZoom: 1
    property int previewAttempt: -1
    property int moreAttempt: -1
    property int viewerAttempt: -1
    readonly property var currentPhoto: viewerIndex >= 0 ? (viewState.photos || [])[viewerIndex] || ({}) : ({})
    function showPhoto(index) {
        viewerIndex=index; viewerAttempt=-1; previewZoom=1; viewer.open();
    }
    function leaveSelection() { selected=[]; selectionMode=false; }
    function dateText(value) {
        if(!value)return "";
        let date=new Date(value);
        return isNaN(date.getTime()) ? "" : date.toLocaleDateString(Qt.locale(controller.language), Locale.LongFormat);
    }
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
        leaveSelection(); viewer.close(); viewerIndex=-1; previewAttempt=-1; moreAttempt=-1; savedScroll=0; albumsVisible=false;
        request({action:"index",album:uid});
    }
    function toggle(index) {
        if(!selectionMode || !(viewState.photos || [])[index]?.safe)return;
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
        PhotoButton { objectName: "photoAlbumsButton"; text: host.tr("Albums"); highlighted: panel.albumsVisible; enabled: !panel.viewState.busy; onClicked: { panel.leaveSelection(); panel.albumsVisible=true; panel.request({action:"albums"}); } }
        Item { Layout.fillWidth: true }
        PhotoButton {
            objectName: "photoSelectionButton"; visible: !panel.albumsVisible
            text: panel.selectionMode ? host.tr("Done") : host.tr("Select photos")
            highlighted: panel.selectionMode
            onClicked: { if(panel.selectionMode)panel.leaveSelection(); else { panel.selectionMode=true; if(!panel.viewState.busy && !(panel.viewState.photoAlbums || []).length)panel.request({action:"albums"}); } }
        }
        Label { text: panel.viewState.photoTotal===null ? "" : host.tr("Total photos")+": "+panel.viewState.photoTotal; color: host.theme.text }
        PhotoButton { text: host.tr("Refresh"); enabled: !panel.viewState.busy; onClicked: panel.albumsVisible ? panel.request({action:"albums"}) : panel.openAlbum(panel.viewState.photoAlbum || "") }
    }
    RowLayout {
        visible: !panel.albumsVisible; spacing: 12
        Label { text: panel.viewState.photoAlbum ? ((panel.viewState.photoAlbums || []).find(a=>a.uid===panel.viewState.photoAlbum)?.name || host.tr("Albums")) : host.tr("Your photo library"); color: host.theme.muted; Layout.fillWidth: true; elide: Text.ElideRight }
        Label { text: host.tr("Thumbnail size"); color: host.theme.muted }
        Slider {
            id: sizeSlider; from: 150; to: 300; value: panel.tileSize; stepSize: 25
            Layout.preferredWidth: 130; Accessible.name: host.tr("Thumbnail size"); onMoved: panel.tileSize=value
            background: Rectangle {
                x: sizeSlider.leftPadding; y: sizeSlider.topPadding+sizeSlider.availableHeight/2-height/2
                width: sizeSlider.availableWidth; height: 4; radius: 2; color: host.theme.border
                Rectangle { width: sizeSlider.visualPosition*parent.width; height: parent.height; radius: 2; color: host.theme.accent }
            }
            handle: Rectangle {
                x: sizeSlider.leftPadding+sizeSlider.visualPosition*(sizeSlider.availableWidth-width)
                y: sizeSlider.topPadding+sizeSlider.availableHeight/2-height/2
                width: 16; height: 16; radius: 8; color: host.theme.canvas
                border.width: sizeSlider.activeFocus ? 3 : 2; border.color: host.theme.accent
            }
        }
    }
    RowLayout {
        visible: panel.albumsVisible
        PhotoButton { text: host.tr("New album"); enabled: !panel.viewState.busy; onClicked: panel.request({action:"create"}) }
        Label { text: host.tr("Deleting an album keeps its photos in the timeline."); wrapMode: Text.Wrap; color: host.theme.text; Layout.fillWidth: true }
    }
    Flow {
        Layout.fillWidth: true; spacing: 8
        visible: !panel.albumsVisible && panel.selectionMode
        Label { text: host.tr("Selected")+": "+panel.selected.length+" / 48"; color: host.theme.text }
        PhotoButton { text: host.tr("Select up to 48"); enabled: !panel.viewState.busy; onClicked: panel.selected=(panel.viewState.photos || []).map((p,i)=>p.safe ? i : -1).filter(i=>i>=0).slice(0,48) }
        PhotoButton { text: host.tr("Clear selection"); enabled: panel.selected.length>0; onClicked: panel.selected=[] }
        ComboBox {
            id: albumChoice; width: 200; implicitHeight: 36
            model: panel.viewState.photoAlbums || []; textRole: "name"; currentIndex: panel.albumIndex; onActivated: panel.albumIndex=currentIndex
            Accessible.name: host.tr("Albums")
            background: Rectangle { radius: 8; color: host.theme.input; border.color: albumChoice.activeFocus ? host.theme.accent : host.theme.border }
            contentItem: Text { text: albumChoice.displayText; font: albumChoice.font; color: host.theme.text; leftPadding: 10; rightPadding: 26; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
            indicator: Text { text: "⌄"; color: host.theme.muted; anchors.right: parent.right; anchors.rightMargin: 10; anchors.verticalCenter: parent.verticalCenter }
            delegate: ItemDelegate { required property var modelData; width: albumChoice.width; text: modelData.name; palette.text: host.theme.text; background: Rectangle { color: parent.highlighted ? host.theme.selected : host.theme.canvas; radius: 6 } }
            popup: Popup { y: albumChoice.height+4; width: albumChoice.width; padding: 6; implicitHeight: Math.min(240,contentItem.implicitHeight+12)
                background: Rectangle { radius: 10; color: host.theme.canvas; border.color: host.theme.border }
                contentItem: ListView { clip: true; implicitHeight: contentHeight; model: albumChoice.popup.visible ? albumChoice.delegateModel : null; currentIndex: albumChoice.highlightedIndex; ScrollIndicator.vertical: ScrollIndicator {} }
            }
        }
        PhotoButton { text: host.tr("Add to album"); enabled: !panel.viewState.busy && panel.selected.length>0 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("add") }
        PhotoButton { text: host.tr("Remove from album"); enabled: !panel.viewState.busy && panel.selected.length>0 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("remove") }
        PhotoButton { text: host.tr("Set album cover"); enabled: !panel.viewState.busy && panel.selected.length===1 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("cover") }
    }
    GridView {
        id: albums; objectName: "photoAlbumsGrid"
        visible: panel.albumsVisible; Layout.fillWidth: true; Layout.fillHeight: true
        clip: true; model: panel.viewState.photoAlbums || []; cellWidth: width/Math.max(1,Math.floor(width/240)); cellHeight: 240
        ScrollBar.vertical: ScrollBar {}
        delegate: Rectangle {
            id: albumTile
            required property var modelData
            width: albums.cellWidth-12; height: albums.cellHeight-12
            radius: 12; color: host.theme.canvas; border.color: host.theme.cardBorder
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 10; spacing: 10
                Rectangle {
                    Layout.fillWidth: true; Layout.fillHeight: true; radius: 10; color: host.theme.selected; clip: true
                    Image {
                        id: albumCover; anchors.fill: parent
                        source: (panel.viewState.photos || []).find(p=>p.uid===albumTile.modelData.cover)?.thumbnail || ""
                        fillMode: Image.PreserveAspectCrop; asynchronous: true; cache: false
                    }
                    DriveIcon { anchors.centerIn: parent; symbol: "photo"; ink: host.theme.accentText; visible: albumCover.status!==Image.Ready }
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; enabled: !panel.viewState.busy && albumTile.modelData.safe; onClicked: panel.openAlbum(albumTile.modelData.uid) }
                }
                RowLayout {
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 3
                        Label { text: albumTile.modelData.name; textFormat: Text.PlainText; color: host.theme.text; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                        Label { text: host.tr("Photos")+": "+albumTile.modelData.count; color: host.theme.muted }
                    }
                    PhotoButton { text: "⋯"; Accessible.name: host.tr("Album actions"); enabled: !panel.viewState.busy && albumTile.modelData.safe; onClicked: albumMenu.popup() }
                }
            }
            Menu {
                id: albumMenu
                MenuItem { text: host.tr("Open"); onTriggered: panel.openAlbum(modelData.uid) }
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
        clip: true; model: panel.viewState.photos || []; cellWidth: width/Math.max(1,Math.floor(width/panel.tileSize)); cellHeight: cellWidth
        cacheBuffer: 250
        activeFocusOnTab: true; keyNavigationEnabled: true
        Keys.onReturnPressed: if(currentIndex>=0)panel.showPhoto(currentIndex)
        Keys.onSpacePressed: if(panel.selectionMode && currentIndex>=0)panel.toggle(currentIndex)
        ScrollBar.vertical: ScrollBar {}
        onContentYChanged: if(!panel.restoring) { panel.savedScroll=Math.max(0,contentY); panel.previewAttempt=-1; panel.moreAttempt=-1; loadTimer.restart(); }
        onModelChanged: {
            panel.restoring=true;
            Qt.callLater(function() { grid.contentY=Math.min(panel.savedScroll,Math.max(0,grid.contentHeight-grid.height)); panel.restoring=false; });
        }
        delegate: Item {
            id: photo; required property var modelData; required property int index
            width: grid.cellWidth; height: grid.cellHeight
            Rectangle {
                anchors.fill: parent; anchors.margins: 4
                color: host.theme.fileTile; radius: 12; clip: true
                DriveIcon { anchors.centerIn: parent; symbol: "photo"; ink: host.theme.fileInk; visible: preview.status!==Image.Ready }
                Image {
                    id: preview; objectName: "photoPreview-"+photo.index
                    anchors.fill: parent; anchors.margins: panel.selected.indexOf(photo.index)>=0 ? 4 : 0
                    source: photo.modelData.thumbnail || ""; sourceSize.width: 600; sourceSize.height: 600
                    fillMode: Image.PreserveAspectCrop; asynchronous: true; cache: false
                }
                MouseArea {
                    id: photoMouse; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.LeftButton | Qt.RightButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: function(mouse) {
                        grid.currentIndex=photo.index; grid.forceActiveFocus();
                        if(mouse.button===Qt.RightButton)photoMenu.popup();
                        else if(panel.selectionMode)panel.toggle(photo.index);
                        else panel.showPhoto(photo.index);
                    }
                    onDoubleClicked: if(!panel.selectionMode)panel.showPhoto(photo.index)
                }
                Rectangle { anchors.fill: parent; color: "transparent"; radius: 12; border.width: panel.selected.indexOf(photo.index)>=0 ? 3 : photoMouse.containsMouse ? 1 : 0; border.color: host.theme.accent }
                CheckBox {
                    id: photoCheck; objectName: "photoCheck-"+photo.index
                    anchors { top: parent.top; left: parent.left; margins: 10 }
                    visible: panel.selectionMode; enabled: photo.modelData.safe
                    checked: panel.selected.indexOf(photo.index)>=0
                    Accessible.name: host.tr("Select photo")+": "+photo.modelData.name
                    width: 30; height: 30; padding: 0
                    indicator: Rectangle {
                        width: 26; height: 26; radius: 7
                        color: photoCheck.checked ? host.theme.accent : host.theme.canvas
                        border.width: 2; border.color: photoCheck.activeFocus ? host.theme.text : host.theme.accent
                        Text { anchors.centerIn: parent; text: photoCheck.checked ? "✓" : ""; color: host.theme.onAccent; font.bold: true; font.pixelSize: 17 }
                    }
                    onClicked: panel.toggle(photo.index)
                }
                ToolTip.visible: photoMouse.containsMouse && !panel.selectionMode
                ToolTip.text: photo.modelData.name + (photo.modelData.modified ? "\n"+panel.dateText(photo.modelData.modified) : "")
                ToolTip.delay: 600
            }
            Menu {
                id: photoMenu
                MenuItem { text: host.tr("Download"); enabled: !panel.viewState.busy && photo.modelData.safe; onTriggered: controller.send("photo_download",[photo.index]) }
                MenuItem { text: host.tr("Show location on OpenStreetMap"); enabled: Boolean(photo.modelData.location); onTriggered: panel.request({action:"map",index:photo.index}) }
                MenuItem { text: host.tr("Add to album"); enabled: !panel.viewState.busy; onTriggered: { panel.selectionMode=true; panel.selected=[photo.index]; panel.request({action:"albums"}); } }
            }
        }
        Label { anchors.centerIn: parent; visible: !panel.viewState.busy && grid.count===0; text: host.tr("No photos"); color: host.theme.text }
    }
    PhotoButton { visible: !panel.albumsVisible && Boolean(panel.viewState.photosMore); Layout.alignment: Qt.AlignHCenter; text: host.tr("Load more photos"); enabled: !panel.viewState.busy; onClicked: panel.request({action:"more"}) }
    Popup {
        id: viewer; objectName: "photoViewer"
        parent: Overlay.overlay; anchors.centerIn: parent
        width: parent ? parent.width-40 : 800; height: parent ? parent.height-40 : 600
        modal: true; focus: true; closePolicy: Popup.CloseOnEscape; padding: 20
        background: Rectangle { color: host.theme.canvas; radius: 18; border.color: host.theme.cardBorder }
        onClosed: panel.previewZoom=1
        contentItem: ColumnLayout {
            spacing: 14; focus: true
            Keys.onLeftPressed: if(panel.viewerIndex>0)panel.showPhoto(panel.viewerIndex-1)
            Keys.onRightPressed: if(panel.viewerIndex<(panel.viewState.photos || []).length-1)panel.showPhoto(panel.viewerIndex+1)
            RowLayout {
                ColumnLayout {
                    Layout.fillWidth: true
                    Label { text: panel.currentPhoto.name || ""; textFormat: Text.PlainText; font.bold: true; font.pixelSize: 18; elide: Text.ElideMiddle; color: host.theme.text; Layout.fillWidth: true }
                    Label { text: panel.dateText(panel.currentPhoto.modified); color: host.theme.muted }
                }
                PhotoButton { text: host.tr("Download"); enabled: !panel.viewState.busy && Boolean(panel.currentPhoto.safe); onClicked: { let index=panel.viewerIndex; viewer.close(); controller.send("photo_download",[index]); } }
                PhotoButton { text: host.tr("Map"); visible: Boolean(panel.currentPhoto.location); onClicked: panel.request({action:"map",index:panel.viewerIndex}) }
                PhotoButton { text: "×"; Accessible.name: host.tr("Close preview"); onClicked: viewer.close() }
            }
            Rectangle {
                Layout.fillWidth: true; Layout.fillHeight: true; radius: 12; color: host.theme.sidebar; clip: true
                Flickable {
                    id: previewViewport
                    anchors.fill: parent; clip: true
                    contentWidth: width*panel.previewZoom; contentHeight: height*panel.previewZoom
                    boundsBehavior: Flickable.StopAtBounds
                    Image { width: previewViewport.contentWidth; height: previewViewport.contentHeight; source: panel.currentPhoto.thumbnail || ""; fillMode: Image.PreserveAspectFit; cache: false; asynchronous: true }
                }
                Label { anchors.centerIn: parent; visible: !panel.currentPhoto.thumbnail; text: panel.viewState.busy ? host.tr("Loading preview…") : host.tr("Preview unavailable. Download the original to view it."); color: host.theme.muted; width: parent.width-40; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap }
            }
            RowLayout {
                PhotoButton { text: "‹"; Accessible.name: host.tr("Previous"); enabled: panel.viewerIndex>0 && !panel.viewState.busy; onClicked: panel.showPhoto(panel.viewerIndex-1) }
                Label { text: (panel.viewerIndex+1)+" / "+(panel.viewState.photos || []).length; color: host.theme.text }
                PhotoButton { text: "›"; Accessible.name: host.tr("Next"); enabled: panel.viewerIndex<(panel.viewState.photos || []).length-1 && !panel.viewState.busy; onClicked: panel.showPhoto(panel.viewerIndex+1) }
                Item { Layout.fillWidth: true }
                Label { text: host.tr("Preview"); color: host.theme.muted }
                PhotoButton { text: "−"; Accessible.name: host.tr("Zoom out"); enabled: panel.previewZoom>1; onClicked: panel.previewZoom=Math.max(1,panel.previewZoom-0.5) }
                Label { text: Math.round(panel.previewZoom*100)+"%"; color: host.theme.text }
                PhotoButton { text: "+"; Accessible.name: host.tr("Zoom in"); enabled: panel.previewZoom<4; onClicked: panel.previewZoom=Math.min(4,panel.previewZoom+0.5) }
            }
        }
    }
    Timer {
        interval: 300; repeat: true; running: viewer.visible
        onTriggered: if(!panel.viewState.busy && panel.viewerIndex>=0 && !panel.currentPhoto.previewLoaded && panel.viewerAttempt!==panel.viewerIndex) {
            panel.viewerAttempt=panel.viewerIndex;
            panel.request({action:"previews",start:panel.viewerIndex});
        }
    }
    Timer {
        id: loadTimer; interval: 300; repeat: true; running: panel.visible && !panel.albumsVisible && !viewer.visible
        onTriggered: {
            if(panel.viewState.busy || panel.albumsVisible || panel.restoring || viewer.visible)return;
            let first=Math.max(0,Math.floor(grid.contentY/grid.cellHeight)*Math.max(1,Math.floor(grid.width/panel.tileSize)));
            if(first<grid.count && !panel.viewState.photos[first].previewLoaded && panel.previewAttempt!==first) { panel.previewAttempt=first; panel.request({action:"previews",start:first}); return; }
            if(panel.viewState.photosMore && panel.moreAttempt!==grid.count && grid.contentY+grid.height>=grid.contentHeight-grid.cellHeight) { panel.moreAttempt=grid.count; panel.request({action:"more"}); }
        }
    }
}
