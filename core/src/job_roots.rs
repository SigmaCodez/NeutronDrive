//! Prove the entire active job set before allowing any persisted identity update.
use crate::{host::Services, process::Failure};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, Failure>;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn folder(node: &Value) -> bool {
    node["safe"] == true
        && node["type"] == "folder"
        && !text(node, "uid").is_empty()
        && node["raw"]["isTrashed"] != true
}
macro_rules! blocked {
    ($message:expr) => { return Ok(json!({"blocked":$message})) };
}
pub fn resolve(jobs: Vec<Value>, selected: &str, host: &mut impl Services) -> Result<Value> {
    let account = host.request("info", json!({"path":"/my-files"}))?;
    if !folder(&account) {
        blocked!("Proton-Kontowurzel konnte nicht sicher bestätigt werden.");
    }
    let account_uid = text(&account, "uid");
    let mut pinned = Vec::new();
    let mut nodes = BTreeMap::new();
    for mut job in jobs.into_iter().filter(|j| j["paused"] != true) {
        let uid = text(&job, "account_root_uid");
        if !uid.is_empty() && uid != account_uid {
            blocked!("Proton-Konto wurde gewechselt; betroffene Jobs bewusst neu anlegen.");
        }
        let node = host.request("info", json!({"path":job["remote_root"]}))?;
        if !folder(&node) {
            blocked!("Ein Remote-Sync-Ordner ist nicht sicher verfügbar.");
        }
        let uid = text(&job, "remote_uid");
        if !uid.is_empty() && uid != text(&node, "uid") {
            blocked!("Remote-Wurzel wurde geändert; Job bewusst neu anlegen.");
        }
        job["remote_uid"] = node["uid"].clone();
        job["account_root_uid"] = account["uid"].clone();
        job["remote_root"] = node["path"].clone();
        nodes.insert(text(&job, "id").to_owned(), node);
        pinned.push(job);
    }
    if !pinned.iter().any(|job| text(job, "id") == selected) {
        return Ok(json!({"cancelled":true}));
    }
    let all: BTreeSet<_> = pinned.iter().map(|j| text(j, "remote_uid")).collect();
    if all.len() != pinned.len() {
        blocked!("Mehrere Sync-Jobs verweisen auf denselben Remote-Ordner.");
    }
    for job in &pinned {
        let boundary = match crate::computer::boundary(text(job, "remote_root")) {
            Ok(path) => path,
            Err(error) => blocked!(error),
        };
        let boundary_node = if boundary == "/my-files" {
            account.clone()
        } else {
            host.request("info", json!({"path":boundary}))?
        };
        if !folder(&boundary_node) {
            blocked!("Computer root is not safely accessible");
        }
        let mut current = nodes[text(job, "id")].clone();
        let mut visited = BTreeSet::from([text(job, "remote_uid").to_owned()]);
        let mut proven = false;
        for _ in 0..128 {
            if current["uid"] == boundary_node["uid"] {
                proven = true;
                break;
            }
            let parent = text(&current["raw"], "parentUid");
            if parent.is_empty() {
                blocked!(
                    "Remote-Ordnerabgrenzung nicht nachweisbar; Jobs überlappen möglicherweise."
                );
            }
            if all.contains(parent) {
                blocked!("Remote-Sync-Jobs dürfen keine gemeinsamen Unterbäume haben.");
            }
            if parent == text(&boundary_node, "uid") {
                proven = true;
                break;
            }
            if !visited.insert(parent.to_owned()) {
                blocked!("Zyklische Remote-Ordnerstruktur.");
            }
            let node = host.request("info", json!({"path":format!("{boundary}/{parent}")}))?;
            if !folder(&node) || text(&node, "uid") != parent {
                blocked!("Remote-Ordnerabgrenzung konnte nicht sicher bestätigt werden.");
            }
            current = node;
        }
        if !proven {
            blocked!("Remote-Ordnerstruktur überschreitet die sichere maximale Tiefe.");
        }
    }
    Ok(json!({"jobs":pinned}))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host(BTreeMap<String, Value>);
    impl Services for Host {
        fn poll(&mut self) -> Result<()> {
            Ok(())
        }
        fn request(&mut self, method: &str, args: Value) -> Result<Value> {
            assert_eq!(method, "info");
            self.0
                .get(text(&args, "path"))
                .cloned()
                .ok_or_else(|| "offline".into())
        }
    }
    fn node(uid: &str, parent: &str) -> Value {
        json!({"uid":uid,"safe":true,"type":"folder","path":format!("/my-files/{uid}"),"raw":{"parentUid":parent}})
    }
    fn host() -> Host {
        Host(BTreeMap::from([
            ("/my-files".into(), node("account", "")),
            ("/my-files/a".into(), node("a", "account")),
            ("/my-files/b".into(), node("b", "a")),
        ]))
    }
    fn job(id: &str, root: &str) -> Value {
        json!({"id":id,"remote_root":format!("/my-files/{root}"),"remote_uid":"","account_root_uid":"","paused":false})
    }
    #[test]
    fn validates_all_roots_before_returning_pins() {
        let mut h = host();
        let result = resolve(vec![job("first", "a")], "first", &mut h).unwrap();
        assert_eq!(result["jobs"][0]["remote_uid"], "a");
        assert_eq!(result["jobs"][0]["account_root_uid"], "account");
        for jobs in [
            vec![job("first", "a"), job("second", "b")],
            vec![job("second", "b"), job("first", "a")],
        ] {
            let result = resolve(jobs, "first", &mut h).unwrap();
            assert!(result["blocked"].as_str().unwrap().contains("Unterbäume"));
            assert!(result["jobs"].is_null());
        }
    }
    #[test]
    fn rejects_account_replacement_trashed_ancestors_and_cycles() {
        let mut h = host();
        let mut pinned = job("first", "a");
        pinned["account_root_uid"] = json!("old-account");
        assert!(resolve(vec![pinned], "first", &mut h).unwrap()["blocked"]
            .as_str()
            .unwrap()
            .contains("Konto"));
        h.0.get_mut("/my-files/a").unwrap()["raw"]["isTrashed"] = json!(true);
        assert!(resolve(vec![job("first", "b")], "first", &mut h).unwrap()["blocked"].is_string());
        h.0.get_mut("/my-files/a").unwrap()["raw"] = json!({"parentUid":"b"});
        assert!(resolve(vec![job("first", "b")], "first", &mut h).unwrap()["blocked"].is_string());
    }
    #[test]
    fn paused_selection_and_transport_failures_remain_distinct() {
        let mut h = host();
        let mut paused = job("first", "a");
        paused["paused"] = json!(true);
        assert_eq!(
            resolve(vec![paused], "first", &mut h).unwrap()["cancelled"],
            true
        );
        assert!(resolve(vec![job("first", "missing")], "first", &mut h).is_err());
    }
}
