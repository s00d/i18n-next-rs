//! Bench inventory (DESIGN 7.7, PLAN): every public function of a crate has a criterion
//! bench. Shared by `tests/bench_inventory.rs` of both crates via `#[path]` (no extra crate).
//!
//! Public functions are `pub fn NAME` (not `pub(crate)` / `pub(super)`) in `src/**/*.rs`.
//! Bench ids follow `group/function/case`, in one of two forms:
//! - `c.bench_function("group/NAME/case", ..)`: three segments, the middle one counts;
//! - `let mut g = c.benchmark_group("group/NAME"); g.bench_function("case", ..)`: the group
//!   id has two segments and its last one counts; a one-segment `bench_function` id is a
//!   case inside such a group and covers nothing by itself.
//!
//! Only the function segment counts, so a group or case that happens to equal a function
//! name does not cover it. Any other id shape fails the check, so the naming stays uniform.
//! Comments (`//`, `/* */`) are stripped before both scans: a commented-out bench or
//! `pub fn` does not count. The check fails with the list of public functions no bench
//! names, like `command-inventory` in tauri-plugin-serialplugin.
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
            out.push(strip_comments(&fs::read_to_string(&path).unwrap()));
        }
    }
}

/// `src` without `//` and `/* */` comments (newlines kept). String literals (with `\`
/// escapes) and char literals are copied as is, so `"http://"` is not a comment.
/// ponytail: raw strings `r#".."#` holding `"` are not parsed; bench and fn scans never
/// need them, a real lexer (syn) if they ever do.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut depth = 1;
                let mut prev = ' ';
                while depth > 0 {
                    let Some(c) = chars.next() else { break };
                    match (prev, c) {
                        ('/', '*') => (depth, prev) = (depth + 1, ' '),
                        ('*', '/') => (depth, prev) = (depth - 1, ' '),
                        _ => prev = c,
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                }
                out.push(' ');
            }
            '"' => {
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            // char literal ('"', '\'', '\\'); a lifetime `'a` is copied as plain text
            '\'' => {
                out.push(c);
                let mut ahead = chars.clone();
                let lit = match ahead.next() {
                    Some('\\') => true,
                    Some(_) => ahead.next() == Some('\''),
                    None => false,
                };
                if lit {
                    while let Some(c) = chars.next() {
                        out.push(c);
                        match c {
                            '\\' => out.extend(chars.next()),
                            '\'' => break,
                            _ => {}
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
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
        for call in ["bench_function(\"", "benchmark_group(\""] {
            for (i, _) in src.match_indices(call) {
                let rest = &src[i + call.len()..];
                let id = &rest[..rest.find('"').unwrap_or(0)];
                let parts: Vec<&str> = id.split('/').collect();
                let well_formed = parts.iter().all(|p| !p.is_empty());
                let function = match (call, parts.len()) {
                    ("bench_function(\"", 3) => Some(Some(parts[1])),
                    // a case inside a `benchmark_group("group/NAME")`
                    ("bench_function(\"", 1) => Some(None),
                    ("benchmark_group(\"", 2) => Some(Some(parts[1])),
                    _ => None,
                };
                let Some(function) = function.filter(|_| well_formed) else {
                    panic!(
                        "bench id {id:?} in {call}..) is malformed: use \
                         bench_function(\"group/function/case\") or \
                         benchmark_group(\"group/function\") + bench_function(\"case\") \
                         (DESIGN 7.7)"
                    );
                };
                names.extend(function.map(str::to_owned));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(src: &str, bench: &str) -> Result<Vec<String>, String> {
        let fns = public_fns(&[strip_comments(src)]);
        let bench = strip_comments(bench);
        std::panic::catch_unwind(|| benched_fns(&[bench]))
            .map(|b| fns.difference(&b).cloned().collect())
            .map_err(|e| e.downcast_ref::<String>().cloned().unwrap_or_default())
    }

    const SRC: &str = "pub fn version() {}\npub fn other() {}\n";

    #[test]
    fn group_form_and_flat_form_cover() {
        let bench = r#"
            let mut g = c.benchmark_group("lookup/version"); g.bench_function("short", |b| ());
            c.bench_function("misc/other/a", |b| ());
        "#;
        assert_eq!(covered(SRC, bench), Ok(vec![]));
    }

    #[test]
    fn uncovered_fn_is_reported() {
        let bench = r#"c.bench_function("lookup/version/a", |b| ());"#;
        assert_eq!(covered(SRC, bench), Ok(vec!["other".to_owned()]));
    }

    #[test]
    fn malformed_ids_fail() {
        for bench in [
            r#"c.bench_function("lookup/version", |b| ());"#,
            r#"c.benchmark_group("lookup/version/x");"#,
            r#"c.benchmark_group("version");"#,
            r#"c.bench_function("lookup//a", |b| ());"#,
        ] {
            let err = covered(SRC, bench).unwrap_err();
            assert!(err.contains("malformed"), "{bench}: {err}");
        }
    }

    #[test]
    fn comments_do_not_count() {
        let bench = r#"
            c.bench_function("lookup/version/a", |b| ()); // "http://x" stays a string
            // c.bench_function("misc/other/a", |b| ());
            /* c.bench_function("misc/other/b", |b| ()); /* nested */ */
            let url = "http://example.com"; let q = '"';
        "#;
        assert_eq!(covered(SRC, bench), Ok(vec!["other".to_owned()]));
        let src = "pub fn version() {}\n// pub fn gone() {}\n/*\npub fn old() {}\n*/\n";
        assert_eq!(
            public_fns(&[strip_comments(src)]),
            BTreeSet::from(["version".to_owned()])
        );
        assert_eq!(
            strip_comments(r#"let s = "a//b"; // c"#),
            r#"let s = "a//b"; "#
        );
    }
}
