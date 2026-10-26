//! Visibility predicate for desktop-entry based file-manager actions.
fn main() {
    let mode: Vec<String> = std::env::args().skip(1).collect();
    if mode.as_slice() == ["--apply-root-icon"] {
        let result = neutrondrive_file_manager::paths::configured_root()
            .ok_or_else(|| "No configured root".to_string())
            .and_then(|root| neutrondrive_file_manager::root_icon::apply(&root));
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if let [flag, parent] = mode.as_slice() {
        if flag == "--watch-emblems" {
            let result = parent
                .parse::<u32>()
                .map_err(|e| e.to_string())
                .and_then(|id| {
                    if id > 1 {
                        neutrondrive_file_manager::watcher::run(id)
                    } else {
                        Err("Invalid parent".into())
                    }
                });
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
    }
    if let [flag, keep, path] = mode.as_slice() {
        if flag == "--apply-emblem" {
            let valid = neutrondrive_file_manager::paths::configured_root().is_some_and(|r| {
                neutrondrive_file_manager::paths::relative(&r, std::path::Path::new(path)).is_some()
            });
            if !valid || !["0", "1", "2"].contains(&keep.as_str()) {
                std::process::exit(2)
            }
            std::process::exit(
                if neutrondrive_file_manager::gio_emblems::apply(path, keep.parse::<u8>().unwrap())
                    .is_ok()
                {
                    0
                } else {
                    1
                },
            );
        }
    }
    let root = neutrondrive_file_manager::paths::configured_root();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let valid = root.is_some_and(|root| {
        !args.is_empty()
            && args.len() <= 128
            && args.iter().all(|path| {
                neutrondrive_file_manager::paths::relative(&root, std::path::Path::new(path))
                    .is_some()
            })
    });
    std::process::exit(if valid { 0 } else { 1 });
}
