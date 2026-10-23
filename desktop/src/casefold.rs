//! Unicode default full case folding, matching the former Python casefold search.
//! C + F mappings; Turkic T and simple-only S mappings are intentionally excluded.
//! Pinned Unicode 16 data, with its license, is kept in ../data for offline builds.
use std::{collections::BTreeMap, sync::OnceLock};
static MAPPINGS: OnceLock<BTreeMap<char, String>> = OnceLock::new();
pub fn fold(value: &str) -> String {
    let mappings = MAPPINGS.get_or_init(|| {
        include_str!("../data/CaseFolding-16.0.0.txt")
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('#').next()?.split(';').map(str::trim);
                let source = fields.next()?;
                if !matches!(fields.next()?, "C" | "F") {
                    return None;
                }
                let source = char::from_u32(
                    u32::from_str_radix(source, 16).expect("pinned Unicode code point"),
                )
                .expect("Unicode scalar");
                let mapped: String = fields
                    .next()
                    .expect("pinned Unicode mapping")
                    .split_whitespace()
                    .map(|v| {
                        char::from_u32(
                            u32::from_str_radix(v, 16).expect("pinned Unicode code point"),
                        )
                        .expect("Unicode scalar")
                    })
                    .collect();
                Some((source, mapped))
            })
            .collect()
    });
    let mut result = String::with_capacity(value.len());
    for c in value.chars() {
        if let Some(mapped) = mappings.get(&c) {
            result.push_str(mapped);
        } else {
            result.push(c);
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_folding_preserves_existing_multilingual_search_semantics() {
        for (source, expected) in [
            ("Straße", "strasse"),
            ("ﬃ", "ffi"),
            ("Σςσ", "σσσ"),
            ("İIı", "i\u{307}iı"),
            ("Ꭰꭰ", "ᎠᎠ"),
            ("文件📁", "文件📁"),
            ("", ""),
        ] {
            assert_eq!(fold(source), expected);
            assert_eq!(fold(expected), expected);
        }
        assert_eq!(MAPPINGS.get().unwrap().len(), 1557);
    }
}
