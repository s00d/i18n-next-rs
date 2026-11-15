//! Bench inventory (DESIGN 7.7, 8b.9, PLAN): every public function of a crate has a
//! criterion bench. Shared by `tests/bench_inventory.rs` of both crates via `#[path]` (no
//! extra crate).
//!
//! Public functions come from the `cargo public-api -ss` snapshot `api/<crate>.txt`, the
//! single source: CI regenerates it and fails on any diff, so it cannot lag behind `src/`.
//! A `pub fn` line counts when it is a free function (its parent path is a `pub mod`) or a
//! method of an inherent impl or of a trait the crate declares. Methods under an
//! `impl Trait for Type` header (`Clone::clone`, `Display::fmt`, `Serialize::serialize`)
//! implement someone else's API and do not count.
//!
//! Bench ids follow `group/function/case`, in one of two forms:
//! - `c.bench_function("group/NAME/case", ..)`: three segments, the middle one counts,
//!   also when the call sits inside a `benchmark_group` (the group then only groups);
//! - `let mut g = c.benchmark_group("group/NAME"); g.bench_function("case", ..)`: the group
//!   id has two segments and its last one counts; a one-segment `bench_function` id is a
//!   case inside such a group and covers nothing by itself.
//!
//! Only the function segment counts, so a group or case that happens to equal a function
//! name does not cover it. Any other id shape fails the check, so the naming stays uniform.
//! Comments (`//`, `/* */`) are stripped from `benches/` first, so a commented-out bench
//! does not count; string, raw string and char literals are kept as is. Whitespace and
//! newlines between `bench_function(` and the id literal are allowed (rustfmt wraps long
//! calls). The check fails with the list of public functions no bench names, like
//! `command-inventory` in tauri-plugin-serialplugin.

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

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `src` without `//` and `/* */` comments (newlines kept). String literals (with `\`
/// escapes), raw strings (`r".."`, `r#".."#`, `br".."`, no escapes) and char literals are
/// copied as is, so `"http://"` is not a comment.
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
            // raw string: `r` (after an optional `b` / `c` prefix) not inside an identifier,
            // then `#`s and `"`; `r#ident` has no `"` and stays plain text
            'r' if {
                let before = out.strip_suffix(['b', 'c']).unwrap_or(&out);
                let mut ahead = chars.clone();
                while ahead.next_if_eq(&'#').is_some() {}
                !before.ends_with(is_ident) && ahead.next() == Some('"')
            } =>
            {
                out.push(c);
                let mut hashes = 0;
                while let Some(h) = chars.next_if_eq(&'#') {
                    out.push(h);
                    hashes += 1;
                }
                out.extend(chars.next()); // opening `"`
                while let Some(c) = chars.next() {
                    out.push(c);
                    if c == '"' {
                        let mut closing = 0;
                        while closing < hashes && chars.next_if_eq(&'#').is_some() {
                            out.push('#');
                            closing += 1;
                        }
                        if closing == hashes {
                            break;
                        }
                    }
                }
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

/// Path segments of `a::B<X, Y>::f<T: Fn(u8) -> u8>(..)` up to the argument list, with
/// generic arguments dropped and `r#` removed: `["a", "B", "f"]`.
fn fn_path(item: &str) -> Vec<String> {
    let mut segments = vec![String::new()];
    let mut depth = 0usize;
    let mut chars = item.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '-' if chars.peek() == Some(&'>') => {
                chars.next();
            }
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '(' if depth == 0 => break,
            ':' if depth == 0 && chars.peek() == Some(&':') => {
                chars.next();
                segments.push(String::new());
            }
            c if depth == 0 && !c.is_whitespace() => segments.last_mut().unwrap().push(c),
            _ => {}
        }
    }
    segments
        .into_iter()
        .map(|s| s.strip_prefix("r#").map(str::to_owned).unwrap_or(s))
        .collect()
}

