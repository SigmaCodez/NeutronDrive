//! Photo library orchestration: inventory first, bounded metadata/preview batches afterwards.
use super::*;
use std::collections::{HashMap, HashSet};
impl Files {
    pub(super) fn photo_submit(&mut self, request: Value) -> Vec<Effect> {
        let mut operation_request = request.clone();
        if let Some(fields) = operation_request.as_object_mut() {
            fields.remove("preserve");
            fields.remove("reloadAlbum");
        }
        self.submit(
            "Photos",
            json!({"action":"photo_library","request":operation_request}),
            Done::PhotoLibrary(request),
        )
    }
    pub(super) fn photo_action(&mut self, request: &Value) -> Vec<Effect> {
        match text(request, "action") {
            "takeout" if self.state["authenticated"] == true && self.state["demo"] != true => self.dialog(
                "folder","Google Takeout import","Extract all Takeout ZIP files into one folder first. Choose the Google Photos folder. Photos and videos are checked before uploading; matching JSON files supply dates and locations.",json!({"multiple":false}),Answer::TakeoutFolder),
            "map" => {
                let photo = request["index"]
                    .as_u64()
                    .and_then(|i| self.state["photos"].get(i as usize));
                let Some(photo) = photo else {
                    return vec![];
                };
                let (Some(lat), Some(lon)) = (
                    photo["location"]["latitude"].as_f64(),
                    photo["location"]["longitude"].as_f64(),
                ) else {
                    return vec![];
                };
                if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
                    return vec![];
                }
                vec![Effect::OpenUrl(format!(
                    "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=14/{lat}/{lon}"
                ))]
            }
            "index" => {
                let mut next = json!({"action":"index","album":text(request,"album")});
                if request["preserve"] == true { next["preserve"] = json!(true); }
                self.photo_submit(next)
            },
            "albums" => self.photo_submit(json!({"action":"albums"})),
            "previews" => {
                let start = request["start"].as_u64().unwrap_or(0) as usize;
                let photos = self.state["photos"].as_array();
                let ids: Vec<_> = photos
                    .into_iter()
                    .flatten()
                    .skip(start)
                    .take(48)
                    .filter(|p| p["previewLoaded"] != true)
                    .map(|p| p["uid"].clone())
                    .collect();
                if ids.is_empty() {
                    return vec![];
                }
                self.photo_submit(json!({"action":"details","ids":ids,"previewStart":start}))
            }
            "more" => {
                let start = self.state["photoOffset"].as_u64().unwrap_or(0) as usize;
                if start >= self.photo_index.len() {
                    return vec![];
                }
                let end = (start + 48).min(self.photo_index.len());
                self.photo_submit(
                    json!({"action":"details","ids":self.photo_index[start..end],"start":start}),
                )
            }
            "create" => self.dialog(
                "prompt",
                "New album",
                "Album name",
                json!({}),
                Answer::PhotoLibrary(json!({"action":"create"})),
            ),
            "rename" | "delete" => {
                let uid = text(request, "album");
                let album = self.state["photoAlbums"]
                    .as_array()
                    .and_then(|a| a.iter().find(|a| a["uid"] == uid && a["safe"] == true))
                    .cloned();
                let Some(album) = album else {
                    return vec![];
                };
                let captured = json!({"action":request["action"],"album":uid});
                if request["action"] == "rename" {
                    self.dialog(
                        "prompt",
                        "Rename album",
                        "Album name",
                        json!({"initial":album["name"]}),
                        Answer::PhotoLibrary(captured),
                    )
                } else {
                    self.dialog("confirm", "Delete album?", "The album is permanently deleted. Photos are kept in your timeline. This cannot be undone.", json!({}), Answer::PhotoLibrary(captured))
                }
            }
            "add" | "remove" | "cover" => {
                let uid = text(request, "album");
                if !self.state["photoAlbums"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|a| a["uid"] == uid && a["safe"] == true))
                {
                    return vec![];
                }
                let Some(indices) = request["indices"].as_array() else {
                    return vec![];
                };
                let ids: BTreeSet<_> = indices
                    .iter()
                    .filter_map(|i| i.as_u64())
                    .filter_map(|i| self.state["photos"].get(i as usize))
                    .filter(|p| p["safe"] == true)
                    .map(|p| text(p, "uid").to_string())
                    .collect();
                if ids.is_empty() || ids.len() > 48 {
                    return vec![];
                }
                let ids: Vec<_> = ids.into_iter().collect();
                if request["action"] == "cover" && ids.len() != 1 {
                    return vec![];
                }
                let captured =
                    json!({"action":request["action"],"album":uid,"ids":ids,"photo":ids[0]});
                self.dialog(
                    "confirm",
                    "Change album?",
                    "Apply this change to the selected photos?",
                    json!({}),
                    Answer::PhotoLibrary(captured),
                )
            }
            _ => vec![],
        }
    }
    pub(super) fn photo_result(&mut self, request: &Value, value: Value) -> Vec<Effect> {
        match text(request, "action") {
            "index" => {
                let Some(items) = value["items"].as_array() else {
                    self.message("Invalid photo inventory", true);
                    return vec![];
                };
                if items.len() > 500000
                    || value["total"].as_u64() != Some(items.len() as u64)
                    || items.iter().any(|i| text(i, "uid").is_empty())
                {
                    self.message("Invalid photo inventory", true);
                    return vec![];
                }
                let ids: Vec<_> = items.iter().map(|i| text(i, "uid").to_string()).collect();
                if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
                    self.message("Duplicate photo inventory identity", true);
                    return vec![];
                }
                self.photo_checked = Some(Instant::now());
                // Refresh an unchanged inventory without discarding decoded
                // previews, pagination or the user's current scroll position.
                if request["preserve"] == true
                    && self.state["photoTotal"].is_u64()
                    && self.photo_index == ids
                    && self.state["photoAlbum"] == request["album"]
                {
                    return if self.state["photos"].as_array().is_some_and(Vec::is_empty) {
                        self.photo_action(&json!({"action":"more"}))
                    } else {
                        vec![]
                    };
                }
                self.photo_index = ids;
                self.state["photoTotal"] = json!(self.photo_index.len());
                self.state["photos"] = json!([]);
                self.photos_revision += 1;
                self.state["photoOffset"] = json!(0);
                self.state["photoAlbum"] = request["album"].clone();
                self.state["photoSelection"] = json!([]);
                self.state["photosMore"] = json!(!self.photo_index.is_empty());
                self.photo_action(&json!({"action":"more"}))
            }
            "details" => {
                let Some(photos) = value["photos"].as_array() else {
                    self.message("Invalid photo metadata", true);
                    return vec![];
                };
                if photos.len() > 48 {
                    self.message("Invalid photo batch", true);
                    return vec![];
                }
                // Validate before changing state; duplicate identities make a
                // response ambiguous, including in thumbnail-only batches.
                let by_id: HashMap<_, _> = photos.iter().map(|p| (text(p, "uid"), p)).collect();
                if by_id.len() != photos.len() || by_id.contains_key("") {
                    self.message("Invalid photo batch", true);
                    return vec![];
                }
                if let Some(start) = request["previewStart"].as_u64() {
                    let Some(ids) = request["ids"].as_array() else {
                        return vec![];
                    };
                    if photos.iter().any(|p| !ids.contains(&p["uid"])) {
                        self.message("Invalid photo batch", true);
                        return vec![];
                    }
                    let mut current = match self.state["photos"].take() {
                        Value::Array(items) => items,
                        _ => Vec::new(),
                    };
                    for (index, old) in current.iter_mut().enumerate() {
                        if index < (start as usize).saturating_sub(48)
                            || index >= start as usize + 96
                        {
                            old["thumbnail"] = json!("");
                            old["previewLoaded"] = json!(false);
                        } else if let Some(p) = by_id.get(text(old, "uid")) {
                            // Metadata may have changed since this tile was
                            // cached, including location, name or access state.
                            *old = (*p).clone();
                            old["thumbnail"] = checked_thumbnail(p);
                            old["previewLoaded"] = json!(true);
                        } else if ids.contains(&old["uid"]) {
                            // Deleted or inaccessible items must not retain an
                            // old image or remain actionable after a fresh read.
                            old["thumbnail"] = json!("");
                            old["safe"] = json!(false);
                            old["location"] = Value::Null;
                            old["previewLoaded"] = json!(true);
                        }
                    }
                    self.state["photos"] = Value::Array(current);
                    self.photos_revision += 1;
                    return vec![];
                }
                let start = request["start"].as_u64().unwrap_or(u64::MAX);
                let Some(ids) = request["ids"].as_array() else {
                    return vec![];
                };
                if start != self.state["photoOffset"].as_u64().unwrap_or(0)
                    || photos.iter().any(|p| !ids.contains(&p["uid"]))
                {
                    self.message("Invalid photo batch", true);
                    return vec![];
                }
                let mut current = match self.state["photos"].take() {
                    Value::Array(items) => items,
                    _ => Vec::new(),
                };
                // One inventory scan per page instead of one scan per photo.
                // Keep the inventory's timeline order even if metadata arrives
                // out of order, and skip missing entries without stalling paging.
                let mut known: HashSet<String> =
                    current.iter().map(|p| text(p, "uid").to_owned()).collect();
                for id in ids.iter().filter_map(Value::as_str) {
                    let Some(photo) = by_id.get(id) else {
                        continue;
                    };
                    if !known.insert(id.to_owned()) {
                        continue;
                    }
                    let mut p = (*photo).clone();
                    p["thumbnail"] = checked_thumbnail(&p);
                    p["previewLoaded"] = json!(true);
                    current.push(p);
                }
                let keep_from = current.len().saturating_sub(96);
                for photo in current.iter_mut().take(keep_from) {
                    photo["thumbnail"] = json!("");
                    photo["previewLoaded"] = json!(false);
                }
                self.state["photos"] = Value::Array(current);
                self.photos_revision += 1;
                let next = start + ids.len() as u64;
                self.state["photoOffset"] = json!(next);
                self.state["photosMore"] = json!((next as usize) < self.photo_index.len());
                vec![]
            }
            "albums" => {
                if value["albums"].as_array().is_some_and(|a| a.len() <= 10000) {
                    self.state["photoAlbums"] = value["albums"].clone();
                    if let Some(album) = request["reloadAlbum"].as_str() {
                        return self.photo_action(&json!({"action":"index","album":album}));
                    }
                } else {
                    self.message("Invalid album list", true);
                }
                vec![]
            }
            "create" | "rename" | "delete" | "add" | "remove" | "cover" => {
                if value["accepted"] != true && value["uid"].as_str().is_none() {
                    self.message("Invalid album operation result", true);
                    return vec![];
                }
                let mut next = json!({"action":"albums"});
                if matches!(text(request, "action"), "add" | "remove" | "delete")
                    && !text(request, "album").is_empty()
                    && self.state["photoAlbum"] == request["album"]
                {
                    let album = if request["action"] == "delete" {
                        ""
                    } else {
                        text(request, "album")
                    };
                    next["reloadAlbum"] = json!(album);
                    self.state["photoAlbum"] = json!(album);
                    // Clear invalidated membership immediately, even if the
                    // follow-up fetch is cancelled or fails.
                    self.state["photoTotal"] = Value::Null;
                    self.state["photos"] = json!([]);
                    self.state["photoOffset"] = json!(0);
                    self.state["photosMore"] = json!(false);
                    self.state["photoSelection"] = json!([]);
                    self.photo_index.clear();
                    self.photo_checked = None;
                    self.photos_revision += 1;
                }
                self.photo_submit(next)
            }
            _ => vec![],
        }
    }
}

