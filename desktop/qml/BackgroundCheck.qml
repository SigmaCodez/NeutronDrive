import QtQuick
import NeutronDrive.Native
Item {
    id: test
    property int stage: 0
    property var dialog: ({})
    property var state: JSON.parse(controller.stateJson)
    property string job: ""
    NativeFiles { id: controller }
    Connections { target: controller; function onDialogRequested(value) { test.dialog=JSON.parse(value) } }
    function require(value,message){if(!value)throw new Error(message)}
    function send(method,args){controller.dispatch(method,JSON.stringify(args))}
    function finish(code){controller.shutdown();Qt.exit(code)}
    Component.onCompleted: Qt.callLater(function(){
        try {
            require(controller.configure(Qt.application.arguments[2],"en","/missing-synthetic-sdk",false),"configure background controller");
            send("show_page",["background"]);
            send("action",["chooseBackground"]);
            require(dialog.kind==="folder","choose local folder");
            send("answer_dialog",[dialog.token,true,["file://"+Qt.application.arguments[3]]]);
            send("edit",["background_remote","/my-files/synthetic-background"]);
            send("action",["addJob"]);
            require(dialog.kind==="confirm","activation confirmation");
            require(!state.busy,"no mutation before confirmation");
            send("answer_dialog",[dialog.token,true,""]);
            require(state.busy,"background worker started");stage=1;
        } catch(error){console.error(error);finish(1)}
    })
    Timer {
        interval:20;running:true;repeat:true
        onTriggered:{
            try {
                if(state.busy)return;
                if(stage===1){
                    require(state.jobs.length===1,"created job rendered: "+state.fields.notice_label.value);
                    job=state.jobs[0].id;send("job_action",["pause",job]);stage=2;
                }else if(stage===2){
                    require(state.jobs[0].paused,"pause reflected in native status");
                    send("job_action",["remove",job]);
                    require(dialog.kind==="confirm" && !state.busy,"removal waits for confirmation");
                    send("answer_dialog",[dialog.token,false,""]);
                    require(state.jobs.length===1,"cancelled removal retains job");
                    send("job_action",["remove",job]);
                    require(dialog.kind==="confirm","removal can be retried");
                    send("answer_dialog",[dialog.token,true,""]);stage=3;
                }else if(stage===3){
                    require(state.jobs.length===0,"removed job not retained");
                    send("action",["stop"]);stage=4;
                }else if(stage===4){
                    send("poll_services",[]);stage=5;
                }else if(stage===5 && !controller.workerRunning()){
                    require(state.jobs.length===0,"stopped status retained");
                    send("action",["autostart"]);stage=6;
                }else if(stage===6){
                    require(!state.noticeError,"autostart enable: "+state.fields.notice_label.value);
                    send("action",["noAutostart"]);stage=7;
                }else if(stage===7){
                    require(!state.noticeError,"autostart disable: "+state.fields.notice_label.value);
                    console.log("NATIVE_AUTOSTART_OK");
                    controller.shutdown();controller.shutdown();
                    require(!controller.workerRunning(),"status and foreground workers joined");
                    console.log("NATIVE_BACKGROUND_OK");finish(0);
                }
            }catch(error){console.error(error);finish(1)}
        }
    }
    Timer {interval:20000;running:true;onTriggered:finish(2)}
}
