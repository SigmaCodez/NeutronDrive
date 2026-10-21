//! Decode complete Linux inotify reads before the host applies watcher effects.
//! Names remain bytes so non-UTF-8 Linux filenames survive the JSON boundary.
use serde_json::{json, Value};

pub fn decode(data: &[u8]) -> Result<Value, String> {
    if data.len() > 1024 * 1024 {
        return Err("inotify read exceeds 1 MiB".into());
    }
    let mut offset = 0;
    let mut events = Vec::new();
    while offset < data.len() {
        if data.len() - offset < 16 {
            return Err("Truncated inotify header".into());
        }
        let watch = i32::from_ne_bytes(data[offset..offset + 4].try_into().unwrap());
        let mask = u32::from_ne_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        let length =
            u32::from_ne_bytes(data[offset + 12..offset + 16].try_into().unwrap()) as usize;
        offset += 16;
        if length > data.len() - offset {
            return Err("Truncated inotify name".into());
        }
        let field = &data[offset..offset + length];
        offset += length;
        let name = if field.is_empty() {
            field
        } else {
            let end = field
                .iter()
                .position(|&v| v == 0)
                .ok_or("Unterminated inotify name")?;
            &field[..end]
        };
        if name.contains(&b'/') || name == b"." || name == b".." {
            return Err("Unsafe inotify name".into());
        }
        if mask & 0x4000 != 0 {
            events.push(json!({"kind":"overflow"}));
        } else if mask & 0x8000 != 0 {
            events.push(json!({"kind":"ignored","watch":watch}));
        } else if !name.starts_with(b".neutrondrive-") {
            events.push(json!({"kind":"local","watch":watch,"name":name,
                "add_tree":mask & 0x40000000 != 0 && mask & (0x100 | 0x80) != 0,
                "rebuild":mask & (0x400 | 0x800) != 0}));
        }
    }
    Ok(json!(events))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(mask: u32, name: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&7i32.to_ne_bytes());
        data.extend_from_slice(&mask.to_ne_bytes());
        data.extend_from_slice(&0u32.to_ne_bytes());
        data.extend_from_slice(&(name.len() as u32).to_ne_bytes());
        data.extend_from_slice(name);
        data
    }
    #[test]
    fn preserves_names_and_classifies_directory_changes() {
        let result = decode(&event(0x40000100, b"a\xff\0\0")).unwrap();
        assert_eq!(result[0]["name"], json!([97, 255]));
        assert_eq!(result[0]["add_tree"], true);
        assert_eq!(decode(&event(0x800, b"")).unwrap()[0]["rebuild"], true);
    }
    #[test]
    fn overflow_ignored_and_internal_names() {
        let mut data = event(0x4000, b"");
        data.extend(event(0x8000, b""));
        data.extend(event(0x100, b".neutrondrive-temp\0"));
        let result = decode(&data).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
        assert_eq!(result[0]["kind"], "overflow");
        assert_eq!(result[1]["kind"], "ignored");
    }
    #[test]
    fn malformed_batches_fail_before_any_effect() {
        for data in [
            vec![0; 15],
            event(0, b"no-null"),
            event(0, b"../a\0"),
            event(0, b"..\0"),
        ] {
            assert!(decode(&data).is_err());
        }
        let mut data = event(0x100, b"ok\0");
        data.push(1);
        assert!(decode(&data).is_err());
        let mut data = event(0, b"x\0");
        data.pop();
        assert!(decode(&data).is_err());
    }
}
