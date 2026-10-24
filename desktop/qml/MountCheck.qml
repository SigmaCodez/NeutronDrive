import QtQuick
import NeutronDrive.Native
Item {
    id: test
    property int stage: 0
    property int settleTicks: 0
    property var dialog: ({})
    property var state: JSON.parse(controller.stateJson)
    NativeFiles { id: controller }
    Connections { target: controller; function onDialogRequested(value){test.dialog=JSON.parse(value)} }
    function require(value,message){if(!value)throw new Error(message)}
    function send(method,args){controller.dispatch(method,JSON.stringify(args))}
    function finish(code){controller.shutdown();Qt.exit(code)}
    Component.onCompleted: Qt.callLater(function(){
        try {
            require(controller.configure(Qt.application.arguments[2],"en",Qt.application.arguments[3],false),"configure mount controller");
            send("show_page",["ondemand"]);
            send("action",["chooseMount"]);require(dialog.kind==="folder","mount chooser");
            send("answer_dialog",[dialog.token,true,["file://"+Qt.application.arguments[4]]]);
            send("action",["mount"]);stage=1;
        }catch(error){console.error(error);finish(1)}
    })
    Timer {
        interval:20;running:true;repeat:true
        onTriggered:{
            try {
                if(state.busy || stage===0)return;
                require(!state.noticeError,"mount action failed at stage "+stage+": "+state.fields.notice_label.value);
                if(stage===1){
                    require(state.fields.mount_status.value.indexOf(Qt.application.arguments[4])>=0,"mounted path in status");
                    send("edit",["cache_path","data"]);send("action",["pin"]);stage=2;
                }else if(stage===2){send("action",["evict"]);stage=3;
                }else if(stage===3){
                    send("action",["pin"]);stage=9;
                }else if(stage===9){send("action",["unmount"]);stage=4;
                }else if(stage===4){send("edit",["mount_offline",true]);send("action",["mount"]);stage=5;
                }else if(stage===5){send("action",["pin"]);stage=6;
                }else if(stage===6){send("action",["unmount"]);stage=7;
                }else if(stage===7){send("show_page",["local-trash"]);stage=8;
                }else if(stage===8){
                    require(state.trash.length===0,"native empty trash listing");
                    require(state.fields.local_trash_empty.visible,"empty trash message");
                    send("setup_folder",[Qt.application.arguments[4]]);send("setup_finish",[]);stage=10;
                }else if(stage===10){
                    require(!state.setupNeeded,"setup completion persisted only after mount and autostart");
                    require(state.fields.mount_writable.value && !state.fields.mount_offline.value,"wizard settings match actual mount");
                    stage=13;
                }else if(stage===13){
                    // Let the newly writable kernel mount finish its startup requests.
                    if(++settleTicks>=25){send("action",["unmount"]);stage=11;}
                }else if(stage===11){send("action",["noAutostart"]);stage=12;
                }else if(stage===12){
                    controller.shutdown();require(!controller.workerRunning(),"mount/status/trash workers joined");
                    console.log("NATIVE_GUIDED_SETUP_OK");
                    console.log("NATIVE_MOUNT_TRASH_OK");finish(0);
                }
            }catch(error){console.error(error);finish(1)}
        }
    }
    Timer {interval:20000;running:true;onTriggered:finish(2)}
}
