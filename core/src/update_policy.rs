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
        let name = match (format, machine) {
            ("deb", "x86_64" | "aarch64") => {
                let arch = if machine == "x86_64" {
                    "amd64"
                } else {
                    "arm64"
                };
                Some(format!(
                    "neutrondrive_{}-1_{arch}.deb",
                    tag.trim_start_matches('v')
                ))
            }
            ("rpm-fedora" | "rpm-opensuse", "x86_64" | "aarch64") => {
                let distro = format.strip_prefix("rpm-").unwrap();
                Some(format!(
                    "neutrondrive-{}-{distro}-{machine}.rpm",
                    tag.trim_start_matches('v')
                ))
            }
            _ => None,
        };
        if let Some(name) = name {
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
    fn rpm_selection_keeps_distribution_architecture_and_digest() {
        use serde_json::json;
        let repo = "https://github.com/SigmaCodez/NeutronDrive";
        let tag = "v0.6.0-alpha16";
        let assets: Vec<_> = ["fedora", "opensuse"].into_iter().flat_map(|distro| {
            ["x86_64", "aarch64"].into_iter().map(move |arch| {
                let name = format!("neutrondrive-0.6.0-alpha16-{distro}-{arch}.rpm");
                json!({"name":name,"browser_download_url":format!("{repo}/releases/download/{tag}/{name}"),"digest":format!("sha256:{}","a".repeat(64)),"size":7})
            })
        }).collect();
        let releases = json!([{"tag_name":tag,"html_url":format!("{repo}/releases/tag/{tag}"),"published_at":"2026-10-25","prerelease":true,"assets":assets}]);
        for distro in ["fedora", "opensuse"] {
            for arch in ["x86_64", "aarch64"] {
                let result = choose(
                    releases.clone(),
                    "0.6.0-alpha15",
                    true,
                    &format!("rpm-{distro}"),
                    arch,
                )
                .unwrap();
                assert!(result["asset_url"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("-{distro}-{arch}.rpm")));
                assert_eq!(result["digest"], "a".repeat(64));
            }
        }
        for (format, arch) in [
            ("rpm", "x86_64"),
            ("rpm-fedora", "unknown"),
            ("deb", "x86_64"),
        ] {
            assert_eq!(
                choose(releases.clone(), "0.6.0-alpha15", true, format, arch).unwrap()["asset_url"],
                ""
            );
        }
        let mut bad = releases;
        bad[0]["assets"][0]["digest"] = json!("bad");
        assert_eq!(
            choose(bad, "0.6.0-alpha15", true, "rpm-fedora", "x86_64").unwrap()["asset_url"],
            ""
        );
    }
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
