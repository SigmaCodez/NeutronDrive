//! Memory views backed by worker-side persistent snapshots; background reconciliation.
use super::*;
use std::collections::{HashMap, HashSet};
impl Files {
    pub fn refresh_cache(&mut self) -> Vec<Effect> {
        if self.busy()
            || self.pending.is_some()
            || self.state["demo"] == true
            || self.state["authenticated"] != true
            || self
                .cache_checked
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        {
            return vec![];
        }
        let operation = if self.state["page"] == "photos" && self.state["photoTotal"].is_u64() {
            let photos = self.state["photos"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let known: HashSet<_> = photos.iter().filter_map(|p| p["uid"].as_str()).collect();
            let end = (self.state["photoOffset"].as_u64().unwrap_or(0) as usize)
                .min(self.photo_index.len());
            let missing: Vec<_> = self.photo_index[..end]
                .iter()
                .filter(|id| !known.contains(id.as_str()))
                .take(48)
                .cloned()
                .collect();
            if self
                .photo_checked
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
            {
                json!({"action":"photo_library","request":{"action":"index","album":self.state["photoAlbum"]}})
            } else if !missing.is_empty() {
                json!({"action":"photo_library","request":{"action":"details","ids":missing}})
            } else if self.cache_turn.is_multiple_of(3) {
                json!({"action":"photo_library","request":{"action":"albums"}})
            } else if !photos.is_empty() {
                let start = ((self.cache_turn / 3) * 48) % photos.len();
                let ids: Vec<_> = photos
                    .iter()
                    .skip(start)
                    .take(48)
                    .map(|p| p["uid"].clone())
                    .collect();
                json!({"action":"photo_library","request":{"action":"details","ids":ids}})
            } else {
                return vec![];
            }
        } else {
            let path = self
                .folder_cache
                .iter()
                .filter(|c| c.saved.elapsed() >= Duration::from_secs(30))
                .min_by_key(|c| (c.path != self.path(), c.saved))
                .map(|c| c.path.clone());
            let Some(path) = path else {
                return vec![];
            };
            json!({"action":"list","path":path})
        };
        self.cache_checked = Some(Instant::now());
        self.cache_turn += 1;
        vec![Effect::PollCache(
            json!({"operation":operation,"album":self.state["photoAlbum"]}),
        )]
    }
    pub fn cache_polled(&mut self, request: &Value, generation: u64, value: Value) {
        if self.busy() || generation != self.next_work || self.state["authenticated"] != true {
            return;
        }
        let operation = &request["operation"];
        if operation["action"] == "list" {
            let Some(nodes) = value.as_array() else {
                return;
            };
            let path = text(operation, "path");
            let mut nodes = nodes.clone();
            nodes.sort_by_cached_key(|n| (n["type"] != "folder", folded(text(n, "name"))));
            if path == self.path() {
                let selected: HashSet<_> = self
                    .selected()
                    .iter()
                    .map(|n| text(n, "uid").to_owned())
                    .collect();
                let anchor = self
                    .selection_anchor
                    .and_then(|i| self.visible.get(i))
                    .map(|n| text(n, "uid").to_owned());
                let focus = self
                    .visible
                    .get(self.state["focusIndex"].as_u64().unwrap_or(0) as usize)
                    .map(|n| text(n, "uid").to_owned());
                self.nodes = nodes;
                self.cache_folder();
                self.render();
                self.selection = self
                    .visible
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        !text(n, "uid").is_empty() && selected.contains(text(n, "uid"))
                    })
                    .map(|(i, _)| i)
                    .collect();
                if let Some(i) =
                    focus.and_then(|id| self.visible.iter().position(|n| n["uid"] == id))
                {
                    self.state["focusIndex"] = json!(i);
                }
                self.selection_anchor =
                    anchor.and_then(|id| self.visible.iter().position(|n| n["uid"] == id));
                self.update_actions();
            } else if self.folder_cache.iter().any(|c| c.path == path) {
                self.store_folder(path.to_owned(), nodes);
            }
            return;
        }
        if self.state["page"] != "photos" || request["album"] != self.state["photoAlbum"] {
            return;
        }
        let action = text(&operation["request"], "action");
        if action == "albums" {
            if value["albums"].as_array().is_some_and(|a| a.len() <= 10000) {
                self.state["photoAlbums"] = value["albums"].clone();
            }
            return;
        }
        if action == "index" {
            let Some(items) = value["items"].as_array() else {
                return;
            };
            let ids: Vec<String> = items.iter().map(|p| text(p, "uid").to_owned()).collect();
            if ids.len() > 500000
                || value["total"].as_u64() != Some(ids.len() as u64)
                || ids.iter().any(String::is_empty)
                || ids.iter().collect::<HashSet<_>>().len() != ids.len()
            {
                return;
            }
            self.photo_checked = Some(Instant::now());
            if ids == self.photo_index {
                return;
            }
            // Move existing thumbnails instead of cloning the entire image cache.
            let old = std::mem::replace(&mut self.state["photos"], json!([]));
            let old = match old {
                Value::Array(v) => v,
                _ => vec![],
            };
            let mut photos: HashMap<String, Value> = old
                .into_iter()
                .map(|p| (text(&p, "uid").to_owned(), p))
                .collect();
            let end = ids
                .iter()
                .rposition(|id| photos.contains_key(id))
                .map_or(48.min(ids.len()), |i| i + 1);
            let updated: Vec<_> = ids.iter().filter_map(|id| photos.remove(id)).collect();
            self.photo_index = ids;
            self.state["photoOffset"] = json!(end);
            self.state["photoTotal"] = json!(self.photo_index.len());
            self.state["photosMore"] = json!(end < self.photo_index.len());
            self.state["photos"] = Value::Array(updated);
            self.photos_revision += 1;
        } else if action == "details" {
            let Some(items) = value["photos"].as_array() else {
                return;
            };
            let Some(ids) = operation["request"]["ids"].as_array() else {
                return;
            };
            if ids.len() > 48
                || items.len() > 48
                || items.iter().any(|p| !ids.contains(&p["uid"]))
                || items
                    .iter()
                    .map(|p| text(p, "uid"))
                    .collect::<HashSet<_>>()
                    .len()
                    != items.len()
            {
                return;
            }
            let Some(photos) = self.state["photos"].as_array_mut() else {
                return;
            };
            let mut changed = false;
            for id in ids.iter().filter_map(Value::as_str) {
                if !self.photo_index.iter().any(|i| i == id) {
                    continue;
                }
                let mut next = items
                    .iter()
                    .find(|p| p["uid"] == id)
                    .cloned()
                    .unwrap_or_else(|| json!({"uid":id,"safe":false,"location":null}));
                next["thumbnail"] = super::photo_library::checked_thumbnail(&next);
                next["previewLoaded"] = json!(true);
                if let Some(old) = photos.iter_mut().find(|p| p["uid"] == id) {
                    if *old != next {
                        *old = next;
                        changed = true;
                    }
                } else {
                    photos.push(next);
                    changed = true;
                }
            }
            if changed {
                let positions: HashMap<_, _> = self
                    .photo_index
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (id.as_str(), i))
                    .collect();
                photos
                    .sort_by_key(|p| positions.get(text(p, "uid")).copied().unwrap_or(usize::MAX));
                self.photos_revision += 1;
            }
        } else {
            return;
        }
        self.trim_photo_memory(
            operation["request"]["ids"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
    }
    pub(super) fn trim_photo_memory(&mut self, protected: &[Value]) {
        self.trim_photo_memory_to(128 * 1024 * 1024, protected);
    }
    fn trim_photo_memory_to(&mut self, limit: usize, protected: &[Value]) {
        let Some(items) = self.state["photos"].as_array_mut() else {
            return;
        };
        let mut bytes: usize = items
            .iter()
            .map(|p| p["thumbnail"].as_str().map_or(0, str::len))
            .sum();
        for p in items.iter_mut() {
            if bytes <= limit {
                break;
            }
            if protected.contains(&p["uid"]) {
                continue;
            }
            self.photos_revision += 1;
            bytes = bytes.saturating_sub(p["thumbnail"].as_str().map_or(0, str::len));
            p["thumbnail"] = json!("");
            p["previewLoaded"] = json!(false);
        }
    }
    pub(super) fn save_photo_view(&mut self) {
        if !self.state["photoTotal"].is_u64() {
            return;
        }
        let album = text(&self.state, "photoAlbum").to_owned();
        self.photo_views.retain(|(id, _, _)| id != &album);
        let value = json!({"photos":self.state["photos"],"offset":self.state["photoOffset"],"more":self.state["photosMore"]});
        self.photo_views
            .push_back((album, self.photo_index.clone(), value));
        while self.photo_views.len() > 8
            || self
                .photo_views
                .iter()
                .map(|(_, ids, v)| serde_json::to_vec(&(ids, v)).map_or(usize::MAX, |v| v.len()))
                .sum::<usize>()
                > 64 * 1024 * 1024
        {
            self.photo_views.pop_front();
        }
    }
    pub(super) fn restore_photo_view(&mut self, album: &str) -> bool {
        let Some((_, ids, v)) = self
            .photo_views
            .iter()
            .find(|(id, _, _)| id == album)
            .cloned()
        else {
            return false;
        };
        self.photo_index = ids;
        self.state["photoTotal"] = json!(self.photo_index.len());
        self.state["photoAlbum"] = json!(album);
        self.state["photos"] = v["photos"].clone();
        self.state["photoOffset"] = v["offset"].clone();
        self.state["photosMore"] = v["more"].clone();
        self.photos_revision += 1;
        self.photo_checked = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Files {
        let locale = Localization::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/neutrondrive/locales"),
            "en",
        )
        .unwrap();
        let mut m = Files::new(locale, "/nonexistent/sdk", false);
        m.state["authenticated"] = json!(true);
        m.state["page"] = json!("photos");
        m.state["photoAlbum"] = json!("");
        m.photo_index = vec!["a".into(), "b".into()];
        m.state["photos"] = json!([{"uid":"a","safe":true,"thumbnail":"data:image/jpeg;base64,YQ==","previewLoaded":true},{"uid":"b","safe":true}]);
        m.state["photoTotal"] = json!(2);
        m.state["photoOffset"] = json!(2);
        m
    }
    fn request(action: &str) -> Value {
        json!({"album":"","operation":{"action":"photo_library","request":{"action":action,"album":"","ids":["a","c"]}}})
    }
    #[test]
    fn disk_hit_is_displayed_and_scheduled_for_background_validation() {
        let mut m = model();
        m.state["page"] = json!("files");
        let effects = m.refresh();
        let Effect::Work(work) = &effects[0] else {
            panic!("Expected reader")
        };
        m.progress(work.id, json!({"kind":"cache_hit"}));
        m.complete(
            work.id,
            Ok(json!([{"uid":"a","name":"Cached","safe":true}])),
        );
        assert!(!m.busy());
        assert_eq!(m.entries[0]["name"], "Cached");
        assert!(m.folder_cache.back().unwrap().saved.elapsed() >= Duration::from_secs(30));
        assert!(matches!(
            m.refresh_cache().as_slice(),
            [Effect::PollCache(_)]
        ));
        let effects = m.dispatch("action", &json!(["refresh"]));
        let Effect::Work(work) = &effects[0] else {
            panic!("Expected fresh reader")
        };
        assert_eq!(work.operation["_fresh"], true);
    }
    #[test]
    fn login_drops_previous_account_memory_views() {
        let mut m = model();
        m.store_folder("/my-files".into(), vec![json!({"uid":"old"})]);
        m.save_photo_view();
        m.dispatch("action", &json!(["login"]));
        assert!(m.folder_cache.is_empty());
        assert!(m.photo_views.is_empty());
        assert!(m.state["photos"].as_array().unwrap().is_empty());
        assert!(m.state["photoTotal"].is_null());
    }
    #[test]
    fn inventory_diff_retains_images_and_rejects_late_or_invalid_results() {
        let mut m = model();
        let r = request("index");
        m.cache_polled(&r, m.next_work + 1, json!({"items":[],"total":0}));
        assert_eq!(m.photo_index, vec!["a", "b"]);
        m.cache_polled(
            &r,
            m.next_work,
            json!({"items":[{"uid":"a"},{"uid":"a"}],"total":2}),
        );
        assert_eq!(m.photo_index, vec!["a", "b"]);
        m.cache_polled(
            &r,
            m.next_work,
            json!({"items":[{"uid":"c"},{"uid":"a"}],"total":2}),
        );
        assert_eq!(m.photo_index, vec!["c", "a"]);
        assert_eq!(
            m.state["photos"][0]["thumbnail"],
            "data:image/jpeg;base64,YQ=="
        );
        assert_eq!(m.state["photos"].as_array().unwrap().len(), 1);
        let revision = m.photos_revision;
        m.cache_polled(
            &r,
            m.next_work,
            json!({"items":[{"uid":"c"},{"uid":"a"}],"total":2}),
        );
        assert_eq!(m.photos_revision, revision);
        m.state["authenticated"] = json!(false);
        m.cache_polled(&r, m.next_work, json!({"items":[],"total":0}));
        assert_eq!(m.photo_index, vec!["c", "a"]);
    }
    #[test]
    fn background_details_merge_and_revoke_without_repeated_notifications() {
        let mut m = model();
        m.photo_index = vec!["c".into(), "a".into()];
        m.state["photos"].as_array_mut().unwrap().pop();
        let r = request("details");
        let value =
            json!({"photos":[{"uid":"c","safe":true,"thumbnail":"data:image/jpeg;base64,Yw=="}]});
        m.cache_polled(&r, m.next_work, value.clone());
        assert_eq!(m.state["photos"][0]["uid"], "c");
        assert_eq!(m.state["photos"][1]["safe"], false);
        assert_eq!(m.state["photos"][1]["thumbnail"], "");
        let revision = m.photos_revision;
        m.cache_polled(&r, m.next_work, value);
        assert_eq!(m.photos_revision, revision);
        assert!(!m.busy());
    }
    #[test]
    fn album_cache_restores_immediately_and_is_bounded() {
        let mut m = model();
        m.save_photo_view();
        m.state["photos"] = json!([]);
        m.photo_index.clear();
        assert!(m.restore_photo_view(""));
        assert_eq!(m.state["photos"][0]["uid"], "a");
        for i in 0..10 {
            m.state["photoAlbum"] = json!(i.to_string());
            m.save_photo_view();
        }
        assert_eq!(m.photo_views.len(), 8);
        assert!(!m.restore_photo_view(""));
    }
    #[test]
    fn cache_budget_keeps_current_previews_and_evicts_other_images() {
        let mut m = model();
        m.state["photos"] = json!([{"uid":"a","thumbnail":"12345678","previewLoaded":true},{"uid":"b","thumbnail":"12345678","previewLoaded":true}]);
        m.trim_photo_memory_to(10, &[json!("a")]);
        assert_eq!(m.state["photos"][0]["thumbnail"], "12345678");
        assert_eq!(m.state["photos"][1]["thumbnail"], "");
        assert_eq!(m.state["photos"][1]["previewLoaded"], false);
    }
    #[test]
    fn file_changes_preserve_selected_identity_focus_and_shift_anchor() {
        let mut m = model();
        m.state["page"] = json!("files");
        m.state["path"] = json!("/my-files");
        m.nodes = vec![
            json!({"uid":"b","name":"b","safe":true}),
            json!({"uid":"c","name":"c","safe":true}),
        ];
        m.render();
        m.select(0, false);
        m.cache_polled(
            &json!({"operation":{"action":"list","path":"/my-files"}}),
            m.next_work,
            json!([{"uid":"a","name":"a","safe":true},{"uid":"b","name":"b","safe":true}]),
        );
        assert_eq!(m.state["selection"], json!([1]));
        assert_eq!(m.state["focusIndex"], 1);
        assert_eq!(m.selection_anchor, Some(1));
        assert_eq!(m.visible.len(), 2);
    }
    #[test]
    fn inactive_folder_can_grow_without_replacing_current_view() {
        let mut m = model();
        m.store_folder("/my-files/other".into(), vec![]);
        m.cache_polled(
            &json!({"operation":{"action":"list","path":"/my-files/other"}}),
            m.next_work,
            json!([{"uid":"new","name":"new","safe":true}]),
        );
        assert_eq!(m.folder_cache.back().unwrap().nodes[0]["uid"], "new");
        assert!(m.nodes.is_empty());
    }
}
