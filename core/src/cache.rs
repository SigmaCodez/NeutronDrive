//! Pure metadata invalidation. Paths are opaque SDK identities, not filesystem paths.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invalidation {
    pub roots: Vec<String>,
    pub children: BTreeMap<String, BTreeSet<String>>,
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub parents: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub uid_paths: BTreeMap<String, String>,
    pub node_uid: Option<String>,
    pub parent_uid: Option<String>,
    pub reset: bool,
}

impl Invalidation {
    fn canonical(&self, path: &str) -> String {
        self.aliases
            .get(path)
            .map(String::as_str)
            .unwrap_or(path)
            .to_owned()
    }

    pub fn plan(&self) -> BTreeSet<String> {
        let mut direct = BTreeSet::new();
        let mut subtrees = Vec::new();
        if !self.reset {
            if let Some(uid) = &self.node_uid {
                direct.extend(self.parents.get(uid).into_iter().flatten().cloned());
                if let Some(path) = self.uid_paths.get(uid) {
                    subtrees.push(self.canonical(path));
                    direct.insert(path.clone());
                }
            }
            if let Some(path) = self
                .parent_uid
                .as_ref()
                .and_then(|uid| self.uid_paths.get(uid))
            {
                direct.insert(path.clone());
            }
        }
        if self.reset || direct.is_empty() {
            subtrees.extend(self.roots.iter().map(|path| self.canonical(path)));
        }
        // Iterative traversal handles deep trees, shared descendants and stale cycles.
        let mut seen = BTreeSet::new();
        while let Some(path) = subtrees.pop() {
            if seen.insert(path.clone()) {
                subtrees.extend(self.children.get(&path).into_iter().flatten().cloned());
            }
        }
        seen.extend(direct.iter().map(|path| self.canonical(path)));
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn moved_folder_invalidates_old_new_parents_and_descendants_only() {
        let input: Invalidation = serde_json::from_value(json!({
            "roots":["root"], "children":{"folder":["child"],"child":["folder"]},
            "aliases":{},"parents":{"node":["old"]},
            "uid_paths":{"node":"folder","parent":"new"},
            "node_uid":"node","parent_uid":"parent","reset":false
        }))
        .unwrap();
        assert_eq!(
            input.plan(),
            BTreeSet::from_iter(["child", "folder", "new", "old"].map(String::from))
        );
    }
    #[test]
    fn unknown_event_and_reset_follow_root_alias_without_prefix_matching() {
        for reset in [false, true] {
            let input: Invalidation = serde_json::from_value(json!({
                "roots":["alias"],"children":{"root":["root/child"],"root-other":["unrelated"]},
                "aliases":{"alias":"root"},"node_uid":null,"parent_uid":null,"reset":reset
            }))
            .unwrap();
            assert_eq!(
                input.plan(),
                BTreeSet::from_iter(["root", "root/child"].map(String::from))
            );
        }
    }
}
