//! Compat goldens (DESIGN 7.2): every call line of `fixtures/compat/__golden__/{load,lookup,
//! interpolate}.md`, written by the JS oracle, replayed through the public API and
//! compared file by file. The runner lives in `support/compat.rs`, shared with the wasm
//! test. The goldens are never rewritten from Rust.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/compat.rs"]
mod compat;

/// Given the JS loader's catalog and its recorded answers, when Rust loads the same files
/// and answers the same calls, then every line matches (markers mapped per the README).
#[test]
fn goldens_match_the_js_library() {
    // step 1: the compiled-in fixture list is the directory, nothing forgotten
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compat/locales");
    let mut on_disk = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                on_disk.push(rel);
            }
        }
    }
    on_disk.sort();
    let listed: Vec<_> = compat::FILES.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(listed, on_disk, "update FILES in tests/support/compat.rs");

    // step 2: every golden line → the same answer
    match compat::run() {
        Ok(lines) => assert!(lines > 4000, "only {lines} golden lines ran"),
        Err(diff) => panic!("compat goldens differ:\n{diff}"),
    }
}
