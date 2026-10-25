//! Photo library orchestration: inventory first, bounded metadata/preview batches afterwards.
use super::*;
impl Files {
    pub(super) fn photo_submit(&mut self, request: Value) -> Vec<Effect> {
        self.submit(
            "Photos",
            json!({"action":"photo_library","request":request}),
            Done::PhotoLibrary(request),
        )
    }
    pub(super) fn photo_action(&mut self, request: &Value) -> Vec<Effect> {
        match text(request, "action") {
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
            "index" => self.photo_submit(json!({"action":"index","album":text(request,"album")})),
            "albums" => self.photo_submit(json!({"action":"albums"})),
            "previews" => {
                let start = request["start"].as_u64().unwrap_or(0) as usize;
                let photos = self.state["photos"].as_array().cloned().unwrap_or_default();
                let ids: Vec<_> = photos
                    .iter()
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
                if let Some(start) = request["previewStart"].as_u64() {
                    let Some(ids) = request["ids"].as_array() else {
                        return vec![];
                    };
                    if photos.len() > 48 || photos.iter().any(|p| !ids.contains(&p["uid"])) {
                        self.message("Invalid photo batch", true);
                        return vec![];
                    }
                    let mut current = self.state["photos"].as_array().cloned().unwrap_or_default();
                    for (index, old) in current.iter_mut().enumerate() {
                        if index < (start as usize).saturating_sub(48)
                            || index >= start as usize + 96
                        {
                            old["thumbnail"] = json!("");
                            old["previewLoaded"] = json!(false);
                        } else if let Some(p) = photos.iter().find(|p| p["uid"] == old["uid"]) {
                            old["thumbnail"] = checked_thumbnail(p);
                            old["previewLoaded"] = json!(true);
                        } else if ids.contains(&old["uid"]) {
                            old["previewLoaded"] = json!(true);
                        }
                    }
                    self.state["photos"] = json!(current);
                    self.photos_revision += 1;
                    return vec![];
                }
                let start = request["start"].as_u64().unwrap_or(u64::MAX);
                let Some(ids) = request["ids"].as_array() else {
                    return vec![];
                };
                if photos.len() > 48
                    || start != self.state["photoOffset"].as_u64().unwrap_or(0)
                    || photos.iter().any(|p| !ids.contains(&p["uid"]))
                {
                    self.message("Invalid photo batch", true);
                    return vec![];
                }
                let mut current = self.state["photos"].as_array().cloned().unwrap_or_default();
                for mut p in photos.iter().cloned() {
                    if current.iter().any(|old| old["uid"] == p["uid"]) {
                        continue;
                    }
                    p["thumbnail"] = checked_thumbnail(&p);
                    p["previewLoaded"] = json!(true);
                    current.push(p);
                }
                let keep_from = current.len().saturating_sub(96);
                for photo in current.iter_mut().take(keep_from) {
                    photo["thumbnail"] = json!("");
                    photo["previewLoaded"] = json!(false);
                }
                self.state["photos"] = json!(current);
                self.photos_revision += 1;
                let next = start + ids.len() as u64;
                self.state["photoOffset"] = json!(next);
                self.state["photosMore"] = json!((next as usize) < self.photo_index.len());
                vec![]
            }
            "albums" => {
                if value["albums"].as_array().is_some_and(|a| a.len() <= 10000) {
                    self.state["photoAlbums"] = value["albums"].clone();
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
                self.photo_submit(json!({"action":"albums"}))
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
