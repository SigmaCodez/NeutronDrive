//! Release ordering. Network I/O remains outside the policy boundary.
pub fn version(value: &str) -> Result<Vec<u64>, String> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let mut parts = value.splitn(3, '.');
    let number = |s: &str| -> Result<u64, String> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Unsupported version".into());
        }
        s.parse().map_err(|_| "Unsupported version".into())
    };
    let major = number(parts.next().unwrap_or(""))?;
    let minor = number(parts.next().unwrap_or(""))?;
    let rest = parts.next().ok_or("Unsupported version")?;
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let patch = number(&rest[..end])?;
    let tail = &rest[end..];
    if tail.is_empty() {
        return Ok(vec![major, minor, patch, 3, 0]);
    }
    let tail = tail
        .strip_prefix('-')
        .or_else(|| tail.strip_prefix('.'))
        .unwrap_or(tail);
    for (stage, rank) in [("alpha", 0), ("beta", 1), ("rc", 2), ("a", 0), ("b", 1)] {
        if let Some(tail) = tail.strip_prefix(stage) {
            let tail = tail
                .strip_prefix('-')
                .or_else(|| tail.strip_prefix('.'))
                .unwrap_or(tail);
            return Ok(vec![major, minor, patch, rank, number(tail)?]);
        }
    }
    Err("Unsupported version".into())
}
pub fn choose(
    releases: serde_json::Value,
    current: &str,
    preview: bool,
    format: &str,
    machine: &str,
) -> Result<serde_json::Value, String> {
    use serde_json::{json, Value};
    const REPO: &str = "https://github.com/SigmaCodez/NeutronDrive";
    let mut best = version(current)?;
    let mut selected = Value::Null;
    for item in releases.as_array().ok_or("Invalid release response")? {
        if !item.is_object() || item["draft"] == true || !item["published_at"].is_string() {
            continue;
        }
        let Some(tag) = item["tag_name"].as_str() else {
            continue;
        };
        let Ok(key) = version(tag) else {
            continue;
        };
        if key <= best || (!preview && (item["prerelease"] == true || key[3] < 3)) {
            continue;
        }
        let url = format!("{REPO}/releases/tag/{tag}");
        if item["html_url"].as_str() != Some(&url) {
            continue;
        }
        let mut result = json!({"tag":tag, "url":url, "asset_url":"", "digest":"", "size":0});
        let arch = match machine {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            _ => "",
        };
        if format == "deb" && !arch.is_empty() {
            let name = format!(
                "neutrondrive_{}-1_{arch}.deb",
                tag.strip_prefix('v').unwrap_or(tag)
            );
            let url = format!("{REPO}/releases/download/{tag}/{name}");
            if let Some(assets) = item["assets"].as_array() {
                for asset in assets {
                    let Some(digest) = asset["digest"]
                        .as_str()
                        .and_then(|s| s.strip_prefix("sha256:"))
                    else {
                        continue;
                    };
                    let size = asset["size"].as_u64().unwrap_or(0);
                    if asset["name"].as_str() == Some(&name)
                        && asset["browser_download_url"].as_str() == Some(&url)
                        && digest.len() == 64
                        && digest
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        && size > 0
                        && size <= 512 * 1024 * 1024
                    {
                        result["asset_url"] = json!(url);
                        result["digest"] = json!(digest);
                        result["size"] = json!(size);
                        break;
                    }
                }
            }
        }
        best = key;
        selected = result;
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_and_invalid_versions() {
        assert!(version("v0.6.0-alpha10").unwrap() > version("0.6.0a9").unwrap());
        assert!(version("0.6.0").unwrap() > version("0.6.0-rc99").unwrap());
        for value in [
            "",
            "1.2",
            "1.2.3-",
            "1.2.3evil",
            "1.2.3-alpha",
            "1.2.3-alpha1+local",
        ] {
            assert!(version(value).is_err());
        }
    }
}
