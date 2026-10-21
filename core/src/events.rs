//! Validate SDK event batches and prepare deliveries without acknowledging I/O.
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn text(value: Option<&Value>, maximum: usize) -> Result<&str, String> {
    let s = value
        .and_then(Value::as_str)
        .ok_or("Ungültiges Ereignisformat der SDK-Bridge.")?;
    if s.is_empty() || s.chars().count() > maximum || s.chars().any(|c| c < ' ') {
        return Err("Ungültiges Ereignisformat der SDK-Bridge.".into());
    }
    Ok(s)
}

pub fn validate(value: Value, paths: &[String]) -> Result<Value, String> {
    if !value.is_object() || value.get("protocol") != Some(&json!(1)) || value["type"] != "batch" {
        return Err("Die installierte SDK-Bridge unterstützt das Ereignisprotokoll nicht.".into());
    }
    let shape = "Unvollständiger Ereignis-Batch der SDK-Bridge.";
    let roots = value["roots"].as_array().ok_or(shape)?;
    let events = value["events"].as_array().ok_or(shape)?;
    let cursors = value["cursors"].as_object().ok_or(shape)?;
    let resets = value["resets"].as_array().ok_or(shape)?;
    if roots.len() != paths.len() || events.len() > 100000 {
        return Err(shape.into());
    }
    let mut scopes = BTreeSet::new();
    let mut actual_paths = Vec::new();
    for root in roots {
        if !root.is_object() {
            return Err("Ungültige Ereignis-Wurzel.".into());
        }
        text(root.get("uid"), 4096)?;
        scopes.insert(text(root.get("scopeId"), 512)?);
        actual_paths.push(text(root.get("path"), 4096)?);
    }
    actual_paths.sort();
    let mut requested: Vec<_> = paths.iter().map(String::as_str).collect();
    requested.sort();
    if actual_paths != requested {
        return Err("Die Ereignis-Wurzeln stimmen nicht mit der Anfrage überein.".into());
    }
    if cursors.keys().any(|s| !scopes.contains(s.as_str()))
        || resets
            .iter()
            .any(|s| !s.as_str().is_some_and(|s| scopes.contains(s)))
    {
        return Err("Unbekannter Ereignisbereich in SDK-Antwort.".into());
    }
    for (scope, cursor) in cursors {
        text(Some(&Value::String(scope.clone())), 512)?;
        text(Some(cursor), 4096)?;
    }
    let kinds = [
        "node_created",
        "node_updated",
        "node_deleted",
        "tree_refresh",
        "tree_remove",
        "fast_forward",
        "shared_with_me_updated",
    ];
    let mut removed = BTreeSet::new();
    for event in events {
        let kind = event["type"].as_str().unwrap_or("");
        if !event.is_object()
            || !kinds.contains(&kind)
            || !event["treeEventScopeId"]
                .as_str()
                .is_some_and(|s| scopes.contains(s))
        {
            return Err("Unbekanntes SDK-Ereignis; Cursor wurde nicht übernommen.".into());
        }
        text(event.get("eventId"), 4096)?;
        for name in ["nodeUid", "parentNodeUid"] {
            if let Some(v) = event.get(name) {
                text(Some(v), 4096)?;
            }
        }
        for name in ["isTrashed", "isShared"] {
            if event.get(name).is_some_and(|v| !v.is_boolean()) {
                return Err("Ungültiger Ereignisstatus; Cursor wurde nicht übernommen.".into());
            }
        }
        if kind.starts_with("node_") && event.get("nodeUid").is_none() {
            return Err("Ereignis ohne Datei-ID; Cursor wurde nicht übernommen.".into());
        }
        if kind == "tree_remove" {
            removed.insert(event["treeEventScopeId"].as_str().unwrap());
        }
    }
    if scopes
        .difference(&removed)
        .copied()
        .collect::<BTreeSet<_>>()
        != cursors.keys().map(String::as_str).collect()
    {
        return Err("Ereignis-Cursor fehlt; Batch wurde nicht bestätigt.".into());
    }
    Ok(value)
}

pub fn notifications(
    batch: Value,
    known: Vec<Value>,
    announced: bool,
    ready: bool,
) -> Result<Value, String> {
    let requested = batch["roots"]
        .as_array()
        .ok_or("Invalid event roots")?
        .iter()
        .map(|r| text(r.get("path"), 4096).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    let batch = validate(batch, &requested)?;
    let roots = batch["roots"].as_array().unwrap();
    let resets = batch["resets"].as_array().unwrap();
    let mut output = Vec::new();
    if !announced || *roots != known || !ready {
        for root in roots {
            let previous = known.iter().find(|r| r["path"] == root["path"]);
            let reset = resets.contains(&root["scopeId"]) || previous != Some(root);
            output.push(json!({"root_uid":root["uid"],"kind":"ready","reset":reset,"scope_id":root["scopeId"]}));
        }
    }
    for event in batch["events"].as_array().unwrap() {
        if event["type"] == "fast_forward" {
            continue;
        }
        for root in roots {
            if root["scopeId"] == event["treeEventScopeId"] {
                let reset = ["tree_refresh", "tree_remove", "shared_with_me_updated"]
                    .contains(&event["type"].as_str().unwrap());
                output.push(json!({"root_uid":root["uid"],"node_uid":event.get("nodeUid"),"parent_uid":event.get("parentNodeUid"),"kind":event["type"],"reset":reset,"scope_id":root["scopeId"],"event_id":event["eventId"],"is_trashed":event.get("isTrashed").and_then(Value::as_bool).unwrap_or(false)}));
            }
        }
    }
    Ok(json!(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch() -> Value {
        json!({"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[],"cursors":{"scope":"cursor"},"resets":[]})
    }
    #[test]
    fn missing_ack_and_unknown_scope_are_rejected() {
        let mut v = batch();
        v["cursors"] = json!({});
        assert!(validate(v, &["/my-files".into()]).is_err());
        let mut v = batch();
        v["resets"] = json!(["unknown"]);
        assert!(validate(v, &["/my-files".into()]).is_err());
    }
    #[test]
    fn reconnect_announces_roots_and_preserves_deletion_status() {
        let mut v = batch();
        v["events"] = json!([{"type":"node_updated","treeEventScopeId":"scope","eventId":"e","nodeUid":"n","isTrashed":true}]);
        let known = v["roots"].as_array().unwrap().clone();
        let result = notifications(v, known, true, false).unwrap();
        assert_eq!(result[0]["kind"], "ready");
        assert_eq!(result[0]["reset"], false);
        assert_eq!(result[1]["is_trashed"], true);
    }
    #[test]
    fn removed_scope_has_no_cursor_but_has_delivery() {
        let mut v = batch();
        v["cursors"] = json!({});
        v["events"] = json!([{"type":"tree_remove","treeEventScopeId":"scope","eventId":"e"}]);
        assert!(validate(v.clone(), &["/my-files".into()]).is_ok());
        let output = notifications(v, vec![], false, false).unwrap();
        assert_eq!(output[1]["kind"], "tree_remove");
        assert_eq!(output[1]["reset"], true);
    }
}
