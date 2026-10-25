//! Visibility predicate for desktop-entry based file-manager actions.
fn main() {
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