fn checked_thumbnail(photo: &Value) -> Value {
    let value = text(photo, "thumbnail");
    if value.starts_with("data:image/jpeg;base64,") && value.len() <= 710000 {
        json!(value)
    } else {
        json!("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Files {
        Files::new(
            Localization::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../src/neutrondrive/locales"),
                "en",
            )
            .unwrap(),
            "unused",
            false,
        )
    }
    #[test]
    fn automatic_inventory_refresh_retains_pages_but_manual_refresh_reloads() {
        let mut m = model();
        m.photo_index = vec!["a".into()];
        m.state["photoTotal"] = json!(1);
        m.state["photoAlbum"] = json!("");
        m.state["photoOffset"] = json!(1);
        m.state["photos"] = json!([{"uid":"a","thumbnail":"cached","previewLoaded":true}]);
        m.photo_checked = Some(Instant::now() - Duration::from_secs(31));
        let effects = m.enter_photos();
        let Effect::Work(w) = &effects[0] else {
            panic!("index refresh expected")
        };
        assert!(w.operation["request"].get("preserve").is_none());
        m.complete(w.id, Ok(json!({"items":[{"uid":"a"}],"total":1})));
        assert_eq!(m.state["photos"][0]["thumbnail"], "cached");
        assert_eq!(m.state["photoOffset"], 1);
        let effects = m.photo_action(&json!({"action":"index","album":""}));
        let Effect::Work(w) = &effects[0] else {
            panic!("manual refresh expected")
        };
        let next = m.complete(w.id, Ok(json!({"items":[{"uid":"a"}],"total":1})));
        assert!(m.state["photos"].as_array().unwrap().is_empty());
        assert!(matches!(&next[0], Effect::Work(w) if w.operation["request"]["action"]=="details"));
    }

    #[test]
    fn preview_refresh_replaces_changed_metadata_and_clears_missing_photo() {
        let mut m = model();
        m.state["photos"] = json!([
            {"uid":"a","name":"old","safe":true,"location":{"latitude":1},"thumbnail":"old-image"},
            {"uid":"b","safe":true,"location":{"latitude":1},"thumbnail":"deleted-image"}
        ]);
        m.photo_result(&json!({"action":"details","previewStart":0,"ids":["a","b"]}),
            json!({"photos":[{"uid":"a","name":"new","safe":true,"revision":"r2","thumbnail":"data:image/jpeg;base64,/9g="}]}));
        assert_eq!(m.state["photos"][0]["name"], "new");
        assert_eq!(m.state["photos"][0]["revision"], "r2");
        assert!(m.state["photos"][0]["location"].is_null());
        assert_eq!(m.state["photos"][1]["thumbnail"], "");
        assert_eq!(m.state["photos"][1]["safe"], false);
        assert!(m.state["photos"][1]["location"].is_null());
        assert!(m.dispatch("photo_download", &json!([1])).is_empty());
    }

    #[test]
    fn album_membership_changes_reload_active_inventory_after_album_list() {
        for action in ["add", "remove", "delete"] {
            let mut m = model();
            m.state["photoAlbum"] = json!("album-a");
            m.state["photoTotal"] = json!(1);
            m.state["photos"] = json!([{"uid":"old"}]);
            let expected = if action == "delete" { "" } else { "album-a" };
            let effects = m.photo_result(
                &json!({"action":action,"album":"album-a"}),
                json!({"accepted":true}),
            );
            let Effect::Work(w) = &effects[0] else {
                panic!("albums expected")
            };
            assert_eq!(w.operation["request"], json!({"action":"albums"}));
            let effects = m.complete(w.id, Ok(json!({"albums":[]})));
            let Effect::Work(w) = &effects[0] else {
                panic!("index expected")
            };
            assert_eq!(
                w.operation["request"],
                json!({"action":"index","album":expected})
            );
        }
    }

    #[test]
    fn cancelled_album_followup_resumes_correct_album_without_stale_photos() {
        let mut m = model();
        m.state["photoAlbum"] = json!("album-a");
        m.state["photoTotal"] = json!(1);
        m.state["photos"] = json!([{"uid":"removed"}]);
        let effects = m.photo_result(
            &json!({"action":"remove","album":"album-a"}),
            json!({"accepted":true}),
        );
        let Effect::Work(old) = &effects[0] else {
            panic!("albums expected")
        };
        m.dispatch("show_page", &json!(["settings"]));
        assert!(m.state["photos"].as_array().unwrap().is_empty());
        m.complete(old.id, Ok(json!({"albums":[]})));
        let effects = m.dispatch("open_section", &json!(["Photos", "photos"]));
        let Effect::Work(next) = &effects[0] else {
            panic!("index expected")
        };
        assert_eq!(
            next.operation["request"],
            json!({"action":"index","album":"album-a"})
        );
    }

    #[test]
    fn unrelated_album_change_preserves_visible_gallery() {
        let mut m = model();
        m.state["photoAlbum"] = json!("album-a");
        m.state["photoTotal"] = json!(1);
        let photos = json!([{"uid":"keep"}]);
        m.state["photos"] = photos.clone();
        let effects = m.photo_result(
            &json!({"action":"delete","album":"album-b"}),
            json!({"accepted":true}),
        );
        let Effect::Work(w) = &effects[0] else {
            panic!("albums expected")
        };
        assert!(m.complete(w.id, Ok(json!({"albums":[]}))).is_empty());
        assert_eq!(m.state["photos"], photos);
        assert_eq!(m.state["photoAlbum"], "album-a");
    }

    #[test]
    fn inventory_is_atomic_and_first_batch_is_bounded() {
        let mut m = model();
        m.photo_index = vec!["previous".into()];
        m.photo_result(
            &json!({"action":"index"}),
            json!({"items":[{"uid":"x"},{"uid":"x"}],"total":2}),
        );
        assert_eq!(m.photo_index, vec!["previous"]);
        let items: Vec<_> = (0..100).map(|i| json!({"uid":format!("p{i}")})).collect();
        let effects = m.photo_result(
            &json!({"action":"index","album":""}),
            json!({"items":items,"total":100}),
        );
        assert_eq!(m.state["photoTotal"], 100);
        let Effect::Work(w) = &effects[0] else {
            panic!("details work expected")
        };
        assert_eq!(w.operation["request"]["ids"].as_array().unwrap().len(), 48);
    }
    #[test]
    fn missing_photos_advance_inventory_cursor_and_old_previews_are_released() {
        let mut m = model();
        m.photo_index = (0..200).map(|i| format!("p{i}")).collect();
        for start in [0, 48, 96] {
            let ids: Vec<_> = (start..start + 48).map(|i| format!("p{i}")).collect();
            let photos: Vec<_> = ids
                .iter()
                .skip(1)
                .map(|id| json!({"uid":id,"thumbnail":"data:image/jpeg;base64,/9g="}))
                .collect();
            m.photo_result(
                &json!({"action":"details","start":start,"ids":ids}),
                json!({"photos":photos}),
            );
        }
        assert_eq!(m.state["photoOffset"], 144);
        let photos = m.state["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 141);
        assert_eq!(
            photos
                .iter()
                .filter(|p| !text(p, "thumbnail").is_empty())
                .count(),
            96
        );
        assert_eq!(photos[0]["previewLoaded"], false);
        m.photo_result(
            &json!({"action":"details","previewStart":0,"ids":["p1"]}),
            json!({"photos":[{"uid":"p1","thumbnail":"file:///etc/passwd"}]}),
        );
        assert_eq!(m.state["photos"][0]["thumbnail"], "");
        assert_eq!(m.state["photoOffset"], 144);
    }
    #[test]
    fn foreign_or_stale_batch_cannot_replace_photos() {
        let mut m = model();
        m.state["photos"] = json!([{"uid":"keep"}]);
        m.photo_result(
            &json!({"action":"details","start":0,"ids":["expected"]}),
            json!({"photos":[{"uid":"unexpected"}]}),
        );
        assert_eq!(m.state["photos"], json!([{"uid":"keep"}]));
        m.photo_result(
            &json!({"action":"details","start":5,"ids":["expected"]}),
            json!({"photos":[{"uid":"expected"}]}),
        );
        assert_eq!(m.state["photos"], json!([{"uid":"keep"}]));
    }
    #[test]
    fn metadata_order_and_missing_entries_preserve_inventory_timeline() {
        let mut m = model();
        m.photo_index = vec!["a".into(), "missing".into(), "b".into(), "c".into()];
        m.photo_result(
            &json!({"action":"details","start":0,"ids":["a","missing","b","c"]}),
            json!({"photos":[{"uid":"c"},{"uid":"a"},{"uid":"b"}]}),
        );
        let ids: Vec<_> = m.state["photos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| text(p, "uid"))
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(m.state["photoOffset"], 4);
        assert_eq!(m.state["photosMore"], false);
    }
    #[test]
    fn duplicate_metadata_is_rejected_without_changing_inventory_or_previews() {
        for request in [
            json!({"action":"details","start":0,"ids":["a"]}),
            json!({"action":"details","previewStart":0,"ids":["a"]}),
        ] {
            let mut m = model();
            let original = json!([{"uid":"a","thumbnail":"existing","previewLoaded":true}]);
            m.state["photos"] = original.clone();
            let revision = m.photos_revision;
            m.photo_result(&request, json!({"photos":[{"uid":"a"},{"uid":"a"}]}));
            assert_eq!(m.state["photos"], original);
            assert_eq!(m.photos_revision, revision);
            assert_eq!(m.state["photoOffset"], 0);
        }
    }
    #[test]
    fn large_inventory_append_retains_unique_entries() {
        let mut m = model();
        m.state["photos"] = json!((0..10000)
            .map(|i| json!({"uid":format!("p{i}")}))
            .collect::<Vec<_>>());
        m.photo_result(
            &json!({"action":"details","start":0,"ids":["p9999","new"]}),
            json!({"photos":[{"uid":"new"},{"uid":"p9999"}]}),
        );
        let photos = m.state["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 10001);
        assert_eq!(photos[9999]["uid"], "p9999");
        assert_eq!(photos[10000]["uid"], "new");
    }
    #[test]
    fn preview_requests_only_missing_images_and_keeps_nearby_cache() {
        let mut m = model();
        m.state["photos"] = json!((0..200).map(|i| json!({"uid":format!("p{i}"),"previewLoaded":i!=55,"thumbnail":"data:image/jpeg;base64,/9g="})).collect::<Vec<_>>());
        let effects = m.photo_action(&json!({"action":"previews","start":48}));
        let Effect::Work(work) = &effects[0] else {
            panic!("preview work")
        };
        assert_eq!(work.operation["request"]["ids"], json!(["p55"]));
        let revision = m.photos_revision;
        m.photo_result(
            &json!({"action":"details","previewStart":48,"ids":["p55"]}),
            json!({"photos":[{"uid":"p55","thumbnail":"data:image/jpeg;base64,/9g="}]}),
        );
        assert_eq!(m.photos_revision, revision + 1);
        assert_eq!(m.state["photos"][0]["previewLoaded"], true);
        assert_eq!(m.state["photos"][55]["previewLoaded"], true);
        assert_eq!(m.state["photos"][143]["previewLoaded"], true);
        assert_eq!(m.state["photos"][144]["previewLoaded"], false);
    }
}