/// Names of the public functions and methods listed in a `cargo public-api -ss` snapshot.
fn public_fns(api: &str) -> BTreeSet<String> {
    let mut modules = BTreeSet::new();
    let mut trait_impl = false;
    let mut fns = Vec::new();
    for line in api.lines().map(str::trim) {
        if let Some(m) = line.strip_prefix("pub mod ") {
            modules.insert(fn_path(m));
        }
        // the header a following `pub fn Type::method` belongs to
        if line.starts_with("impl") || line.starts_with("unsafe impl") {
            trait_impl = line.contains(" for ");
        } else if ["struct", "enum", "union", "trait"]
            .iter()
            .any(|k| line.starts_with(&format!("pub {k} ")))
        {
            trait_impl = false;
        }
        // `pub [const] [async] [unsafe] [extern "abi"] fn path(..)`
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        let mut words = rest.split(' ');
        let qualifiers = words.by_ref().take_while(|w| *w != "fn").all(|w| {
            ["const", "async", "unsafe", "extern", "safe"].contains(&w) || w.starts_with('"')
        });
        let item = words.collect::<Vec<_>>().join(" ");
        if qualifiers && !item.is_empty() {
            fns.push((fn_path(&item), trait_impl));
        }
    }
    fns.into_iter()
        .filter(|(path, trait_impl)| {
            let free = modules.contains(&path[..path.len() - 1]);
            free || !trait_impl
        })
        .filter_map(|(mut path, _)| path.pop())
        .collect()
}

