//! Deliberately allowlisted support summary: never serialize raw runtime or error strings.
use serde_json::{json, Map, Value};

pub fn report(state: &Value) -> String {
    let jobs = state["jobs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut states = Map::new();
    for job in jobs {
        let key = match job["state"].as_str() {
            Some("idle") => "idle",
            Some("syncing") => "syncing",
            Some("paused") => "paused",
            Some("pending" | "waiting") => "waiting",
            Some("offline") => "offline",
            Some("conflict" | "conflicts") => "conflicts",
            Some("blocked") => "blocked",
            Some("error") => "error",
            Some("stopped") => "stopped",
            _ => "unknown",
        };
        let count = states.get(key).and_then(Value::as_u64).unwrap_or(0);
        states.insert(key.into(), json!(count + 1));
    }
    let count_items = |field: &str| {
        jobs.iter()
            .map(|j| j[field].as_array().map_or(0, Vec::len))
            .sum::<usize>()
    };
    let report = json!({
        "report_version":1,"app":"NeutronDrive","version":env!("CARGO_PKG_VERSION"),
        "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "service_running":state["backgroundRunning"]==true,
        "job_count":jobs.len(),"job_states":states,
        "jobs_with_errors":jobs.iter().filter(|j|j["hasError"]==true).count(),
        "jobs_without_completed_sync":jobs.iter().filter(|j|!j["lastSync"].as_f64().is_some_and(|t|t>0.0)).count(),
        "paused_jobs":jobs.iter().filter(|j|j["paused"]==true).count(),
        "timed_pauses":jobs.iter().filter(|j|j["paused"]==true && j["pausedUntil"].as_u64().is_some_and(|t|t>0)).count(),
        "conflict_count":count_items("conflicts"),"skipped_count":count_items("skipped"),
        "active_downloads":state["downloads"].as_array().map_or(0, Vec::len),
        "pending_deletions":state["deletions"]["pending"].as_u64().unwrap_or(0)
    });
    serde_json::to_string_pretty(&report).expect("allowlisted JSON values serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_only_copies_allowlisted_aggregates() {
        let state = json!({"version":"PRIVATE_TOKEN","account":{"email":"secret@example.invalid"},
            "backgroundRunning":true,"downloads":["/home/private/file.txt"],
            "deletions":{"pending":2,"items":[{"error":"PRIVATE_TOKEN"}]},
            "jobs":[{"id":"secret-id","name":"Secret project","local_root":"/home/private",
                "state":"offline","hasError":true,"paused":false,"lastSync":123,
                "conflicts":["PRIVATE_TOKEN"],"skipped":["private filename"],"issues":["private error"]},
                {"state":"PRIVATE_TOKEN","paused":true,"pausedUntil":999,"lastSync":null}]});
        let text = report(&state);
        for secret in [
            "PRIVATE_TOKEN",
            "private",
            "secret",
            "Secret",
            "example.invalid",
        ] {
            assert!(!text.contains(secret), "{text}");
        }
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["job_count"], 2);
        assert_eq!(value["job_states"], json!({"offline":1,"unknown":1}));
        assert_eq!(value["conflict_count"], 1);
        assert_eq!(value["timed_pauses"], 1);
        assert_eq!(value["jobs_without_completed_sync"], 1);
        assert_eq!(value["active_downloads"], 1);
        assert_eq!(value["pending_deletions"], 2);
    }
    #[test]
    fn malformed_optional_fields_are_not_copied_or_interpreted_as_paths() {
        let value: Value = serde_json::from_str(&report(
            &json!({"jobs":"private", "downloads":{"private":1},"deletions":{"pending":"private"}}),
        ))
        .unwrap();
        assert_eq!(value["job_count"], 0);
        assert_eq!(value["pending_deletions"], 0);
        assert_eq!(value["active_downloads"], 0);
    }
}
