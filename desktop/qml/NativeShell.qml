import QtQuick
import NeutronDrive.Native
// Thin QML adapter: the existing view calls Rust controllers through these names.
Item {
    id: controller
    property var view: null
    property bool initialized: false
    property bool allowClose: false
    NativeStartup {id: startup}
    readonly property var launch: JSON.parse(startup.configJson)
    property string catalog: launch.catalog || ""
    property bool fixture: Qt.application.arguments[1] === "--check-window-fixture"
    property bool smoke: Qt.application.arguments[1] === "--check-window" || fixture
    readonly property var state: JSON.parse(files.stateJson)
    readonly property var fileEntries: JSON.parse(files.filesJson)
    readonly property var appearance: appearanceObject
    readonly property var updates: updateAdapter
    readonly property string language: preferences.language
    readonly property string languagePreference: preferences.languagePreference
    readonly property var languageOptions: JSON.parse(preferences.languageOptionsJson)
    signal dialogRequested(var value)
    signal ready()
    NativeFiles { id: files }
    NativePreferences { id: preferences }
    NativeAppearance {
        id: appearanceObject
        Component.onCompleted: setQtFallback(Qt.styleHints.colorScheme === Qt.Dark)
    }
    Connections { target: Qt.styleHints; function onColorSchemeChanged(){appearanceObject.setQtFallback(Qt.styleHints.colorScheme === Qt.Dark)} }
    NativeUpdates { id: updater }
    QtObject {
        id: updateAdapter
        readonly property var state: JSON.parse(updater.stateJson)
        readonly property bool automatic: Boolean(state.automatic)
        readonly property bool autoDownload: Boolean(state.autoDownload)
        readonly property bool previews: Boolean(state.previews)
        readonly property bool busy: Boolean(state.busy)
        readonly property string status: state.status || ""
        readonly property string packageFormat: state.packageFormat || "other"
        readonly property bool available: Boolean(state.available)
        readonly property bool downloadable: Boolean(state.downloadable)
        readonly property bool ready: Boolean(state.ready)
        function setOption(name,value){updater.setOption(name,value)}
        function checkNow(){updater.dispatch("check",controller.state.busy)}
        function downloadUpdate(){updater.dispatch("download",controller.state.busy)}
        function installUpdate(){updater.dispatch("install",controller.state.busy)}
        function openRelease(){updater.dispatch("openRelease",false)}
    }
    Connections { target: updater; function onNotice(message){files.notify(message,false)} }
    Connections {
        target: files
        function onDialogRequested(value){controller.dialogRequested(JSON.parse(value))}
        function onWindowAction(action){
            if(controller.smoke)console.log("NATIVE_WINDOW_ACTION",action);
            if(action === "hide"){controller.view.hide();return}
            if(action === "close"){
                Qt.callLater(function(){
                    if(!updater.shutdown())return;
                    serviceTimer.stop();initialUpdate.stop();updateTimer.stop();
                    files.shutdown();appearanceObject.shutdown();startup.shutdown();
                    controller.allowClose=true;
                    controller.view.close();
                    Qt.quit();
                });
            }
        }
    }
    function send(method,args){files.dispatch(method,JSON.stringify(args))}
    function translate(source){return preferences.translate(source)}
    function edit(name,value){send("edit",[name,value])}
    function show_page(page){send("show_page",[page])}
    function navigate(title,path){send("navigate",[title,path])}
    function crumb(index){send("crumb",[index])}
    function select_row(index,extend){send("select_row",[index,extend])}
    function move_selection(delta,extend){send("move_selection",[delta,extend])}
    function select_all(){send("select_all",[])}
    function row_activated(index){send("row_activated",[index])}
    function job_action(action,id){send("job_action",[action,id])}
    function restore_entry(index){send("restore_entry",[index])}
    function action(name){send("action",[name])}
    function answer_dialog(token,accepted,value){send("answer_dialog",[token,accepted,value])}
    function request_close(){if(smoke)console.log("NATIVE_WINDOW_CLOSE_REQUEST",allowClose);if(allowClose)return true;send("request_close",[updateAdapter.state.installing===true]);return false}
    function setLanguagePreference(value){if(!preferences.setLanguagePreference(value))files.notify(preferences.error,true)}
    function setAppearancePreference(value){if(preferences.setAppearancePreference(value))appearanceObject.setMode(preferences.appearance);else files.notify(preferences.error,true)}
    function stop(){if(!updater.shutdown())return false;files.shutdown();appearanceObject.shutdown();startup.shutdown();return true}
    Component.onCompleted: Qt.callLater(function(){
        try {
            let claimed=startup.claim();
            if(claimed===0){Qt.quit();return}
            if(claimed<0)throw new Error(startup.error);
            if(!preferences.initialize(catalog))throw new Error(preferences.error);
            startup.installTranslations(preferences.language);
            if(!files.configure(catalog,preferences.language,launch.executable,launch.demo))throw new Error("Native files initialization failed");
            if(!updater.configure(catalog,preferences.language,launch.payload,launch.demo || fixture))throw new Error("Native updater initialization failed");
            appearanceObject.setMode(preferences.appearance);
            let component=Qt.createComponent("qrc:/neutrondrive/ui/Main.qml");
            if(component.status!==Component.Ready)throw new Error(component.errorString());
            // A Window parented to this non-visual root Item would wait for a
            // parent window that does not exist. Keep it as a real top-level window.
            view=component.createObject(null);
            if(!view)throw new Error(component.errorString());
            initialized=true;
            if(launch.demo || launch.available){action("refresh")}else{show_page("settings");files.notify(translate("Das Proton-SDK-Backend wurde nicht gefunden."),true)}
            ready();
        }catch(error){console.error(error);stop();Qt.exit(1)}
    })
    Timer {interval:100;running:controller.initialized;repeat:true;onTriggered:{if(startup.activationPending()){controller.view.show();controller.view.raise();controller.view.requestActivate()}}}
    Timer {id:serviceTimer;interval:3000;running:controller.initialized;repeat:true;onTriggered:controller.send("poll_services",[])}
    Timer {id:initialUpdate;interval:10000;running:controller.initialized;onTriggered:updater.tick()}
    Timer {id:updateTimer;interval:3600000;running:controller.initialized;repeat:true;onTriggered:updater.tick()}
    // Isolated end-to-end checks of the actual existing Main.qml, without an account.
    property int stage: 0
    property var lastDialog: ({})
    property var checkedList: null
    property var checkedRow: null
    property int checkedFolder: -1
    function childNamed(item,name){
        if(!item)return null;
        if(item.objectName===name)return item;
        for(let child of (item.children || [])){let found=childNamed(child,name);if(found)return found}
        return null;
    }
    function dialogChild(name){
        for(let item of view.contentData){
            if(item.objectName==="confirmationDialog")return childNamed(item.contentItem,name);
        }
        return null;
    }
    onDialogRequested: function(value){lastDialog=value}
    property int captureFrames: 0
    function capture(name,next){
        if(fixture || Qt.application.arguments.length<4 || !Qt.application.arguments[3]){stage=next;return}
        // Allow the 100 ms theme transition and layout to settle before capture.
        if(captureFrames++<3)return;
        captureFrames=0;
        stage=-1;
        if(!files.snapshotWindow("mainWindow",Qt.application.arguments[3]+"/"+name)){console.error("Window capture failed");stop();Qt.exit(1);return}
        stage=next;
    }
    Timer {
        interval:50;running:controller.smoke && controller.initialized;repeat:true
        onTriggered:{
            try{
                if(controller.state.busy)return;
                if(controller.stage===0){
                    if(!controller.view.visible)throw new Error("Main window is not visible");
                    if(controller.fileEntries.length===0)throw new Error("Demo files missing");
                    if(Qt.application.arguments.length>5){
                        if(controller.language!==Qt.application.arguments[4])throw new Error("Unexpected system language: "+controller.language);
                        if(controller.view.titles.files!==Qt.application.arguments[5])throw new Error("Main window title translation mismatch");
                        console.log("NATIVE_WINDOW_LANGUAGE_OK",controller.language);
                    }
                    controller.setAppearancePreference("dark");
                    if(!controller.view.darkMode)throw new Error("Dark theme did not reach main window");
                    controller.show_page("settings");controller.stage=1;
                }else if(controller.stage===1){
                    controller.capture("dark-settings.png",11);
                }else if(controller.stage===11){
                    controller.setAppearancePreference("light");
                    if(controller.view.darkMode)throw new Error("Light theme did not reach main window");
                    controller.show_page("background");controller.stage=2;
                }else if(controller.stage===2){
                    if(!(controller.childNamed(controller.view.contentItem,"page-background").contentHeight>0))throw new Error("Background page cannot scroll");
                    controller.capture("light-background.png",2001);
                }else if(controller.stage===2001){
                    controller.show_page("ondemand");controller.stage=3;
                }else if(controller.stage===3){
                    if(!(controller.childNamed(controller.view.contentItem,"page-ondemand").contentHeight>0))throw new Error("Online page cannot scroll");
                    controller.capture("light-ondemand.png",3001);
                }else if(controller.stage===3001){
                    controller.show_page("local-trash");controller.stage=4;
                }else if(controller.stage===4){controller.show_page("activity");controller.stage=5;
                }else if(controller.stage===5){controller.show_page("files");controller.stage=15;
                }else if(controller.stage===15){controller.capture("light-files.png",16);
                }else if(controller.stage===16){
                    controller.checkedList=controller.childNamed(controller.view.contentItem,"fileList");
                    if(!controller.checkedList)throw new Error("File list missing");
                    controller.checkedList.forceActiveFocus();
                    if(!files.windowKey(Qt.Key_Down,false))throw new Error("Keyboard event failed");
                    controller.stage=17;
                }else if(controller.stage===17){
                    if(!controller.checkedList.activeFocus || JSON.stringify(controller.state.selection)!=="[1]")throw new Error("Keyboard selection lost focus or selected wrong row");
                    controller.setAppearancePreference("system");
                    appearanceObject.setQtFallback(true);
                    if(!controller.view.darkMode || JSON.stringify(controller.state.selection)!=="[1]")throw new Error("Live dark theme lost selection");
                    appearanceObject.setQtFallback(false);
                    if(controller.view.darkMode || JSON.stringify(controller.state.selection)!=="[1]")throw new Error("Live light theme lost selection");
                    files.windowKey(Qt.Key_A,true);controller.stage=18;
                }else if(controller.stage===18){
                    if(controller.state.selection.length!==controller.fileEntries.length || !controller.state.actions.Download.enabled || controller.state.actions["Umbenennen"].enabled)throw new Error("Select-all action state incorrect");
                    controller.checkedFolder=controller.fileEntries.findIndex(entry=>entry.isFolder);
                    controller.checkedRow=controller.childNamed(controller.checkedList,"fileRow-"+controller.checkedFolder);
                    if(!controller.checkedRow)throw new Error("Folder delegate missing");
                    let point=controller.checkedRow.mapToItem(controller.view.contentItem,30,25);
                    files.windowClick(point.x,point.y,false);controller.stage=19;
                }else if(controller.stage===19){
                    if(JSON.stringify(controller.state.selection)!==JSON.stringify([controller.checkedFolder]))throw new Error("Mouse selection incorrect: "+JSON.stringify(controller.state.selection)+" expected "+controller.checkedFolder);
                    if(controller.childNamed(controller.checkedList,"fileRow-"+controller.checkedFolder)!==controller.checkedRow)throw new Error("Selection replaced file delegate");
                    let point=controller.checkedRow.mapToItem(controller.view.contentItem,30,25);
                    files.windowClick(point.x,point.y,true);controller.stage=20;
                }else if(controller.stage===20){
                    if(controller.state.path==="/my-files")throw new Error("Double-click did not open folder");
                    console.log("NATIVE_KEYBOARD_AND_MOUSE_OK");
                    controller.navigate(controller.translate("Meine Dateien"),"/my-files");controller.stage=201;
                }else if(controller.stage===201){
                    controller.select_row(0,false);
                    controller.childNamed(controller.view.contentItem,"gridViewButton").clicked();controller.stage=202;
                }else if(controller.stage===202){
                    controller.checkedList=controller.childNamed(controller.view.contentItem,"fileGrid");
                    if(!controller.checkedList || !controller.checkedList.visible || JSON.stringify(controller.state.selection)!=="[0]")throw new Error("Grid switch lost selection");
                    controller.checkedList.forceActiveFocus();
                    files.windowKey(Qt.Key_Right,false);controller.stage=203;
                }else if(controller.stage===203){
                    if(JSON.stringify(controller.state.selection)!=="[1]")throw new Error("Grid arrow navigation failed");
                    files.windowKey(Qt.Key_A,true);controller.stage=204;
                }else if(controller.stage===204){
                    if(controller.state.selection.length!==controller.fileEntries.length)throw new Error("Grid select-all failed");
                    controller.checkedFolder=controller.fileEntries.findIndex(entry=>entry.isFolder);
                    controller.checkedRow=controller.childNamed(controller.checkedList,"fileTile-"+controller.checkedFolder);
                    if(!controller.checkedRow)throw new Error("Grid folder delegate missing");
                    let point=controller.checkedRow.mapToItem(controller.view.contentItem,40,40);
                    files.windowClick(point.x,point.y,false);controller.stage=205;
                }else if(controller.stage===205){
                    if(JSON.stringify(controller.state.selection)!==JSON.stringify([controller.checkedFolder]))throw new Error("Grid mouse selection failed");
                    controller.capture("light-grid.png",206);
                }else if(controller.stage===206){
                    controller.setAppearancePreference("dark");controller.stage=2061;
                }else if(controller.stage===2061){
                    controller.stage=20611;
                }else if(controller.stage===20611){
                    controller.capture("dark-grid.png",2062);
                }else if(controller.stage===2062){
                    controller.setAppearancePreference("light");
                    let point=controller.checkedRow.mapToItem(controller.view.contentItem,40,40);
                    files.windowClick(point.x,point.y,true);controller.stage=207;
                }else if(controller.stage===207){
                    if(controller.state.path==="/my-files")throw new Error("Grid double-click did not open folder");
                    controller.navigate(controller.translate("Meine Dateien"),"/my-files");
                    controller.childNamed(controller.view.contentItem,"listViewButton").clicked();
                    console.log("NATIVE_GRID_KEYBOARD_AND_MOUSE_OK");controller.stage=21;
                }else if(controller.stage===21){
                    if(controller.fixture){
                        controller.select_row(0,false);controller.action("Umbenennen");controller.stage=22;
                    }else{controller.action("chooseBackground");controller.stage=6}
                }else if(controller.stage===22){
                    let prompt=controller.dialogChild("promptInput");
                    if(controller.lastDialog.kind!=="prompt" || !prompt || prompt.text!==controller.fileEntries[0].name)throw new Error("Prompt did not preserve original filename");
                    prompt.text="Meine Dateien {p0} 中文.txt";
                    controller.dialogChild("dialogAccept").clicked();controller.stage=23;
                }else if(controller.stage===23){
                    if(controller.state.noticeError)throw new Error("Synthetic rename failed");
                    controller.select_row(0,false);controller.action("Freigabe");controller.stage=24;
                }else if(controller.stage===24){
                    if(controller.lastDialog.kind!=="share")throw new Error("Share dialog missing");
                    controller.dialogChild("copyShareLink").clicked();controller.stage=25;
                }else if(controller.stage===25){
                    if(files.clipboardText()!=="https://drive.proton.me/urls/synthetic?literal=%3Cb%3E")throw new Error("Clipboard value changed");
                    console.log("NATIVE_PROMPT_AND_CLIPBOARD_OK");
                    controller.action("chooseBackground");controller.stage=6;
                }else if(controller.stage===6){
                    if(controller.lastDialog.kind!=="folder")throw new Error("Folder dialog missing");
                    for(let child of controller.view.contentData){if(child.objectName==="folderPicker")child.close()}
                    controller.answer_dialog(controller.lastDialog.token,false,[]);
                    console.log("NATIVE_MAIN_WINDOW_OK");console.log("NATIVE_WINDOW_CLOSE_RESULT",controller.view.close());controller.stage=7;
                }
            }catch(error){console.error(error);controller.stop();Qt.exit(1)}
        }
    }
    Timer{interval:15000;running:controller.smoke;onTriggered:{controller.stop();Qt.exit(2)}}
}
