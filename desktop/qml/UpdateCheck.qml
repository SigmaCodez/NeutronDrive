import QtQuick
import NeutronDrive.Native
Item {
    id:test
    NativeUpdates { id:updates }
    NativeUpdates { id:reloaded }
    property var state: JSON.parse(updates.stateJson)
    function require(value,message){if(!value)throw new Error(message)}
    Component.onCompleted: Qt.callLater(function(){
        try {
            require(updates.configure(Qt.application.arguments[2],"en","/synthetic/payload",true),"configure updates");
            require(state.packageFormat==="other","uninstalled build format");
            require(!state.available && !state.ready && !state.busy,"initial state");
            require(state.status!=="Noch nicht nach Updates gesucht.","translated status");
            updates.setOption("automatic",false);
            updates.setOption("download",true);
            updates.setOption("previews",false);
            require(!state.automatic && state.autoDownload && !state.previews,"saved options");
            require(reloaded.configure(Qt.application.arguments[2],"en","/synthetic/payload",true),"reloaded options");
            let saved=JSON.parse(reloaded.stateJson);
            require(!saved.automatic && saved.autoDownload && !saved.previews,"persisted options");
            updates.setOption("invalid",true);
            updates.dispatch("check",false);updates.tick();updates.dispatch("download",false);updates.dispatch("install",false);
            require(!state.busy && !updates.workerRunning(),"demo makes no network or installation requests");
            require(updates.shutdown() && updates.shutdown() && reloaded.shutdown(),"idempotent shutdown");
            updates.setOption("automatic",true);
            require(!state.automatic,"closed controller cannot change preferences");
            console.log("NATIVE_UPDATES_OK");Qt.exit(0);
        } catch(error){console.error(error);updates.shutdown();reloaded.shutdown();Qt.exit(1)}
    })
    Timer{interval:10000;running:true;onTriggered:Qt.exit(2)}
}
