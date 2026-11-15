//! Bench inventory (DESIGN 7.7, PLAN): every public function of a crate has a criterion
//! bench. Shared by `tests/bench_inventory.rs` of both crates via `#[path]` (no extra crate).
//!
//! Public functions are `pub fn NAME` (not `pub(crate)` / `pub(super)`) in `src/**/*.rs`.
//! Bench ids are `group/function/case`: `bench_function("group/NAME/case", ..)`, or a
//! criterion group `benchmark_group("group/NAME")` whose cases supply the last segment.
//! Only the function segment counts, so a group or case that happens to equal a function
//! name does not cover it. A malformed id fails the check, so the naming stays uniform.
//! The check fails with the list of public functions no bench names, like
//! `command-inventory` in tauri-plugin-serialplugin.
//!
//! The source scan is a stopgap: once the API snapshot `api/<crate>.txt` (cargo
//! public-api, 8b.9) holds methods too, the list of functions is taken from it.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(fs::read_to_string(&path).unwrap());
        }
    }
}

/// Identifier at the start of `s`.
fn ident(s: &str) -> &str {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    &s[..end]
}

fn public_fns(sources: &[String]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for src in sources {
        for line in src.lines() {
            let line = line.trim_start();
            let rest = ["pub fn ", "pub const fn ", "pub async fn "]
                .iter()
                .find_map(|p| line.strip_prefix(p));
            if let Some(rest) = rest {
                names.insert(ident(rest).to_owned());
            }
        }
    }
    names
}

/// The function segment of every quoted bench id / group id.
fn benched_fns(sources: &[String]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for src in sources {
        // (call, number of `/` segments its id must have)
        for (call, segments) in [("bench_function(\"", 3), ("benchmark_group(\"", 2)] {
            for (i, _) in src.match_indices(call) {
                let rest = &src[i + call.len()..];
                let id = &rest[..rest.find('"').unwrap_or(0)];
                let parts: Vec<&str> = id.split('/').collect();
                assert!(
                    parts.len() == segments && parts.iter().all(|p| !p.is_empty()),
                    "bench id {id:?} in {call}..) must have {segments} non-empty `/` segments \
                     (group/function/case, DESIGN 7.7)"
                );
                names.insert(parts[1].to_owned());
            }
        }
    }
    names
}

/// Panics with the public functions of the crate at `root` that no bench names.
pub fn check(root: &Path) {
    let (mut src, mut benches) = (Vec::new(), Vec::new());
    rust_files(&root.join("src"), &mut src);
    rust_files(&root.join("benches"), &mut benches);

    let fns = public_fns(&src);
    assert!(
        !fns.is_empty(),
        "scan found no public fns in {}",
        root.display()
    );
    let benched = benched_fns(&benches);
    let missing: Vec<_> = fns.difference(&benched).collect();
    assert!(
        missing.is_empty(),
        "public functions without a bench in benches/: {missing:?}"
    );
}
