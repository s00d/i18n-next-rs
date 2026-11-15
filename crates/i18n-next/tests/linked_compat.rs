//! Linked-message compat goldens (DESIGN 7.2, M3): every call line of
//! `fixtures/compat/__golden__/linked.md`, written by `scripts/compat.mjs` through the Nuxt
//! module's own merge (`mergeSourceTranslations` with `linkedMessages`), replayed through
//! the public API on the same files (`fixtures/compat/linked`):
//!
//! - `linked_t(locale, page, key)`: every leaf of every premerged `(locale, page)` view, read
//!   with `with_locale(locale).page(page).try_t(key)` (`<node>`: a subtree, `t` returns the key
//!   without `missing`; `<missing>`: nothing);
//! - `plain_t`: the same views without `linked_messages(true)` (off by default);
//! - `linked_tp`: `t_with` on a resolved message, its placeholders filled at read;
//! - `case(tag, modifier, text)`: `@.<modifier>:` on `text` in a locale whose `iso` is `tag`
//!   (`""`: no iso), as `toLocaleUpperCase` / `toLocaleLowerCase` run in V8.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{Args, I18n};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `LINKED_LOCALES` of `scripts/compat.mjs`, global fallback `en`.
const CONFIG: &str = r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
  {"code": "en", "iso": "en-US"}, {"code": "de", "iso": "de-DE"},
  {"code": "de-at", "iso": "de-AT", "fallbackLocale": "de"}, {"code": "de-ch", "fallbackLocale": "de"},
  {"code": "tr", "iso": "tr-TR"},
  {"code": "el", "iso": "el-GR"}, {"code": "lt"}, {"code": "xx", "iso": "en_US"}]}"#;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compat")
}

fn files(dir: &Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_str().unwrap();
        let r = if rel.is_empty() {
            name.to_owned()
        } else {
            format!("{rel}/{name}")
        };
        if path.is_dir() {
            files(&path, &r, out);
        } else {
            out.push((r, std::fs::read(&path).unwrap()));
        }
    }
}

struct Catalog {
    i18n: I18n,
    misses: Arc<AtomicUsize>,
}

impl Catalog {
    fn new(linked: bool) -> Catalog {
        let mut list = Vec::new();
        files(&fixtures().join("linked"), "", &mut list);
        let misses = Arc::new(AtomicUsize::new(0));
        let m = misses.clone();
        let i18n = I18n::builder()
            .config_json(CONFIG)
            .files(list)
            .linked_messages(linked)
            .missing(move |_, _, _| {
                m.fetch_add(1, Ordering::Relaxed);
            })
            .build()
            .unwrap();
        // what `build` reported is not part of the lookups below
        misses.store(0, Ordering::Relaxed);
        Catalog { i18n, misses }
    }

    /// The golden spelling of a lookup: the text, `<node>` or `<missing>`.
    fn read(&self, locale: &str, page: &str, key: &str) -> String {
        let view = self.i18n.with_locale(locale).page(page);
        if let Some(text) = view.try_t(key) {
            return text;
        }
        let before = self.misses.load(Ordering::Relaxed);
        let _ = view.t(key);
        let missed = key.is_empty() || self.misses.load(Ordering::Relaxed) > before;
        if missed { "\u{0}missing" } else { "\u{0}node" }.to_owned()
    }
}

/// `"…"` decoded, or a marker as `\0missing` / `\0node`.
fn expected(result: &str) -> String {
    match result {
        "<missing>" => "\u{0}missing".to_owned(),
        "<node>" => "\u{0}node".to_owned(),
        r => serde_json::from_str::<String>(r).unwrap_or_else(|e| panic!("{r}: {e}")),
    }
}

fn str_arg(v: &Value) -> &str {
    v.as_str().unwrap()
}

