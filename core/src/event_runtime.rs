//! Durable event acknowledgements. Never commit before every delivery succeeds.
use crate::{computer, events, process, storage};
use process::Failure;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
type Result<T> = std::result::Result<T, Failure>;

pub fn roots(paths: &[String]) -> Result<()> {
    if paths.is_empty()
        || paths.len() > 64
        || paths.iter().collect::<BTreeSet<_>>().len() != paths.len()
    {
        return Err("One to 64 distinct event roots are required".into());
    }
    for path in paths {
        computer::boundary(path)?;
    }
    Ok(())
}
fn prepare(file: &Path) -> Result<()> {
    let parent = file.parent().ok_or("Invalid event state path")?;
    storage::directory(parent, true, true)?;
    Ok(())
}
pub fn load(file: &Path, paths: &[String]) -> Result<Value> {
    roots(paths)?;
    prepare(file)?;
    let Some(mut value) = storage::read_optional(file, 1024 * 1024, true)? else {
        return Ok(json!({"cursors":{},"roots":[]}));
    };
    let removed = value.get("removed_scopes").cloned().unwrap_or(json!([]));
    let removed = removed
        .as_array()
        .ok_or("Ungültige Ereignis-Cursordatei.")?;
    if !value.is_object() {
        return Err("Ungültige Ereignis-Cursordatei.".into());
    }
    value["type"] = json!("batch");
    value["events"] = json!(removed
        .iter()
        .map(|scope| json!({"type":"tree_remove","treeEventScopeId":scope,"eventId":"none"}))
        .collect::<Vec<_>>());
    value["resets"] = json!(removed);
    events::validate(value, paths).map_err(Into::into)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub file: String,
    pub paths: Vec<String>,
    pub known_roots: Vec<Value>,
    pub announced: bool,
    pub ready: bool,
}
fn notify(callback: &mut impl FnMut(Value) -> i32, value: Value) -> Result<()> {
    if callback(value) != 0 {
        return Err(Failure {
            message: "Ereignisquelle angehalten.".into(),
            cancelled: true,
        });
    }
    Ok(())
}
pub fn deliver(
    config: Delivery,
    batch: Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    roots(&config.paths)?;
    let batch = events::validate(batch, &config.paths)?;
    let notifications = events::notifications(
        batch.clone(),
        config.known_roots,
        config.announced,
        config.ready,
    )?;
    notify(callback, json!({"kind":"poll"}))?;
    for event in notifications.as_array().ok_or("Invalid event delivery")? {
        notify(callback, json!({"kind":"remote_event","event":event}))?;
    }
    notify(callback, json!({"kind":"poll"}))?;
    let file = Path::new(&config.file);
    prepare(file)?;
    let removed: BTreeSet<_> = batch["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "tree_remove")
        .map(|e| e["treeEventScopeId"].as_str().unwrap())
        .collect();
    storage::write(
        file,
        json!({"protocol":1,"roots":batch["roots"],"cursors":batch["cursors"],"removed_scopes":removed}),
    )?;
    Ok(batch)
}
pub fn poll(
    executable: String,
    config: Delivery,
    cursors: Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    let batch = query(executable, &config.paths, cursors, callback)?;
    deliver(config, batch, callback)
}

pub fn query(
    executable: String,
    paths: &[String],
    cursors: Value,
    callback: &mut impl FnMut(Value) -> i32,
) -> Result<Value> {
    roots(paths)?;
    if !cursors.is_object() {
        return Err("Invalid event cursors".into());
    }
    let mut args = vec![
        "events".into(),
        "poll".into(),
        "--json".into(),
        "--cursor-json".into(),
        cursors.to_string(),
        "--".into(),
    ];
    args.extend_from_slice(paths);
    let value = process::run(
        process::Request {
            version: 1,
            executable,
            args,
            timeout: 120.,
            parse: true,
            login: false,
            max_output: 64 * 1024 * 1024,
        },
        callback,
    )?;
    events::validate(value, paths).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "nd-event-native-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn config(&self) -> Delivery {
            Delivery {
                file: self.0.join("private/events.json").to_str().unwrap().into(),
                paths: vec!["/my-files".into()],
                known_roots: vec![],
                announced: false,
                ready: false,
            }
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn batch(cursor: &str) -> Value {
        json!({"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[{"type":"node_updated","treeEventScopeId":"scope","eventId":"change","nodeUid":"node"}],"cursors":{"scope":cursor},"resets":[]})
    }
    #[test]
    fn commit_occurs_only_after_all_callbacks_return() {
        let temp = Temporary::new();
        let file = PathBuf::from(temp.config().file);
        let mut received = 0;
        deliver(temp.config(), batch("one"), &mut |event| {
            assert!(!file.exists());
            if event["kind"] == "remote_event" {
                received += 1;
            }
            0
        })
        .unwrap();
        assert_eq!(received, 2);
        assert_eq!(
            load(&file, &["/my-files".into()]).unwrap()["cursors"]["scope"],
            "one"
        );
    }
    #[test]
    fn rejected_delivery_keeps_old_cursor_for_replay() {
        let temp = Temporary::new();
        let file = PathBuf::from(temp.config().file);
        deliver(temp.config(), batch("old"), &mut |_| 0).unwrap();
        let old = std::fs::read(&file).unwrap();
        let error = deliver(temp.config(), batch("new"), &mut |event| {
            i32::from(event["kind"] == "remote_event")
        })
        .unwrap_err();
        assert!(error.cancelled);
        assert_eq!(std::fs::read(&file).unwrap(), old);
    }
    #[test]
    fn cancelled_after_last_delivery_does_not_commit() {
        let temp = Temporary::new();
        let mut received = 0;
        let result = deliver(temp.config(), batch("new"), &mut |event| {
            if event["kind"] == "remote_event" {
                received += 1;
            }
            i32::from(received == 2 && event["kind"] == "poll")
        });
        assert!(result.unwrap_err().cancelled);
        assert!(!Path::new(&temp.config().file).exists());
    }
    #[test]
    fn malformed_batch_delivers_nothing() {
        let temp = Temporary::new();
        let mut value = batch("new");
        value["events"][0]["type"] = json!("unknown");
        assert!(deliver(temp.config(), value, &mut |_| panic!("unexpected delivery")).is_err());
        assert!(!Path::new(&temp.config().file).exists());
    }
    #[test]
    fn invalid_durable_state_is_not_missing_state() {
        let temp = Temporary::new();
        let file = PathBuf::from(temp.config().file);
        prepare(&file).unwrap();
        storage::write(&file, Value::Null).unwrap();
        assert!(load(&file, &["/my-files".into()]).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap().trim(), "null");
    }
    #[test]
    fn removed_scope_survives_restart_without_cursor() {
        let temp = Temporary::new();
        let mut value = batch("old");
        value["events"] =
            json!([{"type":"tree_remove","treeEventScopeId":"scope","eventId":"gone"}]);
        value["cursors"] = json!({});
        value["resets"] = json!(["scope"]);
        deliver(temp.config(), value, &mut |_| 0).unwrap();
        let reloaded = load(Path::new(&temp.config().file), &["/my-files".into()]).unwrap();
        assert_eq!(reloaded["cursors"], json!({}));
        assert_eq!(reloaded["events"][0]["type"], "tree_remove");
    }
}
