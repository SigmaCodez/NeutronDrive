//! Separate application processes must reuse local views without cloud calls.
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};
#[test]
fn cache_child() {
    let Ok(root) = std::env::var("ND_CACHE_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    let op: Value = serde_json::from_str(&std::env::var("ND_CACHE_OPERATION").unwrap()).unwrap();
    let result = neutrondrive_core::browse_cache::run(
        root.join("sdk").to_str().unwrap().into(),
        op,
        std::env::var("ND_CACHE_FRESH").unwrap() == "1",
        &mut |_| 0,
    );
    std::fs::write(
        root.join("result"),
        serde_json::to_vec(&result.map_err(|e| e.message)).unwrap(),
    )
    .unwrap();
}
#[test]
fn restarting_offline_reuses_every_cloud_section_and_reconciles_deletions() {
    let root = std::env::temp_dir().join(format!("nd-cache-process-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("sdk"),r##"#!/usr/bin/python3
import json,pathlib,sys
r=pathlib.Path(__file__).parent
if sys.argv[1:3]==['cache','identity']:
 print(json.dumps({'namespace':(r/'account').read_text()}));sys.exit(0)
with (r/'cloud-calls').open('a') as f:f.write('cloud\n')
if (r/'offline').exists():sys.exit(1)
a=sys.argv[1:3]
if a==['filesystem','list']:
 v=[] if (r/'deleted').exists() else [{'uid':'one','name':{'ok':True,'value':'Test'},'type':'folder'}]
elif a==['account','storage']:v={'account':{'email':'synthetic@example.invalid'}}
elif a==['photo','library']:
 q=json.loads(sys.argv[-1])
 if q['action']=='index':v={'items':[{'uid':'photo'}],'total':1}
 elif q['action']=='albums':v={'albums':[{'uid':'album','name':'Test','safe':True}]}
 else:v={'photos':[{'uid':i,'revision':'r1','name':'Photo','safe':True,'thumbnail':'data:image/jpeg;base64,/9g='} for i in q['ids']]}
else:v={}
print(json.dumps(v))
"##).unwrap();
    std::fs::set_permissions(root.join("sdk"), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(root.join("account"), "a".repeat(64)).unwrap();
    let invoke = |op: &Value, fresh: bool| {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cache_child", "--nocapture"])
            .env("ND_CACHE_CHILD", &root)
            .env("ND_CACHE_OPERATION", op.to_string())
            .env("ND_CACHE_FRESH", if fresh { "1" } else { "0" })
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_STATE_HOME", root.join("state"))
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        serde_json::from_slice::<Value>(&std::fs::read(root.join("result")).unwrap()).unwrap()
    };
    let mut ops: Vec<Value> = [
        "/my-files",
        "/shared-with-me",
        "/shared-by-me",
        "/devices",
        "/trash",
    ]
    .iter()
    .map(|p| json!({"action":"list","path":p}))
    .collect();
    for action in ["index", "albums", "details"] {
        ops.push(json!({"action":"photo_library","request":{"action":action,"ids":["photo"],"album":""}}));
    }
    let first: Vec<_> = ops.iter().map(|op| invoke(op, false)).collect();
    assert!(first.iter().all(|v| v.get("Ok").is_some()), "{first:?}");
    let calls = std::fs::read(root.join("cloud-calls")).unwrap();
    use std::os::fd::AsRawFd;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("state/neutrondrive/backend.lock"))
        .unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let cached_start = std::time::Instant::now();

    std::fs::write(root.join("offline"), "").unwrap();
    for (op, expected) in ops.iter().zip(&first) {
        assert_eq!(&invoke(op, false), expected);
    }
    assert_eq!(
        std::fs::read(root.join("cloud-calls")).unwrap(),
        calls,
        "Restart contacted cloud despite complete snapshots"
    );
    assert!(
        cached_start.elapsed() < std::time::Duration::from_secs(3),
        "Cache waited for the cloud lock"
    );
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
    assert!(invoke(&ops[0], true).get("Err").is_some());
    assert_eq!(
        invoke(&ops[0], false),
        first[0],
        "Temporary refresh failure erased cache"
    );
    std::fs::write(root.join("account"), "b".repeat(64)).unwrap();
    assert!(invoke(&ops[0], false).get("Err").is_some());
    std::fs::write(root.join("account"), "a".repeat(64)).unwrap();
    std::fs::remove_file(root.join("offline")).unwrap();
    std::fs::write(root.join("deleted"), "").unwrap();
    assert_eq!(invoke(&ops[0], true), json!({"Ok":[]}));
    std::fs::write(root.join("offline"), "").unwrap();
    assert_eq!(invoke(&ops[0], false), json!({"Ok":[]}));
    std::fs::remove_dir_all(root).unwrap();
}
