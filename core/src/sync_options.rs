//! Persisted per-job scope and UTC scheduling, independent of filesystem I/O.
use crate::process::Failure;
use serde::{Deserialize, Serialize};
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub daily_utc_minute: Option<u16>,
}
impl Options {
    pub fn validate(&self) -> Result<(), Failure> {
        if self.exclusions.len() > 256 || self.daily_utc_minute.is_some_and(|m| m >= 1440) {
            return Err("Invalid sync options".into());
        }
        for path in &self.exclusions {
            if path.len() > 4096
                || path.contains(['\\', '*', '?', '[', ']', '{', '}'])
                || path.chars().any(char::is_control)
                || path
                    .split('/')
                    .any(|p| p.is_empty() || matches!(p, "." | ".."))
            {
                return Err(
                    "Exclusions must be relative paths without wildcards, empty or parent components".into(),
                );
            }
        }
        Ok(())
    }
    pub fn excludes(&self, path: &str) -> bool {
        self.exclusions.iter().any(|p| {
            path == p
                || path
                    .strip_prefix(p)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    }
    pub fn next_after(&self, now: u64) -> Option<u64> {
        self.daily_utc_minute.map(|minute| {
            let today = (now / 86400 * 86400).saturating_add(u64::from(minute) * 60);
            if today > now {
                today
            } else {
                today.saturating_add(86400)
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_is_relative_and_component_bounded() {
        let mut options = Options {
            exclusions: vec!["a/build".into()],
            daily_utc_minute: None,
        };
        assert!(options.validate().is_ok());
        assert!(options.excludes("a/build/cache"));
        assert!(options.excludes("a/build"));
        assert!(!options.excludes("a/builder"));
        assert!(!options.excludes("other/a/build"));
        for invalid in [
            "", "/tmp", "../x", "a/../b", "a//b", "a/", "a\\b", "a\n", "*.tmp",
        ] {
            options.exclusions = vec![invalid.into()];
            assert!(options.validate().is_err(), "{invalid:?}");
        }
    }
    #[test]
    fn daily_deadline_is_strictly_future_and_utc() {
        let options = Options {
            exclusions: vec![],
            daily_utc_minute: Some(60),
        };
        assert_eq!(options.next_after(3599), Some(3600));
        assert_eq!(options.next_after(3600), Some(90000));
        assert_eq!(options.next_after(172801), Some(176400));
        assert_eq!(Options::default().next_after(0), None);
    }
}
