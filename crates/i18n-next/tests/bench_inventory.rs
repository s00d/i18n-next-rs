//! Rule (DESIGN 7.7, PLAN): every public function of the crate has a criterion bench.
//!
//! The test collects `pub fn NAME` (not `pub(crate)` / `pub(super)`) from `src/**/*.rs`
//! and the bench ids from `benches/*.rs`: `bench_function("group/NAME/case", ..)` or a
//! group `benchmark_group("NAME")`. It fails with the list of public functions that no
//! bench names, like `command-inventory` in tauri-plugin-serialplugin.
//!
//! The source scan is a stopgap: once the API snapshot `api/i18n-next.txt` (cargo
//! public-api, 8b.9) holds methods too, the list of functions is taken from it.
#![cfg(not(target_arch = "wasm32"))]

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

/// Every `/`-separated segment of every quoted bench / group id.
fn bench_names(sources: &[String]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for src in sources {
        for call in ["bench_function(\"", "benchmark_group(\""] {
            for (i, _) in src.match_indices(call) {
                let rest = &src[i + call.len()..];
                let id = &rest[..rest.find('"').unwrap_or(0)];
                names.extend(id.split('/').map(str::to_owned));
            }
        }
    }
    names
}

#[test]
fn every_public_function_has_a_bench() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (mut src, mut benches) = (Vec::new(), Vec::new());
    rust_files(&root.join("src"), &mut src);
    rust_files(&root.join("benches"), &mut benches);

    let fns = public_fns(&src);
    assert!(fns.contains("version"), "scan found no public fns: {fns:?}");
    let benched = bench_names(&benches);
    let missing: Vec<_> = fns.difference(&benched).collect();
    assert!(
        missing.is_empty(),
        "public functions without a bench in benches/: {missing:?}"
    );
}
