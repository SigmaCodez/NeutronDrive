import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: panel
    required property var host
    required property var controller
    required property var viewState
    readonly property var photos: controller.photoEntries || []
    property var appliedPhotos: []
    property bool pendingPhotos: false
    property bool scrolling: grid.moving
    onScrollingChanged: if(!scrolling && pendingPhotos)updatePhotos()
    ListModel { id: photoModel; dynamicRoles: true }
    function updatePhotos() {
        if(!photoModel)return;
        let items=photos;
        let replaces=items.length<photoModel.count;
        let revoked=[];
        for(let i=0;i<Math.min(appliedPhotos.length,items.length);i++) {
            if(appliedPhotos[i].uid!==items[i].uid)replaces=true;
            if(appliedPhotos[i].safe===true && items[i].safe!==true)revoked.push(i);
        }
        if(replaces) {
            let positions=new Map(items.map((p,i)=>[p.uid,i]));
            selected=selected.map(i=>positions.get(appliedPhotos[i]?.uid)).filter(i=>i!==undefined && items[i].safe===true);
            let next=positions.get(appliedPhotos[viewerIndex]?.uid);
            if(next===undefined) { viewer.close(); viewerIndex=-1; } else viewerIndex=next;
            pendingViewerIndex=-1; previewAttempt=-1; moreAttempt=-1;
        }
        // Access changes take effect immediately, including during a flick.
        // Keep valid selections but never show a revoked tile as actionable.
        let available=selected.filter(index=>items[index]?.safe===true);
        if(available.length!==selected.length)selected=available;
        if(viewer.visible && pendingViewerIndex>=0 && items.length>pendingViewerIndex) {
            let next=pendingViewerIndex; pendingViewerIndex=-1; showPhoto(next);
        }
        if(scrolling && photoModel.count>0 && !replaces) {
            for(let index of revoked)photoModel.setProperty(index,"entry",items[index]);
            // Append new pages immediately so reaching the bottom never requires
            // releasing the wheel/touchpad. Existing tiles update after movement.
            let added=[];
            for(let i=photoModel.count;i<items.length;i++) { photoModel.append({entry:items[i]}); added.push(items[i]); }
            if(added.length)appliedPhotos=appliedPhotos.concat(added);
            pendingPhotos=true; return;
        }
        pendingPhotos=false;
        // Keep delegates and scroll position alive across status updates and pages.
        let wanted=new Set(items.map(p=>p.uid));
        for(let i=photoModel.count-1;i>=0;i--)if(!wanted.has(photoModel.get(i).entry.uid))photoModel.remove(i);
        for(let i=0;i<items.length;i++) {
            let next=items[i];
            if(i>=photoModel.count) { photoModel.append({entry:next}); continue; }
            if(photoModel.get(i).entry.uid!==next.uid) {
                let found=-1;
                for(let j=i+1;j<photoModel.count;j++)if(photoModel.get(j).entry.uid===next.uid) { found=j; break; }
                if(found<0)photoModel.insert(i,{entry:next}); else photoModel.move(found,i,1);
            }
            let old=photoModel.get(i).entry;
            if(old.thumbnail!==next.thumbnail || old.previewLoaded!==next.previewLoaded || old.name!==next.name || old.safe!==next.safe || old.modified!==next.modified || JSON.stringify(old.location)!==JSON.stringify(next.location))photoModel.setProperty(i,"entry",next);
        }
        appliedPhotos=items;
    }
    onPhotosChanged: updatePhotos()
    onVisibleChanged: {
        if(!visible) { viewer.close(); grid.focus=false; }
        else { previewRetryAt=0; retryAt=0; }
    }
    Component.onCompleted: updatePhotos()
    property bool albumsVisible: false
    property var selected: []
    property bool selectionMode: false
    property int tileSize: 200
    property int pendingViewerIndex: -1
    property int viewerIndex: -1
    property real previewZoom: 1
    property int previewAttempt: -1
    property double previewRetryAt: 0
    property int moreAttempt: -1
    property double retryAt: 0
    property int viewerAttempt: -1
    property double viewerRetryAt: 0
    readonly property var currentPhoto: viewerIndex >= 0 ? photos[viewerIndex] || ({}) : ({})
    function showPhoto(index) {
        if(index<0 || index>=photos.length)return;
        viewerIndex=index; viewerAttempt=-1; previewZoom=1;
        previewViewport.contentX=0; previewViewport.contentY=0;
        viewer.open();
    }
    function movePreview(delta) {
        if(delta>0 && viewerIndex===photos.length-1 && viewState.photosMore) {
            pendingViewerIndex=photos.length;
            loadViewerPreview();
        } else { pendingViewerIndex=-1; showPhoto(viewerIndex+delta); }
    }
    function zoomPreview(delta) { previewZoom=Math.max(1,Math.min(4,previewZoom+delta)); }
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
    component PhotoButton: DriveButton { theme: host.theme }
    component PhotoMenu: DriveMenu { theme: host.theme }
    component PhotoMenuItem: DriveMenuItem { theme: host.theme }
    function request(value) { controller.send("photo_library", [value]); }
    function openAlbum(uid) {
        leaveSelection(); viewer.close(); viewerIndex=-1; previewAttempt=-1; moreAttempt=-1; savedScroll=0; albumsVisible=false;
        grid.contentY=0;
        request({action:"index",album:uid});
    }
    function toggle(index) {
        if(!selectionMode || !photos[index]?.safe)return;
        let items=selected.slice(); let at=items.indexOf(index);
        if(at>=0)items.splice(at,1);else if(items.length<48)items.push(index);
        selected=items;
    }
    property bool displayOptions: false
    function selectedAlbum() { return (panel.viewState.photoAlbums || [])[albumIndex]; }
    function changeAlbum(action) {
        let album=action==="remove" ? (viewState.photoAlbums || []).find(a=>a.uid===viewState.photoAlbum) : selectedAlbum();
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
        PhotoButton { text: host.tr("Refresh"); enabled: !panel.viewState.busy; onClicked: panel.albumsVisible ? panel.request({action:"albums"}) : panel.openAlbum(panel.viewState.photoAlbum || "") }
    }
    RowLayout {
        Layout.fillWidth: true; spacing: 8
        Label { text: panel.viewState.photoTotal===null ? "" : host.tr("Total photos")+": "+panel.viewState.photoTotal; color: host.theme.muted; Layout.fillWidth: true }
        PhotoButton {
            objectName: "takeoutImportButton"; text: host.tr("Import photos")
            enabled: !panel.viewState.busy && Boolean(panel.viewState.authenticated) && !panel.viewState.demo
            onClicked: importMenu.popup()
            ToolTip.visible: hovered; ToolTip.text: host.tr("Import a photo folder or an extracted Google Photos export.")
            Accessible.description: host.tr("Photo import")
            PhotoMenu {
                id: importMenu
                PhotoMenuItem { text: host.tr("Import a folder"); onTriggered: panel.request({action:"import_folder"}) }
                PhotoMenuItem { text: host.tr("Google Takeout import"); onTriggered: panel.request({action:"takeout"}) }
            }
        }
        PhotoButton { objectName: "photoDisplayButton"; visible: !panel.albumsVisible; text: host.tr("View options"); highlighted: panel.displayOptions; onClicked: panel.displayOptions = !panel.displayOptions }
    }
    RowLayout {
        objectName: "photoDisplayOptions"; visible: !panel.albumsVisible && (panel.displayOptions || Boolean(panel.viewState.photoAlbum)); spacing: 12
        Label { text: panel.viewState.photoAlbum ? ((panel.viewState.photoAlbums || []).find(a=>a.uid===panel.viewState.photoAlbum)?.name || host.tr("Albums")) : host.tr("Your photo library"); color: host.theme.muted; Layout.fillWidth: true; elide: Text.ElideRight }
        Label { visible: panel.displayOptions; text: host.tr("Thumbnail size"); color: host.theme.muted }
        Slider {
            id: sizeSlider; visible: panel.displayOptions; from: 150; to: 300; value: panel.tileSize; stepSize: 25
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
        PhotoButton { text: host.tr("Select up to 48"); enabled: !panel.viewState.busy; onClicked: panel.selected=panel.photos.map((p,i)=>p.safe ? i : -1).filter(i=>i>=0).slice(0,48) }
        PhotoButton { text: host.tr("Clear selection"); enabled: panel.selected.length>0; onClicked: panel.selected=[] }
        DriveCombo {
            id: albumChoice; theme: host.theme; width: 200
            model: panel.viewState.photoAlbums || []; textRole: "name"; currentIndex: panel.albumIndex; onActivated: panel.albumIndex=currentIndex
            Accessible.name: host.tr("Albums")
        }
        PhotoButton { text: host.tr("Add to album"); enabled: !panel.viewState.busy && panel.selected.length>0 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("add") }
        PhotoButton { text: host.tr("Remove from album"); enabled: !panel.viewState.busy && panel.selected.length>0 && Boolean(panel.viewState.photoAlbum); onClicked: panel.changeAlbum("remove") }
        PhotoButton { text: host.tr("Set album cover"); enabled: !panel.viewState.busy && panel.selected.length===1 && Boolean(panel.selectedAlbum()); onClicked: panel.changeAlbum("cover") }
    }
    GridView {
        id: albums; objectName: "photoAlbumsGrid"
        visible: panel.albumsVisible; Layout.fillWidth: true; Layout.fillHeight: true
        clip: true; model: panel.viewState.photoAlbums || []; cellWidth: width/Math.max(1,Math.floor(width/240)); cellHeight: 240
        ScrollBar.vertical: DriveScrollBar { theme: host.theme }
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
                        source: panel.photos.find(p=>p.uid===albumTile.modelData.cover)?.thumbnail || ""
                        fillMode: Image.PreserveAspectCrop; asynchronous: true; cache: true
                    }
                    DriveIcon { anchors.centerIn: parent; symbol: "photo"; ink: host.theme.accentText; visible: albumCover.status!==Image.Ready }
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; enabled: !panel.viewState.busy && albumTile.modelData.safe; onClicked: panel.openAlbum(albumTile.modelData.uid) }
                }
                RowLayout {
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 3
                        Label { text: albumTile.modelData.name || host.tr("Album"); textFormat: Text.PlainText; color: host.theme.text; font.bold: true; elide: Text.ElideRight; Layout.fillWidth: true }
                        Label { text: host.tr("Photos")+": "+albumTile.modelData.count; color: host.theme.muted }
                    }
                    PhotoButton { text: "⋯"; Accessible.name: host.tr("Album actions"); enabled: !panel.viewState.busy && albumTile.modelData.safe; onClicked: albumMenu.popup() }
                }
            }
            PhotoMenu {
                id: albumMenu
                PhotoMenuItem { text: host.tr("Open"); onTriggered: panel.openAlbum(modelData.uid) }
                PhotoMenuItem { text: host.tr("Rename album"); onTriggered: panel.request({action:"rename",album:modelData.uid}) }
                PhotoMenuItem { text: host.tr("Delete album"); onTriggered: panel.request({action:"delete",album:modelData.uid}) }
            }
        }
        Label { anchors.centerIn: parent; visible: !panel.viewState.busy && albums.count===0; text: host.tr("No albums yet"); color: host.theme.text }
    }
    GridView {
        id: grid
        objectName: "photoGrid"; visible: !panel.albumsVisible
        Layout.fillWidth: true; Layout.fillHeight: true
        clip: true; model: photoModel; reuseItems: true; cellWidth: width/Math.max(1,Math.floor(width/panel.tileSize)); cellHeight: cellWidth
        cacheBuffer: grid.height
        activeFocusOnTab: true; keyNavigationEnabled: true
        Keys.onReturnPressed: if(currentIndex>=0)panel.showPhoto(currentIndex)
        Keys.onSpacePressed: if(panel.selectionMode && currentIndex>=0)panel.toggle(currentIndex)
        ScrollBar.vertical: DriveScrollBar { theme: host.theme }
        onContentYChanged: if(!panel.restoring) { panel.savedScroll=Math.max(0,contentY); }
        delegate: Item {
            id: photo; required property var entry; required property int index
            width: grid.cellWidth; height: grid.cellHeight
            Rectangle {
                anchors.fill: parent; anchors.margins: 2
                color: host.theme.fileTile; radius: 6; clip: true
                DriveIcon { anchors.centerIn: parent; symbol: "photo"; ink: host.theme.fileInk; visible: preview.status!==Image.Ready }
                Image {
                    id: preview; objectName: "photoPreview-"+photo.index
                    anchors.fill: parent; anchors.margins: panel.selected.indexOf(photo.index)>=0 ? 4 : 0
                    source: photo.entry.thumbnail || ""; sourceSize: Qt.size(Math.ceil(grid.cellWidth*Screen.devicePixelRatio), Math.ceil(grid.cellWidth*Screen.devicePixelRatio))
                    fillMode: Image.PreserveAspectCrop; asynchronous: true; cache: true
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
                Rectangle { anchors.fill: parent; color: "transparent"; radius: 12; border.width: panel.selected.indexOf(photo.index)>=0 ? 3 : (grid.activeFocus && grid.currentIndex===photo.index) ? 2 : photoMouse.containsMouse ? 1 : 0; border.color: host.theme.accent }
                CheckBox {
                    id: photoCheck; objectName: "photoCheck-"+photo.index
                    anchors { top: parent.top; left: parent.left; margins: 10 }
                    visible: panel.selectionMode; enabled: photo.entry.safe
                    checked: panel.selected.indexOf(photo.index)>=0
                    Accessible.name: host.tr("Select photo")+": "+photo.entry.name
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
                ToolTip.text: photo.entry.name + (photo.entry.modified ? "\n"+panel.dateText(photo.entry.modified) : "")
                ToolTip.delay: 600
            }
            PhotoMenu {
                id: photoMenu
                PhotoMenuItem { text: host.tr("Download"); enabled: !panel.viewState.busy && photo.entry.safe; onTriggered: controller.send("photo_download",[photo.index]) }
                PhotoMenuItem { text: host.tr("Show location on OpenStreetMap"); enabled: Boolean(photo.entry.location); onTriggered: panel.request({action:"map",index:photo.index}) }
                PhotoMenuItem { text: host.tr("Add to album"); enabled: !panel.viewState.busy; onTriggered: { panel.selectionMode=true; panel.selected=[photo.index]; panel.request({action:"albums"}); } }
            }
        }
        Label { anchors.centerIn: parent; visible: !panel.viewState.busy && grid.count===0; text: host.tr("No photos"); color: host.theme.text }
    }
    Popup {
        id: viewer; objectName: "photoViewer"
        parent: Overlay.overlay; anchors.centerIn: parent
        width: parent ? parent.width-40 : 800; height: parent ? parent.height-40 : 600
        modal: true; focus: true; closePolicy: Popup.CloseOnEscape; padding: 20
        background: Rectangle { color: host.theme.canvas; radius: 18; border.color: host.theme.cardBorder }
        onClosed: {
            if(panel.viewerIndex>=0 && panel.viewerIndex<grid.count) {
                grid.currentIndex=panel.viewerIndex;
                grid.positionViewAtIndex(panel.viewerIndex,GridView.Contain);
                grid.forceActiveFocus();
            }
            panel.previewZoom=1; panel.viewerIndex=-1; panel.pendingViewerIndex=-1;
        }
        contentItem: ColumnLayout {
            spacing: 14; focus: true
            // Shortcuts keep working when a viewer toolbar button has keyboard focus.
            Shortcut { sequence: "Left"; enabled: viewer.visible; onActivated: panel.movePreview(-1) }
            Shortcut { sequence: "Right"; enabled: viewer.visible; onActivated: panel.movePreview(1) }
            Shortcut { sequence: "Home"; enabled: viewer.visible; onActivated: panel.showPhoto(0) }
            Shortcut { sequence: "End"; enabled: viewer.visible; onActivated: panel.showPhoto(panel.photos.length-1) }
            Shortcut { sequences: ["+", "="]; enabled: viewer.visible; onActivated: panel.zoomPreview(0.5) }
            Shortcut { sequence: "-"; enabled: viewer.visible; onActivated: panel.zoomPreview(-0.5) }
            Shortcut { sequence: "0"; enabled: viewer.visible; onActivated: panel.previewZoom=1 }
            RowLayout {
                ColumnLayout {
                    Layout.fillWidth: true
                    Label { text: panel.currentPhoto.name || ""; textFormat: Text.PlainText; font.bold: true; font.pixelSize: 18; elide: Text.ElideMiddle; color: host.theme.text; Layout.fillWidth: true }
                    Label { text: panel.dateText(panel.currentPhoto.modified); color: host.theme.muted }
                }
                PhotoButton { text: host.tr("Download"); enabled: !panel.viewState.busy && Boolean(panel.currentPhoto.safe); onClicked: { let index=panel.viewerIndex; viewer.close(); controller.send("photo_download",[index]); } }
                PhotoButton { text: host.tr("Map"); visible: Boolean(panel.currentPhoto.location); onClicked: panel.request({action:"map",index:panel.viewerIndex}) }
                PhotoButton { objectName: "closePhotoPreview"; text: "×"; Accessible.name: host.tr("Close preview"); onClicked: viewer.close() }
            }
            Rectangle {
                Layout.fillWidth: true; Layout.fillHeight: true; radius: 12; color: host.theme.sidebar; clip: true
                Flickable {
                    id: previewViewport
                    anchors.fill: parent; clip: true
                    contentWidth: width*panel.previewZoom; contentHeight: height*panel.previewZoom
                    boundsBehavior: Flickable.StopAtBounds
                    Image { width: previewViewport.contentWidth; height: previewViewport.contentHeight; source: panel.currentPhoto.thumbnail || ""; fillMode: Image.PreserveAspectFit; cache: true; asynchronous: true }
                }
                Label { anchors.centerIn: parent; visible: !panel.currentPhoto.thumbnail; text: panel.viewState.busy ? host.tr("Loading preview…") : host.tr("Preview unavailable. Download the original to view it."); color: host.theme.muted; width: parent.width-40; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap }
            }
            RowLayout {
                PhotoButton { text: "‹"; Accessible.name: host.tr("Previous"); enabled: panel.viewerIndex>0; onClicked: panel.movePreview(-1) }
                Label { text: (panel.viewerIndex+1)+" / "+panel.photos.length; color: host.theme.text }
                PhotoButton { text: "›"; Accessible.name: host.tr("Next"); enabled: !panel.viewState.busy && (panel.viewerIndex<panel.photos.length-1 || panel.viewState.photosMore); onClicked: panel.movePreview(1) }
                Item { Layout.fillWidth: true }
                Label { text: host.tr("Preview"); color: host.theme.muted }
                PhotoButton { text: "−"; Accessible.name: host.tr("Zoom out"); enabled: panel.previewZoom>1; onClicked: panel.zoomPreview(-0.5) }
                Label { text: Math.round(panel.previewZoom*100)+"%"; color: host.theme.text }
                PhotoButton { text: "+"; Accessible.name: host.tr("Zoom in"); enabled: panel.previewZoom<4; onClicked: panel.zoomPreview(0.5) }
            }
        }
    }
    // Failure retries have a cooldown; moving the viewport must not turn a
    // failed request into a tight loop. New visible ranges still load promptly.
    function loadViewerPreview() {
        if(!viewer.visible || panel.viewState.busy || panel.viewerIndex<0)return;
        if(panel.pendingViewerIndex>=0 && panel.viewState.photosMore) {
            let offset=Number.isFinite(panel.viewState.photoOffset) ? panel.viewState.photoOffset : panel.photos.length;
            if(panel.moreAttempt!==offset || Date.now()>=panel.retryAt) {
                panel.moreAttempt=offset; panel.retryAt=Date.now()+15000; panel.request({action:"more"});
            }
            return;
        }
        if(panel.currentPhoto.previewLoaded)return;
        if(panel.viewerAttempt===panel.viewerIndex && Date.now()<panel.viewerRetryAt)return;
        panel.viewerAttempt=panel.viewerIndex; panel.viewerRetryAt=Date.now()+15000;
        panel.request({action:"previews",start:panel.viewerIndex});
    }
    function loadVisiblePhotos() {
        if(!panel.visible || panel.viewState.busy || panel.albumsVisible || panel.restoring || viewer.visible)return;
        let columns=Math.max(1,Math.floor(grid.width/panel.tileSize));
        let first=Math.max(0,Math.floor(grid.contentY/grid.cellHeight)*columns);
        let end=Math.min(grid.count,panel.photos.length,first+columns*(Math.ceil(grid.height/grid.cellHeight)+2));
        let missing=-1;
        for(let i=first;i<end;i++)if(!panel.photos[i].previewLoaded){missing=i;break;}
        if(missing>=0 && (panel.previewAttempt!==missing || Date.now()>=panel.previewRetryAt)) {
            panel.previewAttempt=missing; panel.previewRetryAt=Date.now()+15000;
            panel.request({action:"previews",start:missing}); return;
        }
        // A batch can contain no accessible photos. Its inventory offset still
        // advances, so grid.count alone must not suppress the next page.
        let offset=Number.isFinite(panel.viewState.photoOffset) ? panel.viewState.photoOffset : grid.count;
        if(panel.viewState.photosMore && (panel.moreAttempt!==offset || Date.now()>=panel.retryAt)
                && grid.contentY+grid.height>=grid.contentHeight-grid.height*2) {
            panel.moreAttempt=offset; panel.retryAt=Date.now()+15000;
            panel.request({action:"more"});
        }
    }
    Timer {
        interval: 300; repeat: true; running: viewer.visible
        onTriggered: panel.loadViewerPreview()
    }
    Timer {
        id: loadTimer; interval: 120; repeat: true; running: panel.visible && !panel.albumsVisible && !viewer.visible
        onTriggered: panel.loadVisiblePhotos()
    }
}