/// The function segment of every quoted bench id / group id.
fn benched_fns(sources: &[String]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for src in sources {
        for call in ["bench_function(", "benchmark_group("] {
            for (i, _) in src.match_indices(call) {
                // a non-literal id (a variable, `BenchmarkId`) is not checked
                let Some(rest) = src[i + call.len()..].trim_start().strip_prefix('"') else {
                    continue;
                };
                let id = &rest[..rest.find('"').unwrap_or(0)];
                let parts: Vec<&str> = id.split('/').collect();
                let well_formed = parts.iter().all(|p| !p.is_empty());
                let function = match (call, parts.len()) {
                    ("bench_function(", 3) => Some(Some(parts[1])),
                    // a case inside a `benchmark_group("group/NAME")`
                    ("bench_function(", 1) => Some(None),
                    ("benchmark_group(", 2) => Some(Some(parts[1])),
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

/// Panics with the public functions of crate `name` at `root` that no bench names. The
/// list comes from `api/<name>.txt` at the workspace root (`root/../..`).
pub fn check(root: &Path, name: &str) {
    let api = root.join("../../api").join(format!("{name}.txt"));
    let fns = public_fns(&fs::read_to_string(&api).unwrap());
    assert!(!fns.is_empty(), "no public fns in {}", api.display());
    let mut benches = Vec::new();
    rust_files(&root.join("benches"), &mut benches);
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

    fn covered(api: &str, bench: &str) -> Result<Vec<String>, String> {
        let fns = public_fns(api);
        let bench = strip_comments(bench);
        std::panic::catch_unwind(|| benched_fns(&[bench]))
            .map(|b| fns.difference(&b).cloned().collect())
            .map_err(|e| e.downcast_ref::<String>().cloned().unwrap_or_default())
    }

    const API: &str = "pub mod c\npub fn c::version() -> &'static str\npub fn c::other()\n";

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn group_form_and_flat_form_cover() {
        let bench = r#"
            let mut g = c.benchmark_group("lookup/version"); g.bench_function("short", |b| ());
            c.bench_function("misc/other/a", |b| ());
        "#;
        assert_eq!(covered(API, bench), Ok(vec![]));
    }

    #[test]
    fn three_segment_id_inside_a_group_counts_its_middle_segment() {
        let bench = r#"
            let mut g = c.benchmark_group("lookup/version");
            g.bench_function("misc/other/a", |b| ());
        "#;
        assert_eq!(covered(API, bench), Ok(vec![]));
    }

    #[test]
    fn uncovered_fn_is_reported() {
        let bench = r#"c.bench_function("lookup/version/a", |b| ());"#;
        assert_eq!(covered(API, bench), Ok(vec!["other".to_owned()]));
    }

    #[test]
    fn malformed_ids_fail() {
        for bench in [
            r#"c.bench_function("lookup/version", |b| ());"#,
            r#"c.benchmark_group("lookup/version/x");"#,
            r#"c.benchmark_group("version");"#,
            r#"c.bench_function("lookup//a", |b| ());"#,
            "c.bench_function(\n    \"lookup/version\",\n    |b| (),\n);",
        ] {
            let err = covered(API, bench).unwrap_err();
            assert!(err.contains("malformed"), "{bench}: {err}");
        }
    }

    #[test]
    fn wrapped_bench_function_call_counts() {
        let bench = "c.bench_function(\n        \"lookup/version/a\",\n        |b| (),\n    );\n\
                     c.benchmark_group(\n\t\"misc/other\"\n);";
        assert_eq!(covered(API, bench), Ok(vec![]));
    }

    #[test]
    fn comments_do_not_count() {
        let bench = r#"
            c.bench_function("lookup/version/a", |b| ()); // "http://x" stays a string
            // c.bench_function("misc/other/a", |b| ());
            /* c.bench_function("misc/other/b", |b| ()); /* nested */ */
            let url = "http://example.com"; let q = '"';
        "#;
        assert_eq!(covered(API, bench), Ok(vec!["other".to_owned()]));
        assert_eq!(
            strip_comments(r#"let s = "a//b"; // c"#),
            r#"let s = "a//b"; "#
        );
    }

    #[test]
    fn raw_strings_are_kept_and_end_where_they_end() {
        // `r"C:\"` ends at its second `"` (no escapes in raw strings)
        let bench = "let p = r\"C:\\\"; // c.bench_function(\"misc/other/a\", |b| ());\n\
                     c.bench_function(\"lookup/version/a\", |b| ());";
        assert_eq!(covered(API, bench), Ok(vec!["other".to_owned()]));
        // `r#".."#` holding `\"` and `//`, then a real comment
        let src = r###"let j = r#"{"k":"a\"b", "u": "//x"}"#; // gone"###;
        assert_eq!(
            strip_comments(src),
            r###"let j = r#"{"k":"a\"b", "u": "//x"}"#; "###
        );
        // `br".."`, a `"#` inside `r##".."##`, and a raw identifier
        let src = r####"let b = br"a\"; let h = r##"x"#y"##; let r#type = 1; // gone"####;
        assert_eq!(
            strip_comments(src),
            r####"let b = br"a\"; let h = r##"x"#y"##; let r#type = 1; "####
        );
        // an identifier ending in `r` before a string is not a raw string prefix
        assert_eq!(strip_comments(r#"bar("a//b") // c"#), r#"bar("a//b") "#);
    }

    #[test]
    fn public_api_lines() {
        let api = r#"
pub mod k
pub mod k::sub
pub fn k::version() -> &'static str
pub async fn k::sub::load(&str) -> k::Result<()>
pub const fn k::sub::r#type() -> u8
pub unsafe extern "C" fn k::sub::raw()
pub fn k::init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R>
pub fn k::apply<F: core::ops::Fn(u8) -> u8>(F)
pub struct k::Catalog<T>
impl<T> k::Catalog<T>
pub fn k::Catalog<T>::get(&self, &str) -> core::option::Option<&T>
pub async fn k::Catalog<T>::ensure_async(&self) -> k::Result<()>
pub const k::Catalog<T>::MAX: usize
impl<T: core::clone::Clone> core::clone::Clone for k::Catalog<T>
pub fn k::Catalog<T>::clone(&self) -> k::Catalog<T>
impl core::fmt::Display for k::Catalog<alloc::string::String>
pub type k::Catalog<alloc::string::String>::Output = ()
pub fn k::Catalog<alloc::string::String>::fmt(&self, &mut core::fmt::Formatter<'_>) -> core::fmt::Result
pub trait k::Source
pub fn k::Source::read(&self) -> alloc::vec::Vec<u8>
pub fn k::after_impls()
pub enum k::Error
pub k::Error::Unknown(alloc::string::String)
pub type k::Result<T> = core::result::Result<T, k::Error>
"#;
        assert_eq!(
            public_fns(api),
            set(&[
                "after_impls",
                "apply",
                "ensure_async",
                "get",
                "init",
                "load",
                "raw",
                "read",
                "type",
                "version"
            ])
        );
    }

    #[test]
    fn committed_snapshots_parse() {
        let api = |c: &str| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../api/{c}.txt"));
            public_fns(&fs::read_to_string(path).unwrap())
        };
        assert_eq!(api("i18n-next"), set(&["version"]));
        assert_eq!(api("tauri-plugin-i18n-next"), set(&["init", "ping"]));
    }
}
