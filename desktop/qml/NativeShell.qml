import QtQuick
import Qt.labs.platform as Platform
import NeutronDrive.Native
// Thin QML adapter: the existing view calls Rust controllers through these names.
Item {
    id: controller
    property var view: null
    property bool initialized: false
    property bool allowClose: false
    property bool explicitQuit: false
    property bool trayAvailable: false
    function reveal(page) {
        if(page)show_page(page);
        view.show();view.raise();view.requestActivate();
    }
    Platform.SystemTrayIcon {
        id: tray
        visible: controller.initialized && !controller.smoke && !controller.launch.demo
        icon.source: "qrc:/neutrondrive/ui/tray.svg"
        tooltip: "NeutronDrive · " + (controller.state.fields ? controller.state.fields.background_status.value : "")
        onActivated: controller.reveal("")
        menu: Platform.Menu {
            Platform.MenuItem { text: controller.translate("NeutronDrive öffnen"); onTriggered: controller.reveal("") }
            Platform.MenuItem { text: controller.translate("Einstellungen"); onTriggered: controller.reveal("settings") }
            Platform.MenuItem { text: controller.translate("Online-Laufwerk"); onTriggered: controller.reveal("ondemand") }
            Platform.MenuSeparator {}
            Platform.MenuItem { text: controller.translate("Sync-Dienst starten"); enabled: !controller.state.busy && !controller.state.backgroundRunning; onTriggered: controller.action("start") }
            Platform.MenuItem { text: controller.translate("Sync-Dienst stoppen"); enabled: !controller.state.busy && Boolean(controller.state.backgroundRunning); onTriggered: controller.action("stop") }
            Platform.MenuSeparator {}
            Platform.MenuItem { text: controller.translate(controller.launch.flatpak ? "Fenster schließen" : "Fenster und Tray beenden"); onTriggered: { controller.explicitQuit=true;controller.view.close();controller.explicitQuit=false; } }
        }
    }
    NativeStartup {id: startup}
    readonly property var launch: JSON.parse(startup.configJson)
    property string catalog: launch.catalog || ""
    property bool fixture: Qt.application.arguments[1] === "--check-window-fixture"
    property bool smoke: Qt.application.arguments[1] === "--check-window" || fixture
    readonly property var photoEntries: JSON.parse(files.photosJson)
    readonly property var state: Object.assign(JSON.parse(files.stateJson), {photos: photoEntries})
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
        function installUpdate(){updater.dispatch("install",controller.state.busy || controller.view.dialogOpen)}
        function restartUpdate(){updater.dispatch("restart",controller.state.busy || controller.view.dialogOpen)}
        function openRelease(){updater.dispatch("openRelease",false)}
    }
    Connections {
        target: updater
        function onNotice(message){files.notify(message,false)}
        function onRestartRequested(){
            Qt.callLater(function(){
                if(!updater.shutdown())return;
                controller.initialized=false;
                serviceTimer.stop();initialUpdate.stop();updateTimer.stop();
                files.shutdown();appearanceObject.shutdown();startup.shutdown();
                controller.allowClose=true;
                controller.view.close();
                Qt.quit();
            });
        }
    }
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
    function send(method,args){if((updateAdapter.state.installing || updateAdapter.state.restartPending) && method!=="request_close")return;files.dispatch(method,JSON.stringify(args))}
    function handleFileAction(value) {
        // Availability changes stay in the background; only sharing needs a window.
        if(value.action === "share")view.openShareLoading();
        send("file_manager",[value]);
    }
    function translate(source){let activeLanguage=preferences.language;let ready=initialized;return preferences.translate(source)}
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
    function request_close(){if(smoke)console.log("NATIVE_WINDOW_CLOSE_REQUEST",allowClose);if(allowClose)return true;send("request_close",[updateAdapter.state.installing===true,!smoke && !launch.demo && startup.trayAvailable() && !explicitQuit]);return false}
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
            if(smoke)send("setup_later",[]);
            let component=Qt.createComponent("qrc:/neutrondrive/ui/Main.qml");
            if(component.status!==Component.Ready)throw new Error(component.errorString());
            // A Window parented to this non-visual root Item would wait for a
            // parent window that does not exist. Keep it as a real top-level window.
            view=component.createObject(null, (launch.tray || launch.fileAction) ? {visible:false} : {});
            if(!view)throw new Error(component.errorString());
            trayAvailable=startup.trayAvailable();
            initialized=true;
            if(launch.demo || launch.available){action("refresh")}else{show_page("settings");files.notify(translate("Das Proton-SDK-Backend wurde nicht gefunden."),true)}
            ready();
        }catch(error){console.error(error);stop();Qt.exit(1)}
    })
    Timer {interval:100;running:controller.initialized;repeat:true;onTriggered:{if(startup.activationPending()){controller.view.show();controller.view.raise();controller.view.requestActivate()}}}
    Timer { interval: 150; running: controller.initialized && !controller.smoke; repeat: true; onTriggered: {
        if(updateAdapter.state.installing || updateAdapter.state.restartPending || controller.state.busy || controller.view.dialogOpen)return;
        let request=startup.nextFileAction();
        if(request)controller.handleFileAction(JSON.parse(request));
    } }
    Timer {id:serviceTimer;interval:3000;running:controller.initialized;repeat:true;onTriggered:{let available=startup.trayAvailable();if(controller.trayAvailable && !available && controller.view && !controller.view.visible)controller.reveal("");controller.trayAvailable=available;controller.send("poll_services",[])}}
    // Some desktops publish their tray host after autostart applications.
    // Keep the icon registered and reveal a usable window if no host appears.
    Timer { interval: 15000; running: controller.initialized && controller.launch.tray === true; onTriggered: {
        if(!startup.trayAvailable() && !controller.view.visible)controller.reveal("");
    } }
    Timer {id:initialUpdate;interval:10000;running:controller.initialized;onTriggered:updater.tick()}
    Timer {id:updateTimer;interval:3600000;running:controller.initialized;repeat:true;onTriggered:updater.tick()}
    // Isolated end-to-end checks of the actual existing Main.qml, without an account.
    property int stage: 900
    property var checkedWizard: null
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
        if(name==="copyShareLink")return childNamed(view.sharingWindow.contentItem,name);
        for(let item of view.contentData){
            if(item.objectName==="confirmationDialog")return childNamed(item.contentItem,name);
        }
        return null;
    }
    onDialogRequested: function(value){lastDialog=value}
    property var photoBench: null
    property var photoBenchDelegate: null
    property real photoBenchScroll: 0
    property real photoBenchStarted: 0
    QtObject { id: photoBenchController; property var photoEntries: []; property string language: "en"; property var requests: []; function send(method,args){requests=requests.concat([args[0]])} }
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
                if(controller.stage===900){
                    controller.view.s=Object.assign({},controller.state,{setupNeeded:true,setupFolder:"/home/alex/ProtonDrive",authenticated:false,busy:false});
                    controller.checkedWizard=Array.from(controller.view.contentData).find(item=>item.objectName==="setupWizard");
                    if(!controller.checkedWizard)throw new Error("Setup wizard missing");
                    controller.view.width=900;controller.view.height=640;
                    controller.setAppearancePreference("light");controller.stage=901;
                }else if(controller.stage===901){
                    let wizard=controller.checkedWizard;
                    let next=controller.childNamed(wizard.contentItem,"setupNext");
                    let path=controller.childNamed(wizard.contentItem,"setupPath");
                    if(wizard.step!==0 || !wizard.visible || next.enabled || path.visible)throw new Error("Unauthenticated setup must show only sign-in and disable Next");
                    if(wizard.height>controller.view.height-30)throw new Error("Setup exceeds minimum window height");
                    controller.capture("setup-sign-in-light.png",902);
                }else if(controller.stage===902){
                    controller.setAppearancePreference("dark");controller.stage=903;
                }else if(controller.stage===903){controller.capture("setup-sign-in-dark.png",904);
                }else if(controller.stage===904){
                    controller.view.s=Object.assign({},controller.view.s,{authenticated:true});controller.stage=905;
                }else if(controller.stage===905){
                    let next=controller.childNamed(controller.checkedWizard.contentItem,"setupNext");
                    if(!next.enabled)throw new Error("Authenticated setup cannot continue");
                    next.clicked();controller.stage=906;
                }else if(controller.stage===906){
                    let wizard=controller.checkedWizard;
                    if(wizard.step!==1 || controller.childNamed(wizard.contentItem,"setupSignIn").visible || !controller.childNamed(wizard.contentItem,"setupPath").visible)throw new Error("Sync folder is not a separate step");
                    if(wizard.height>controller.view.height-30)throw new Error("Folder step exceeds minimum window height");
                    controller.capture("setup-folder-dark.png",907);
                }else if(controller.stage===907){
                    controller.setAppearancePreference("light");controller.stage=908;
                }else if(controller.stage===908){controller.capture("setup-folder-light.png",909);
                }else if(controller.stage===909){
                    let wizard=controller.checkedWizard;
                    let path=controller.childNamed(wizard.contentItem,"setupPath");
                    let finish=controller.childNamed(wizard.contentItem,"setupFinish");
                    path.text="   ";
                    if(finish.enabled)throw new Error("Setup accepts blank folder");
                    path.text="/tmp/NeutronDrive-setup-preview";
                    if(!finish.enabled)throw new Error("Setup rejects authenticated folder step");
                    controller.childNamed(wizard.contentItem,"setupBack").clicked();
                    if(wizard.step!==0)throw new Error("Setup Back failed");
                    controller.childNamed(wizard.contentItem,"setupNext").clicked();
                    if(path.text!=="/tmp/NeutronDrive-setup-preview")throw new Error("Back lost selected folder");
                    controller.view.s=Qt.binding(function(){return controller.state});
                    controller.view.width=1220;controller.view.height=820;
                    console.log("NATIVE_SETUP_STEPS_AND_THEMES_OK");controller.stage=0;
                }else if(controller.stage===0){
                    if(!controller.view.visible)throw new Error("Main window is not visible");
                    if(controller.fileEntries.length===0)throw new Error("Demo files missing");
                    if(!controller.fixture && (!controller.state.account || !controller.state.account.name)){controller.send("account_storage",[]);return;}
                    let account=controller.childNamed(controller.view.contentItem,"accountMenuButton");
                    if(!account || !account.visible || account.popup.count!==5)throw new Error("Account menu missing");
                    if(!controller.fixture && account.accountName!=="Alex Morgan")throw new Error("Account identity not shown");
                    account.popup.itemAt(0).triggered();
                    if(controller.state.page!=="settings")throw new Error("Account settings navigation failed");
                    console.log("NATIVE_ACCOUNT_MENU_OK");
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
                    let choice=controller.childNamed(controller.view.contentItem,"appearanceSelector");
                    choice.popup.open();controller.stage=110;
                }else if(controller.stage===110){
                    let choice=controller.childNamed(controller.view.contentItem,"appearanceSelector");
                    if(!choice.popup.visible || choice.popup.contentItem.count!==3)throw new Error("Themed appearance choices missing");
                    choice.currentIndex=1;choice.activated(1);choice.popup.close();
                    if(controller.view.darkMode)throw new Error("Themed appearance choice did not apply");
                    console.log("NATIVE_THEMED_CONTROLS_OK");
                    controller.setAppearancePreference("light");
                    if(controller.view.darkMode)throw new Error("Light theme did not reach main window");
                    controller.show_page("background");controller.stage=2;
                }else if(controller.stage===2){
                    if(!(controller.childNamed(controller.view.contentItem,"page-settings").contentHeight>0))throw new Error("Background page cannot scroll");
                    if(controller.childNamed(controller.view.contentItem,"nav-background"))throw new Error("Obsolete sync navigation remains");
                    if(!controller.childNamed(controller.view.contentItem,"restartSyncButton"))throw new Error("Sync restart missing in Settings");
                    controller.capture("light-background.png",2000);
                }else if(controller.stage===2000){
                    controller.show_page("computer-backup");controller.stage=20001;
                }else if(controller.stage===20001){
                    if(!controller.childNamed(controller.view.contentItem,"computerBackupButton"))throw new Error("Computer backup configuration missing");
                    console.log("NATIVE_SYNC_SETTINGS_AND_COMPUTER_BACKUP_OK");controller.capture("light-computer-backup.png",2001);
                }else if(controller.stage===2001){
                    controller.navigate(controller.translate("Computer"),"/devices");controller.stage=20011;
                }else if(controller.stage===20011){
                    let setup=controller.childNamed(controller.view.contentItem,"configureComputerBackup");
                    if(!setup || !setup.visible)throw new Error("Computer setup card missing");
                    let donate=controller.childNamed(controller.view.contentItem,"donationButton");
                    if(!donate || !donate.visible)throw new Error("Ko-fi footer missing");
                    controller.capture("light-computers.png",20012);
                }else if(controller.stage===20012){
                    controller.navigate(controller.translate("Meine Dateien"),"/my-files");controller.stage=20013;
                }else if(controller.stage===20013){
                    controller.show_page("ondemand");controller.stage=3;
                }else if(controller.stage===3){
                    if(!(controller.childNamed(controller.view.contentItem,"page-ondemand").contentHeight>0))throw new Error("Online page cannot scroll");
                    controller.capture("light-ondemand.png",3001);
                }else if(controller.stage===3001){
                    controller.show_page("local-trash");controller.stage=4;
                }else if(controller.stage===4){controller.show_page("activity");controller.stage=5;
                }else if(controller.stage===5){controller.show_page("files");controller.stage=15;
                }else if(controller.stage===15){
                    controller.view.width=900;controller.view.height=640;controller.stage=150;
                }else if(controller.stage===150){
                    let actions=controller.childNamed(controller.view.contentItem,"fileActionBar");
                    let list=controller.childNamed(controller.view.contentItem,"fileList");
                    if(!actions || !list || list.height<100)throw new Error("Compact layout hides the file browser: " + (list ? list.height : "missing list") + ", actions=" + Boolean(actions));
                    for(let item of actions.children){
                        if(!item.visible || item.width<=0)continue;
                        if(item.x+item.width>actions.width+1 || item.y+item.height>actions.height+1)throw new Error("File action clipped in compact layout");
                    }
                    console.log("NATIVE_COMPACT_FILE_ACTIONS_OK");
                    controller.capture("compact-files.png",151);
                }else if(controller.stage===151){
                    controller.view.width=1220;controller.view.height=820;
                    controller.view.hide();
                    // Empty selections exercise action routing without touching any files.
                    controller.handleFileAction({action:"pin",paths:[]});controller.stage=1511;
                }else if(controller.stage===1511){
                    if(controller.view.visible)throw new Error("Pin unexpectedly opened the main window");
                    controller.handleFileAction({action:"evict",paths:[]});controller.stage=1512;
                }else if(controller.stage===1512){
                    if(controller.view.visible)throw new Error("Evict unexpectedly opened the main window");
                    console.log("NATIVE_FILE_ACTIONS_STAY_HIDDEN_OK");
                    files.notify("",false);
                    controller.view.show();controller.stage=152;
                }else if(controller.stage===152){controller.capture("light-files.png",16);
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
                    controller.capture("dark-grid.png",20612);
                }else if(controller.stage===20612){
                    controller.childNamed(controller.view.contentItem,"accountMenuButton").popup.open();controller.stage=20613;
                }else if(controller.stage===20613){controller.capture("dark-account-menu.png",20614);
                }else if(controller.stage===20614){
                    controller.childNamed(controller.view.contentItem,"accountMenuButton").popup.close();controller.stage=2062;
                }else if(controller.stage===2062){
                    controller.setAppearancePreference("light");
                    let point=controller.checkedRow.mapToItem(controller.view.contentItem,40,40);
                    files.windowClick(point.x,point.y,true);controller.stage=207;
                }else if(controller.stage===207){
                    if(controller.state.path==="/my-files")throw new Error("Grid double-click did not open folder");
                    controller.navigate(controller.translate("Meine Dateien"),"/my-files");
                    controller.childNamed(controller.view.contentItem,"listViewButton").clicked();
                    console.log("NATIVE_GRID_KEYBOARD_AND_MOUSE_OK");controller.stage=208;
                }else if(controller.stage===208){
                    if(controller.fixture){controller.stage=21;return;}
                    controller.show_page("photos");controller.send("photo_library",[{action:"index",album:""}]);controller.stage=209;
                }else if(controller.stage===209){
                    if(controller.state.photos.length!==2 || !controller.childNamed(controller.view.contentItem,"photoGrid"))throw new Error("Photo timeline failed");
                    let preview=controller.childNamed(controller.view.contentItem,"photoPreview-0");
                    if(!preview || preview.status!==Image.Ready)return;
                    if(!controller.childNamed(controller.view.contentItem,"takeoutImportButton"))throw new Error("Takeout import missing");
                    console.log("NATIVE_TAKEOUT_AND_DONATION_OK");
                    controller.capture("light-photos.png",210);
                }else if(controller.stage===210){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    let check=controller.childNamed(panel,"photoCheck-0");
                    if(!check || check.visible || panel.selectionMode)throw new Error("Photo selection boxes must be hidden by default");
                    controller.childNamed(panel,"photoSelectionButton").clicked();
                    if(!check.visible)throw new Error("Photo selection toggle did not show overlay");
                    check.clicked();
                    if(panel.selected.length!==1)throw new Error("Photo checkbox did not select exactly one item");
                    controller.stage=21001;
                }else if(controller.stage===21001){controller.capture("light-photos-selection.png",21002);
                }else if(controller.stage===21002){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    controller.childNamed(panel,"photoSelectionButton").clicked();
                    if(panel.selected.length || controller.childNamed(panel,"photoCheck-0").visible)throw new Error("Leaving photo selection did not reset controls");
                    panel.showPhoto(0);controller.stage=21003;
                }else if(controller.stage===21003){controller.capture("light-photo-viewer.png",21004);
                }else if(controller.stage===21004){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    let previewWindow=null;
                    for(let item of panel.data)if(item.objectName==="photoViewer")previewWindow=item;
                    if(!previewWindow)throw new Error("Photo preview missing");
                    controller.childNamed(previewWindow.contentItem,"closePhotoPreview").forceActiveFocus();
                    files.windowKey(Qt.Key_Right,false);controller.stage=210041;
                }else if(controller.stage===210041){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    if(panel.viewerIndex!==1)throw new Error("Photo viewer keyboard navigation failed with toolbar focus");
                    files.windowKey(Qt.Key_Plus,false);controller.stage=210042;
                }else if(controller.stage===210042){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    if(panel.previewZoom!==1.5)throw new Error("Photo viewer keyboard zoom failed");
                    files.windowKey(Qt.Key_Escape,false);controller.stage=210043;
                }else if(controller.stage===210043){
                    let panel=controller.childNamed(controller.view.contentItem,"page-photos");
                    let grid=controller.childNamed(panel,"photoGrid");
                    if(panel.viewerIndex!==-1 || grid.currentIndex!==1 || !grid.activeFocus)throw new Error("Closing preview did not return focus to the current photo");
                    panel.openAlbum("");
                    console.log("NATIVE_PHOTO_SELECTION_AND_VIEWER_OK");controller.stage=21005;
                }else if(controller.stage===21005){
                    controller.childNamed(controller.view.contentItem,"photoAlbumsButton").clicked();controller.stage=2101;
                }else if(controller.stage===2101){
                    let albums=controller.childNamed(controller.view.contentItem,"photoAlbumsGrid");
                    if(!albums || !albums.visible || albums.count!==1)throw new Error("Album browser failed");
                    controller.capture("light-albums.png",2102);
                }else if(controller.stage===2102){
                    let items=[];
                    for(let i=0;i<2000;i++)items.push({uid:"bench-"+i,name:"Sample "+i,safe:true,thumbnail:"",previewLoaded:true});
                    photoBenchController.photoEntries=items;
                    let component=Qt.createComponent("qrc:/neutrondrive/ui/PhotoLibraryPanel.qml");
                    if(component.status!==Component.Ready)throw new Error(component.errorString());
                    controller.photoBench=component.createObject(controller.view.contentItem,{host:controller.view,controller:photoBenchController,viewState:{busy:true,photosMore:false,photoTotal:2000},x:250,y:70,width:900,height:650});
                    if(!controller.photoBench)throw new Error(component.errorString());
                    controller.stage=21020;
                }else if(controller.stage===21020){
                    let grid=controller.childNamed(controller.photoBench,"photoGrid");
                    if(grid.count!==2000)throw new Error("Large photo model missing");
                    grid.positionViewAtIndex(1000,GridView.Beginning);controller.stage=21021;
                }else if(controller.stage===21021){
                    let grid=controller.childNamed(controller.photoBench,"photoGrid");
                    controller.photoBenchDelegate=controller.childNamed(grid,"photoPreview-1000");
                    if(!controller.photoBenchDelegate)throw new Error("Scrolled photo delegate missing");
                    controller.photoBenchScroll=grid.contentY;
                    let items=photoBenchController.photoEntries.slice();items[1000]=Object.assign({},items[1000],{name:"Updated sample"});
                    for(let i=2000;i<2048;i++)items.push({uid:"bench-"+i,name:"Sample "+i,safe:true,thumbnail:"",previewLoaded:true});
                    controller.photoBenchStarted=Date.now();photoBenchController.photoEntries=items;
                    controller.photoBench.viewState={busy:false,photosMore:false,photoTotal:2048};controller.stage=21022;
                }else if(controller.stage===21022){
                    let grid=controller.childNamed(controller.photoBench,"photoGrid");
                    if(grid.count!==2048 || grid.contentY!==controller.photoBenchScroll || controller.childNamed(grid,"photoPreview-1000")!==controller.photoBenchDelegate)throw new Error("Photo append/status update recreated delegates or moved scroll position");
                    console.log("NATIVE_PHOTO_INCREMENTAL_MODEL_OK",Date.now()-controller.photoBenchStarted);
                    if(controller.photoBenchDelegate.sourceSize.width>Math.ceil(grid.cellWidth*controller.view.screen.devicePixelRatio))throw new Error("Photo thumbnail decode exceeds tile resolution");
                    controller.photoBench.selectionMode=true;controller.photoBench.selected=[1000];
                    // Drive the scrolling state deterministically: offscreen Qt does
                    // not consistently advance synthetic kinetic gestures.
                    controller.photoBench.scrolling=true;controller.stage=21023;
                }else if(controller.stage===21023){
                    let grid=controller.childNamed(controller.photoBench,"photoGrid");

                    let items=photoBenchController.photoEntries.slice();items[1000]=Object.assign({},items[1000],{name:"During flick"});
                    items.push({uid:"bench-2048",name:"Next page",safe:true,thumbnail:"",previewLoaded:true});
                    photoBenchController.photoEntries=items;
                    if(grid.count!==2049 || grid.contentY!==controller.photoBenchScroll)throw new Error("Appending during scrolling stalled or moved the viewport");
                    if(!controller.photoBench.pendingPhotos)throw new Error("Photo update was not deferred during movement");
                    controller.photoBench.scrolling=false;controller.stage=21024;
                }else if(controller.stage===21024){
                    if(controller.photoBench.pendingPhotos || controller.photoBench.appliedPhotos[1000].name!=="During flick" || controller.photoBench.selected[0]!==1000)throw new Error("Deferred photo result or selection lost");
                    photoBenchController.requests=[];
                    controller.photoBench.viewState={busy:false,photosMore:true,photoTotal:3000};
                    controller.childNamed(controller.photoBench,"photoGrid").positionViewAtEnd();
                    controller.stage=21025;
                }else if(controller.stage===21025){
                    if(!photoBenchController.requests.some(request=>request.action==="more"))return;
                    controller.photoBench.viewState={busy:false,photosMore:false,photoTotal:2049};
                    controller.photoBench.scrolling=true;
                    let items=photoBenchController.photoEntries.slice();items[2048]=Object.assign({},items[2048],{previewLoaded:false});
                    photoBenchController.requests=[];photoBenchController.photoEntries=items;controller.stage=21026;
                }else if(controller.stage===21026){
                    if(!photoBenchController.requests.some(request=>request.action==="previews" && request.start===2048))return;
                    console.log("NATIVE_PHOTO_AUTOMATIC_PAGING_AND_PREFETCH_OK");
                    // Reordering after the first entry must invalidate index-based
                    // actions even while scrolling; cached metadata alone must not.
                    controller.photoBench.showPhoto(1000);
                    controller.photoBench.zoomPreview(1);
                    controller.photoBench.movePreview(1);
                    if(controller.photoBench.viewerIndex!==1001 || controller.photoBench.previewZoom!==1)throw new Error("Photo navigation did not reset zoom");
                    controller.photoBench.showPhoto(-1);
                    controller.photoBench.showPhoto(99999);
                    if(controller.photoBench.viewerIndex!==1001)throw new Error("Invalid photo navigation changed selection");
                    controller.photoBench.zoomPreview(99);
                    if(controller.photoBench.previewZoom!==4)throw new Error("Photo zoom upper bound failed");
                    controller.photoBench.zoomPreview(-99);
                    if(controller.photoBench.previewZoom!==1)throw new Error("Photo zoom lower bound failed");
                    controller.photoBench.selectionMode=true;controller.photoBench.selected=[1000];
                    controller.photoBench.scrolling=true;
                    let reordered=photoBenchController.photoEntries.slice();
                    let swap=reordered[1000];reordered[1000]=reordered[1001];reordered[1001]=swap;
                    photoBenchController.photoEntries=reordered;
                    if(controller.photoBench.selected.length || controller.photoBench.selectionMode || controller.photoBench.viewerIndex!==-1 || controller.photoBench.pendingPhotos)throw new Error("Changed photo identities retained stale selection or viewer");
                    if(controller.photoBench.appliedPhotos[1000].uid!=="bench-1001")throw new Error("Photo reorder was deferred while scrolling");
                    console.log("NATIVE_PHOTO_IDENTITY_AND_NAVIGATION_OK");
                    controller.photoBench.scrolling=true;photoBenchController.photoEntries=[];
                    if(controller.photoBench.pendingPhotos || controller.childNamed(controller.photoBench,"photoGrid").count!==0)throw new Error("Photo inventory reset left stale tiles during scrolling");
                    console.log("NATIVE_PHOTO_DEFERRED_UPDATES_OK");
                    controller.photoBench.destroy();controller.photoBench=null;
                    controller.show_page("files");
                    let index=controller.fileEntries.findIndex(entry=>!entry.isFolder);
                    controller.select_row(index,false);controller.action("Versions");controller.stage=211;
                }else if(controller.stage===211){
                    if(!controller.state.history.visible || controller.state.history.revisions.length!==2)throw new Error("Version history failed");
                    controller.capture("light-history.png",212);
                }else if(controller.stage===212){
                    let closeButton=null;
                    for(let item of controller.view.contentData)if(item.objectName==="historyDialog")closeButton=controller.childNamed(item.footer,"historyClose");
                    if(!closeButton || !closeButton.visible || !closeButton.enabled || !closeButton.text)throw new Error("History close button missing");
                    closeButton.clicked();
                    if(controller.state.history.visible)throw new Error("History close button did not close dialog");
                    console.log("NATIVE_PHOTOS_AND_HISTORY_OK");controller.stage=21;
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
                    controller.select_row(0,false);controller.view.hide();controller.action("Freigabe");controller.stage=24;
                }else if(controller.stage===24){
                    if(controller.lastDialog.kind!=="share" || !controller.view.sharingWindow.visible)throw new Error("Separate share window missing");
                    if(controller.view.visible)throw new Error("Sharing unexpectedly revealed main window");
                    if(Qt.application.arguments.length>4 && !files.snapshotWindow("shareWindow",Qt.application.arguments[4]+"/share-window-light.png"))throw new Error("Share window capture failed");
                    controller.dialogChild("copyShareLink").clicked();controller.stage=25;
                }else if(controller.stage===25){
                    if(files.clipboardText()!=="https://drive.proton.me/urls/synthetic?literal=%3Cb%3E")throw new Error("Clipboard value changed");
                    controller.view.sharingWindow.close();controller.view.show();
                    console.log("NATIVE_PROMPT_AND_CLIPBOARD_OK");
                    controller.action("chooseBackground");controller.stage=6;
                }else if(controller.stage===6){
                    if(controller.lastDialog.kind!=="folder")throw new Error("Folder dialog missing");
                    for(let child of controller.view.contentData){if(child.objectName==="folderPicker")child.close()}
                    controller.answer_dialog(controller.lastDialog.token,false,[]);
                    // Exercise the real window's tray-close path without requiring a
                    // tray host in the isolated offscreen test session.
                    controller.send("request_close",[false,true]);controller.stage=61;
                }else if(controller.stage===61){
                    if(controller.view.visible)return;
                    if(controller.allowClose)throw new Error("Tray close shut down the application");
                    controller.reveal("");controller.stage=62;
                }else if(controller.stage===62){
                    if(!controller.view.visible)throw new Error("Tray window did not reopen");
                    console.log("NATIVE_TRAY_HIDE_AND_REOPEN_OK");
                    console.log("NATIVE_MAIN_WINDOW_OK");console.log("NATIVE_WINDOW_CLOSE_RESULT",controller.view.close());controller.stage=7;
                }
            }catch(error){console.error(error);controller.stop();Qt.exit(1)}
        }
    }
    Timer{interval:15000;running:controller.smoke;onTriggered:{controller.stop();Qt.exit(2)}}
}
