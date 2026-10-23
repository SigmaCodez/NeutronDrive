//! Offline locale/catalog handling. Substitutions are data, never format programs.
use crate::process::Failure;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
pub const SUPPORTED: &[&str] = &[
    "en", "de", "fr", "nl", "es", "es-419", "it", "pl", "pt-BR", "ru", "tr", "ca-ES", "cs-CZ",
    "da-DK", "hu", "id", "nb-NO", "pt", "ro", "sk", "sv-SE", "el-GR", "be-BY", "zh-CN", "zh-TW",
];
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Supported,
    Match {
        value: String,
    },
    Select {
        environment: BTreeMap<String, String>,
        ui_languages: Vec<String>,
    },
    Placeholders {
        text: String,
    },
    Valid {
        source: String,
        target: String,
    },
    Catalog {
        directory: String,
        code: String,
    },
    Translate {
        source: String,
        english: Map<String, Value>,
        translated: Map<String, Value>,
    },
    Diagnostic {
        text: String,
        english: Map<String, Value>,
        translated: Map<String, Value>,
    },
}
pub fn run(operation: Operation) -> Result<Value, Failure> {
    Ok(match operation {
        Operation::Supported => json!(SUPPORTED),
        Operation::Match { value } => json!(matching(&value)),
        Operation::Select {
            environment,
            ui_languages,
        } => json!(select(&environment, &ui_languages)),
        Operation::Placeholders { text } => {
            json!(fields(&text)?.into_iter().map(|f| f.0).collect::<Vec<_>>())
        }
        Operation::Valid { source, target } => json!(valid(&source, &target)),
        Operation::Catalog { directory, code } => {
            Value::Object(catalog(Path::new(&directory), &code)?)
        }
        Operation::Translate {
            source,
            english,
            translated,
        } => json!(translate(&source, &english, &translated)),
        Operation::Diagnostic {
            text,
            english,
            translated,
        } => json!(diagnostic(&text, &english, &translated)),
    })
}
fn matching(value: &str) -> Option<&'static str> {
    let value = value.split('.').next().unwrap_or("").replace('_', "-");
    let value = value.split('@').next().unwrap_or("").to_lowercase();
    let parts: Vec<_> = value.split('-').collect();
    let base = parts[0];
    Some(match base {
        "c" | "posix" | "" => "en",
        "zh" => {
            if parts.contains(&"hant")
                || (!parts.contains(&"hans")
                    && parts.iter().any(|p| ["tw", "hk", "mo"].contains(p)))
            {
                "zh-TW"
            } else {
                "zh-CN"
            }
        }
        "es" if parts.iter().skip(1).any(|p| {
            [
                "ar", "bo", "cl", "co", "cr", "cu", "do", "ec", "gt", "hn", "mx", "ni", "pa", "py",
                "pe", "pr", "sv", "uy", "ve", "419",
            ]
            .contains(p)
        }) =>
        {
            "es-419"
        }
        "pt" if parts.contains(&"br") => "pt-BR",
        "ca" => "ca-ES",
        "cs" => "cs-CZ",
        "da" => "da-DK",
        "nb" | "no" => "nb-NO",
        "sv" => "sv-SE",
        "el" => "el-GR",
        "be" => "be-BY",
        _ => *SUPPORTED.iter().find(|s| **s == base)?,
    })
}
fn select(env: &BTreeMap<String, String>, ui: &[String]) -> &'static str {
    let effective = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| env.get(*k).filter(|v| !v.is_empty()));
    if effective.is_some_and(|v| {
        matches!(
            v.split('.').next().unwrap_or("").to_uppercase().as_str(),
            "C" | "POSIX"
        )
    }) {
        return "en";
    }
    let preferred: Vec<_> = env
        .get("LANGUAGE")
        .map(|v| v.split(':').filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let preferred = if preferred.is_empty() {
        if let Some(v) = effective {
            vec![v.as_str()]
        } else {
            ui.iter().map(String::as_str).collect()
        }
    } else {
        preferred
    };
    preferred.into_iter().find_map(matching).unwrap_or("en")
}
// Keep conversions/specifications in the comparison so optional catalogs cannot
// introduce attribute traversal, huge widths or conversions absent from source.
#[derive(Debug, PartialEq)]
enum Token {
    Literal(String),
    Field(String, String, String),
}
fn parse(text: &str) -> Result<Vec<Token>, Failure> {
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' || c == '}' {
            if chars.peek() == Some(&c) {
                chars.next();
                literal.push(c);
                continue;
            }
            if c == '}' {
                return Err("Invalid translation template".into());
            }
            if !literal.is_empty() {
                tokens.push(Token::Literal(std::mem::take(&mut literal)));
            }
            let mut field = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '}' {
                    closed = true;
                    break;
                }
                if c == '{' {
                    return Err("Nested translation fields are unsupported".into());
                }
                field.push(c);
            }
            if !closed {
                return Err("Invalid translation template".into());
            }
            let (head, spec) = field.split_once(':').unwrap_or((&field, ""));
            let (name, conversion) = head.split_once('!').unwrap_or((head, ""));
            tokens.push(Token::Field(name.into(), spec.into(), conversion.into()));
        } else {
            literal.push(c);
        }
    }
    if !literal.is_empty() {
        tokens.push(Token::Literal(literal));
    }
    Ok(tokens)
}
fn fields(text: &str) -> Result<Vec<(String, String, String)>, Failure> {
    match parse(text) {
        Ok(tokens) => {
            let mut result: Vec<_> = tokens
                .into_iter()
                .filter_map(|t| match t {
                    Token::Field(a, b, c) => Some((a, b, c)),
                    _ => None,
                })
                .collect();
            result.sort();
            Ok(result)
        }
        Err(error) => {
            if text
                .as_bytes()
                .windows(3)
                .any(|w| w[0] == b'{' && w[1] == b'p' && w[2].is_ascii_digit())
            {
                Err(error)
            } else {
                Ok(vec![])
            }
        }
    }
}
fn valid(source: &str, target: &str) -> bool {
    match (fields(source), fields(target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
fn catalog(directory: &Path, code: &str) -> Result<Map<String, Value>, Failure> {
    let code = if SUPPORTED.contains(&code) {
        code
    } else {
        "en"
    };
    let load = || -> Result<Map<String, Value>, Failure> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(directory.join(format!("{code}.json")))
            .map_err(|_| "UI catalog is missing")?;
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect UI catalog")?
            .is_file()
        {
            return Err("UI catalog must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read UI catalog")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("UI catalog is too large".into());
        }
        let value: Map<String, Value> =
            serde_json::from_slice(&bytes).map_err(|_| "UI catalog is invalid")?;
        let mut filtered = Map::new();
        for (source, text) in value {
            if text
                .as_str()
                .is_some_and(|t| !t.is_empty() && valid(&source, t))
            {
                filtered.insert(source, text);
            } else if code == "en" {
                return Err("The English UI catalog contains invalid entries.".into());
            }
        }
        Ok(filtered)
    };
    match load() {
        Ok(v) => Ok(v),
        Err(_) if code != "en" => Ok(Map::new()),
        Err(e) => Err(e),
    }
}
fn translate<'a>(
    source: &'a str,
    english: &'a Map<String, Value>,
    translated: &'a Map<String, Value>,
) -> &'a str {
    translated
        .get(source)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && valid(source, s))
        .or_else(|| {
            english
                .get(source)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or(source)
}
fn captures(
    tokens: &[Token],
    text: &str,
    values: &mut BTreeMap<String, String>,
    budget: &mut usize,
) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let Some((first, rest)) = tokens.split_first() else {
        return text.is_empty();
    };
    match first {
        Token::Literal(literal) => text
            .strip_prefix(literal)
            .is_some_and(|tail| captures(rest, tail, values, budget)),
        Token::Field(name, spec, conversion) => {
            if !spec.is_empty()
                || !conversion.is_empty()
                || !name
                    .strip_prefix('p')
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                return false;
            }
            for end in text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
            {
                let mut attempt = values.clone();
                attempt.insert(name.clone(), text[..end].into());
                if captures(rest, &text[end..], &mut attempt, budget) {
                    *values = attempt;
                    return true;
                }
                if *budget == 0 {
                    break;
                }
            }
            false
        }
    }
}
fn render(tokens: Vec<Token>, values: &BTreeMap<String, String>) -> Option<String> {
    let mut text = String::new();
    for token in tokens {
        match token {
            Token::Literal(s) => text.push_str(&s),
            Token::Field(name, spec, conversion) => {
                if !spec.is_empty() || !conversion.is_empty() {
                    return None;
                }
                text.push_str(values.get(&name)?);
            }
        }
    }
    Some(text)
}
fn diagnostic(text: &str, english: &Map<String, Value>, translated: &Map<String, Value>) -> String {
    if english.contains_key(text) {
        return translate(text, english, translated).into();
    }
    // Bound work on arbitrary SDK diagnostics; unknown messages remain literal.
    if text.len() > 65536 {
        return text.into();
    }
    for source in english.keys() {
        if !source.contains('{') {
            continue;
        }
        let Ok(tokens) = parse(source) else { continue };
        if !tokens
            .iter()
            .any(|t| matches!(t,Token::Literal(s) if s.trim().chars().count()>8))
        {
            continue;
        }
        if tokens.len() > 64 {
            continue;
        }
        let mut values = BTreeMap::new();
        if captures(&tokens, text, &mut values, &mut 100000) {
            if let Ok(tokens) = parse(translate(source, english, translated)) {
                if let Some(result) = render(tokens, &values) {
                    return result;
                }
            }
        }
    }
    text.into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_values_cannot_be_format_programs() {
        let source = "Interner Fehler ({p0}). Es wurde kein Erfolg angenommen.";
        let english = Map::from_iter([(
            source.into(),
            json!("Internal error ({p0}). The operation was not marked as successful."),
        )]);
        assert_eq!(
            diagnostic(
                "Interner Fehler ({p1.__class__}). Es wurde kein Erfolg angenommen.",
                &english,
                &Map::new()
            ),
            "Internal error ({p1.__class__}). The operation was not marked as successful."
        );
        assert!(!valid("{p0}", "{p0:1000000000}"));
        assert!(!valid("{p0}", "{p0.__class__}"));
    }
    #[test]
    fn regions_and_environment_precedence() {
        assert_eq!(matching("zh_Hans_TW"), Some("zh-CN"));
        assert_eq!(matching("es_MX"), Some("es-419"));
        assert_eq!(
            select(
                &BTreeMap::from([
                    ("LANG".into(), "C.UTF-8".into()),
                    ("LANGUAGE".into(), "de".into())
                ]),
                &[]
            ),
            "en"
        );
    }
    #[test]
    fn escaped_braces_and_unicode_values() {
        let tokens = parse("Prefix {{literal}} {p0} suffix {p1}").unwrap();
        let mut values = BTreeMap::new();
        assert!(captures(
            &tokens,
            "Prefix {literal} ü😀 suffix あ",
            &mut values,
            &mut 10000
        ));
        assert_eq!(values["p0"], "ü😀");
        assert_eq!(values["p1"], "あ");
    }
}