/// The messages of one `case` catalog: `t<i>` and `<modifier><i>` per text of `tag`.
fn case_catalog(tag: &str, texts: &[&str]) -> BTreeMap<String, String> {
    let mut tree = serde_json::Map::new();
    for (i, text) in texts.iter().enumerate() {
        tree.insert(format!("t{i}"), Value::from(*text));
        for m in ["upper", "lower", "capitalize"] {
            tree.insert(format!("{m}{i}"), Value::from(format!("@.{m}:t{i}")));
        }
    }
    let mut b = I18n::builder().locale("p").linked_messages(true).messages(
        "p",
        None,
        &Value::Object(tree).to_string(),
    );
    if !tag.is_empty() {
        b = b.iso("p", tag);
    }
    let i18n = b.build().unwrap();
    let mut out = BTreeMap::new();
    for (i, text) in texts.iter().enumerate() {
        for m in ["upper", "lower", "capitalize"] {
            out.insert(format!("{m}\u{0}{text}"), i18n.t(&format!("{m}{i}")));
        }
    }
    out
}

/// Given the module's premerged views and V8's case mapping, when Rust resolves the same
/// files at `build`, then every line matches.
#[test]
fn linked_goldens_match_the_nuxt_module() {
    let golden = std::fs::read_to_string(fixtures().join("__golden__/linked.md")).unwrap();
    let lines: Vec<(&str, Vec<Value>, &str)> = golden
        .lines()
        .filter(|l| {
            !l.is_empty() && !l.starts_with('#') && !l.starts_with('>') && !l.starts_with("```")
        })
        .map(|l| {
            let (call, result) = l.split_once(" → ").unwrap();
            let (name, args) = call.split_once('(').unwrap();
            let args = args.strip_suffix(')').unwrap();
            let args: Vec<Value> = serde_json::from_str(&format!("[{args}]")).unwrap();
            (name, args, result)
        })
        .collect();

    let linked = Catalog::new(true);
    let plain = Catalog::new(false);
    let mut cases: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, args, _) in &lines {
        if *name == "case" {
            let list = cases.entry(str_arg(&args[0])).or_default();
            let text = str_arg(&args[2]);
            if !list.contains(&text) {
                list.push(text);
            }
        }
    }
    let cased: BTreeMap<&str, BTreeMap<String, String>> = cases
        .iter()
        .map(|(tag, texts)| (*tag, case_catalog(tag, texts)))
        .collect();

    let mut diffs = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (name, args, result) in &lines {
        *counts.entry(name).or_default() += 1;
        let got = match *name {
            "linked_t" | "plain_t" => {
                let cat = if *name == "linked_t" { &linked } else { &plain };
                cat.read(str_arg(&args[0]), str_arg(&args[1]), str_arg(&args[2]))
            }
            "linked_tp" => {
                let p = args[3].as_object().unwrap();
                let mut a = Args::new();
                for (k, v) in p {
                    a = match v {
                        Value::String(s) => a.str(k, s),
                        v => a.int(k, v.as_i64().unwrap()),
                    };
                }
                linked
                    .i18n
                    .with_locale(str_arg(&args[0]))
                    .page(str_arg(&args[1]))
                    .t_with(str_arg(&args[2]), &a)
            }
            "case" => cased[str_arg(&args[0])]
                [&format!("{}\u{0}{}", str_arg(&args[1]), str_arg(&args[2]))]
                .clone(),
            other => panic!("unknown call {other}"),
        };
        if got != expected(result) {
            diffs.push(format!("{name}{args:?}: golden {result}, rust {got:?}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "{} of {} linked golden lines differ:\n{}",
        diffs.len(),
        lines.len(),
        diffs
            .iter()
            .take(80)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        lines.len() > 20_000,
        "only {} lines: {counts:?}",
        lines.len()
    );
    for call in ["linked_t", "plain_t", "linked_tp", "case"] {
        assert!(
            counts.get(call).copied().unwrap_or(0) > 0,
            "no {call} lines"
        );
    }
}
