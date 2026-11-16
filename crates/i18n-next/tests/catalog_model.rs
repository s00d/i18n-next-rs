//! Catalog model (DESIGN 7.3): random sessions checked after every step against a small
//! reference: JSON trees per `(locale, page)` in a `BTreeMap`, merged like
//! `deepMergeTranslationsRecursive`, read like `getByPath`, with the layer walk, the
//! chains and `t` / `t_with` (JS `interpolate`) written out.
//!
//! Each case builds a catalog the way an app does: 2–3 locales (the first is the start
//! locale, named by `.locale`, by `defaultLocale` or by nobody → `en`), an `i18n.json` with
//! a random subset of them in random order (some with `fallbackLocale`, `es` disabled),
//! `fallback_for` links (cycles allowed), a global fallback, `iso`, `pages(true|false)`,
//! one or two page aliases, embedded `files()` (root, page, nested page folder, two folders
//! of one page) and `messages`. Files are merged in JS path order with the
//! `storeLoadedTranslationFile` rule of `disablePageLocales` (a folded page file wins over
//! the root file in either order).
//!
//! Actions: `add_messages` (also through an alias and into a new locale), re-adding a chunk
//! added before, `set_locale`, lookups through every entry point (`t` / `t_with` with
//! named, duplicate, list, `{01}`, `.or` and overflowing parameters / `try_t` / `has` /
//! `get` / `tc` / `tc_with` with integer and `f64` counts, on `page()` and `with_locale()`
//! views) with keys drawn mostly from the keys
//! already written, a lookup of a key written in both the page and the root of one
//! locale, and bad names. After every step: the answer, every `missing` call, the
//! version, the active locale, `pages()` and `locales()`.
//!
//! Two counters prove the session space can see what it must: lookups whose answer
//! changes if the root layer were read before the page layer, and lookups whose answer
//! changes if merging an object into an object replaced it. The run fails if either stays
//! 0. A third counts `tc` answers where the category rule (unmarked or `=0 `) picked another
//! form than the index rule would. The model's `pluralForm` is transcribed from the JS
//! (`createLocalePlural`, `unmarkedIndex`, `pluralIndex`, `markedIndex`), not from the
//! crate: only the category of a count comes from the generated rules (checked against
//! `Intl` by `plural_props`).
//! At the end of a session: a value and a view handed out before it are unchanged /
//! live, versions only grew, and a fresh catalog built from the model's chunks through
//! `messages` answers every locale × page × key like the session's catalog.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/plural/mod.rs"]
#[allow(dead_code)]
mod plural;

use i18n_next::{Args, Count, Error, I18n};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Locale codes of a case. `es` is disabled in `i18n.json` whenever it is drawn; `ru` and
/// `zz` sort after `pages/`, `de` / `en` / `es` before (the folding order differs).
const POOL: &[&str] = &["en", "de", "ru", "es", "zz"];
/// A locale no case knows (until `add_messages` writes to it).
const UNKNOWN: &str = "xx";
/// Page names a lookup uses: real pages, the two aliases, the root spellings.
const PAGES: &[Option<&str>] = &[
    None,
    Some("p"),
    Some("q"),
    Some("n-m"),
    Some("al1"),
    Some("al2"),
    Some("index"),
    Some(""),
];
/// Pages `add_messages` writes to (an alias writes its target).
const ADD_PAGES: &[Option<&str>] = &[
    None,
    Some("p"),
    Some("q"),
    Some("n-m"),
    Some("al1"),
    Some("index"),
    Some(""),
];
/// Keys that are always worth asking, written or not: arrays, flat vs walked, `__proto__`,
/// the empty key and its dotted relatives.
const FIXED_KEYS: &[&str] = &[
    "a",
    "n.a",
    "x.y",
    "x.y.a",
    "l.0",
    "l.01",
    "l.length",
    "",
    ".a",
    "n.",
    "__proto__",
    "n.__proto__",
    "01",
];
const BAD: &[&str] = &[
    "../x",
    "CON",
    "",
    "ünï",
    "__proto__",
    "../../etc/passwd",
    "a\0b",
];
/// Leaf texts: placeholders with JS whitespace (U+00A0 is `\s`, U+200B is not), `{01}`,
/// list items and `{length}`.
const TEXTS: &[&str] = &[
    "v1",
    "{x}!",
    "",
    "w {y}",
    "{x}{x}",
    "{01} {1}",
    "{0}/{length}",
    "{x\u{a0}}",
    "{ x}",
    "{x\u{200b}}",
    "{a}{b}{c}{d}{x}",
    // plural templates: index, unmarked by category (ru, uk), `=0 `, trimming, empty forms
    "car | cars",
    "z | {count} o | {count} f | {count} m",
    "z | o | {count} f | {count} m",
    "=0 none | one {count} | other {count}",
    "=0 n | a | b | c | d | e",
    " p {x} |{count}| q ",
    "|",
    "=0 lone {count}",
    "{x} | {count} {x} | {count}",
    // the category shape but for a `{` in the zero form: the index rule
    "z {x} | {count} o | {count} f | {count} m",
];

/// `tc` counts: the `Count`, its JS spelling, and the whole count `pluralForm` reads.
fn counts() -> [(Count, &'static str, Option<u64>); 16] {
    [
        (Count::from(0), "0", Some(0)),
        (Count::from(1), "1", Some(1)),
        (Count::from(2u64), "2", Some(2)),
        (Count::from(3usize), "3", Some(3)),
        (Count::from(5i32), "5", Some(5)),
        (Count::from(11u32), "11", Some(11)),
        (Count::from(21), "21", Some(21)),
        (Count::from(22), "22", Some(22)),
        (Count::from(101), "101", Some(101)),
        (Count::from(250u64), "250", Some(250)),
        (Count::from(1000), "1000", Some(1000)),
        (Count::from(-1), "-1", None),
        (Count::from(1.5), "1.5", None),
        (Count::from(2.0), "2", Some(2)),
        (Count::from(-0.0), "0", Some(0)),
        (Count::from(f64::NAN), "NaN", None),
    ]
}

/// Lookups whose answer depends on the page layer being read before the root layer.
static LAYER_ORDER_SEEN: AtomicUsize = AtomicUsize::new(0);
/// Lookups whose answer depends on an object merging into an object (not replacing it).
static DEEP_MERGE_SEEN: AtomicUsize = AtomicUsize::new(0);
/// `tc` answers where a category rule picked another form than the index rule.
static CATEGORY_SEEN: AtomicUsize = AtomicUsize::new(0);

/// `t_with` parameters, each with its JS meaning in [`ArgsKind::param`].
#[derive(Debug, Clone, Copy)]
enum ArgsKind {
    /// `x` twice (the last wins), `y`, a named `01`.
    Named,
    /// `["L0", "L1"]`: `{0}`, `{1}`, `{length}`; `{01}` is no index.
    List,
    /// [`ArgsKind::Named`] with a default.
    NamedOr,
    /// An empty default counts as none: the key comes back.
    EmptyOr,
    /// Six parameters: two in the overflow, `x` twice across the boundary.
    Overflow,
}

impl ArgsKind {
    fn args(self) -> Args<'static> {
        let named = || {
            Args::new()
                .str("x", "old")
                .str("x", "X")
                .str("y", "Y")
                .str("01", "Z")
        };
        match self {
            ArgsKind::Named => named(),
            ArgsKind::List => Args::list(["L0", "L1"]),
            ArgsKind::NamedOr => named().or("D {x} {01}"),
            ArgsKind::EmptyOr => Args::new().str("x", "X").or(""),
            ArgsKind::Overflow => Args::new()
                .str("a", "1")
                .str("b", "2")
                .str("c", "3")
                .str("d", "4")
                .str("x", "five")
                .str("x", "X6"),
        }
    }

    /// What `params[name]` is in JS for these parameters (own properties only).
    fn param(self, name: &str) -> Option<String> {
        let v = match (self, name) {
            (ArgsKind::Named | ArgsKind::NamedOr, "x") => "X",
            (ArgsKind::Named | ArgsKind::NamedOr, "y") => "Y",
            (ArgsKind::Named | ArgsKind::NamedOr, "01") => "Z",
            (ArgsKind::List, "0") => "L0",
            (ArgsKind::List, "1") => "L1",
            (ArgsKind::List, "length") => "2",
            (ArgsKind::EmptyOr, "x") => "X",
            (ArgsKind::Overflow, "a") => "1",
            (ArgsKind::Overflow, "b") => "2",
            (ArgsKind::Overflow, "c") => "3",
            (ArgsKind::Overflow, "d") => "4",
            (ArgsKind::Overflow, "x") => "X6",
            _ => return None,
        };
        Some(v.to_owned())
    }

    fn default(self) -> Option<&'static str> {
        match self {
            ArgsKind::NamedOr => Some("D {x} {01}"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Via {
    T,
    TWith(ArgsKind),
    TryT,
    Has,
    Get,
    /// `tc` (no args) / `tc_with`, with the count at this index of [`counts`].
    Tc(u8, Option<ArgsKind>),
}

/// A locale of the case by index: `codes[i % (n + 1)]`, the last slot is [`UNKNOWN`].
type Loc = u8;

#[derive(Debug, Clone)]
enum Action {
    Add {
        locale: Loc,
        page: Option<&'static str>,
        tree: Value,
    },
    /// The same `add_messages` as an earlier one (index into the adds so far).
    ReAdd(usize),
    SetLocale(Loc),
    Lookup {
        via: Via,
        /// Mostly an index into the keys written so far, sometimes a fixed key.
        key: usize,
        page: Option<&'static str>,
        /// `with_locale` view; `None` reads the active locale.
        locale: Option<Loc>,
    },
    /// A key written in both a page chunk and the root chunk of one locale, read through
    /// that page.
    Both {
        via: Via,
        pick: usize,
    },
    /// A name from outside: 0 `set_locale`, 1 `page().t`, 2 `with_locale().t`, 3 `add_messages`.
    BadName {
        name: &'static str,
        kind: u8,
    },
}

#[derive(Debug, Clone, Copy)]
enum Start {
    /// `.locale(code)`
    Builder,
    /// `"defaultLocale": code` in `i18n.json`
    Config,
    /// Nobody names it: `en`.
    Default,
}

#[derive(Debug, Clone)]
struct Case {
    codes: Vec<&'static str>,
    start: Start,
    /// `i18n.json` `locales[]` in file order: code, `fallbackLocale`, `iso`.
    configured: Vec<(&'static str, Option<&'static str>, Option<&'static str>)>,
    links: Vec<(&'static str, &'static str)>,
    fallback: Option<&'static str>,
    iso: Option<(&'static str, &'static str)>,
    pages: bool,
    aliases: Vec<(&'static str, &'static str)>,
    files: Vec<(String, Value)>,
    messages: Vec<(&'static str, Option<&'static str>, Value)>,
    actions: Vec<Action>,
}

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => prop::sample::select(TEXTS).prop_map(Value::from),
        1 => (0i64..3).prop_map(Value::from),
        1 => any::<bool>().prop_map(Value::from),
        1 => Just(Value::Null),
    ]
}

fn key() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["a", "n", "x.y", "l", "__proto__", "", "0", "01", "x"])
        .prop_map(str::to_owned)
}

fn tree() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(3, 16, 3, |inner| {
        prop_oneof![
            1 => prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            3 => prop::collection::btree_map(key(), inner, 0..3)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn object() -> impl Strategy<Value = Value> {
    prop::collection::btree_map(key(), tree(), 0..4)
        .prop_map(|m| Value::Object(m.into_iter().collect()))
}

fn kind2() -> impl Strategy<Value = ArgsKind> {
    prop::sample::select(vec![
        ArgsKind::Named,
        ArgsKind::List,
        ArgsKind::NamedOr,
        ArgsKind::EmptyOr,
        ArgsKind::Overflow,
    ])
}

fn via() -> impl Strategy<Value = Via> {
    let kind = prop::sample::select(vec![
        ArgsKind::Named,
        ArgsKind::List,
        ArgsKind::NamedOr,
        ArgsKind::EmptyOr,
        ArgsKind::Overflow,
    ]);
    prop_oneof![
        3 => Just(Via::T),
        2 => kind.prop_map(Via::TWith),
        1 => Just(Via::TryT),
        2 => Just(Via::Has),
        1 => Just(Via::Get),
        4 => (0u8..16, prop::option::of(kind2())).prop_map(|(c, k)| Via::Tc(c, k)),
    ]
}

fn action() -> impl Strategy<Value = Action> {
    let view_locale = prop_oneof![3 => Just(None), 2 => any::<Loc>().prop_map(Some)];
    prop_oneof![
        3 => (any::<Loc>(), prop::sample::select(ADD_PAGES), object())
            .prop_map(|(locale, page, tree)| Action::Add { locale, page, tree }),
        1 => any::<usize>().prop_map(Action::ReAdd),
        1 => any::<Loc>().prop_map(Action::SetLocale),
        6 => (via(), any::<usize>(), prop::sample::select(PAGES), view_locale)
            .prop_map(|(via, key, page, locale)| Action::Lookup { via, key, page, locale }),
        3 => (via(), any::<usize>()).prop_map(|(via, pick)| Action::Both { via, pick }),
        1 => (prop::sample::select(BAD), 0u8..4).prop_map(|(name, kind)| Action::BadName { name, kind }),
    ]
}

/// Paths a file of `code` can have: root, two pages, a nested folder and the dashed
/// folder of the same page (`n-m`).
fn file_path(code: &str, kind: usize) -> String {
    match kind {
        0 => format!("{code}.json"),
        1 => format!("pages/p/{code}.json"),
        2 => format!("pages/q/{code}.json"),
        3 => format!("pages/n/m/{code}.json"),
        _ => format!("pages/n-m/{code}.json"),
    }
}

fn case() -> impl Strategy<Value = Case> {
    prop::sample::subsequence(POOL.to_vec(), 2..=3)
        .prop_shuffle()
        .prop_flat_map(|codes| {
            let n = codes.len();
            (
                Just(codes),
                prop::sample::select(vec![Start::Builder, Start::Config, Start::Default]),
                prop::collection::vec((any::<bool>(), prop::option::of(0..n), any::<bool>()), n),
                prop::collection::vec((0..n, 0..n), 0..3),
                prop::option::of(0..=n),
                (any::<bool>(), prop::option::of((0..n, 0..4usize))),
                prop::collection::vec((0..2usize, 0..2usize), 0..3),
                prop::collection::vec((0..n, 0..5usize, object()), 0..7),
                prop::collection::vec(
                    (
                        0..n,
                        prop::sample::select(vec![None, Some("p"), Some("q"), Some("al1")]),
                        object(),
                    ),
                    0..3,
                ),
                prop::collection::vec(action(), 1..40),
            )
        })
        .prop_map(
            |(
                mut codes,
                start,
                conf,
                links,
                fallback,
                (pages, iso),
                aliases,
                files,
                messages,
                actions,
            )| {
                // The start locale is never the disabled one.
                if codes[0] == "es" {
                    codes.rotate_left(1);
                }
                let mut configured = Vec::new();
                for (i, (on, fb, iso)) in conf.iter().enumerate() {
                    let code = codes[i];
                    if *on || code == "es" {
                        configured.push((code, fb.map(|f| codes[f]), iso.then_some("x-ISO")));
                    }
                }
                // `fallbackLocale` in the file names configured codes only: what a locale
                // named nowhere else becomes is the builder's business (`fallback_for`).
                let names: Vec<&str> = configured.iter().map(|c| c.0).collect();
                for c in &mut configured {
                    c.1 = c.1.filter(|f| names.contains(f));
                }
                Case {
                    start,
                    configured,
                    links: links.iter().map(|&(a, b)| (codes[a], codes[b])).collect(),
                    fallback: fallback.map(|f| codes.get(f).copied().unwrap_or("en")),
                    // unsupported tags (index rule) and real ones (`uk-UA` unmarked, `ar` six categories)
                    iso: iso.map(|(i, t)| (codes[i], ["ab", "Ab-CD", "uk-UA", "ar"][t])),
                    pages,
                    aliases: aliases
                        .iter()
                        .map(|&(a, t)| (["al1", "al2"][a], ["p", "q"][t]))
                        .collect(),
                    // One file per path, as in a directory (the last drawn wins).
                    files: files
                        .into_iter()
                        .map(|(c, kind, tree)| (file_path(codes[c], kind), tree))
                        .collect::<BTreeMap<_, _>>()
                        .into_iter()
                        .collect(),
                    messages: messages
                        .into_iter()
                        .map(|(c, page, tree)| (codes[c], page, tree))
                        .collect(),
                    actions,
                    codes,
                }
            },
        )
}

/// `deepMergeTranslationsRecursive` (`deep`), or a merge where an object replaces an
/// object (`!deep`, the shadow model a broken merge would follow).
fn merge(target: &mut Map<String, Value>, source: &Map<String, Value>, deep: bool) {
    for (k, v) in source {
        if k == "__proto__" {
            continue;
        }
        match v {
            Value::Object(src) => {
                let mut dst = match target.remove(k) {
                    Some(Value::Object(d)) if deep => d,
                    _ => Map::new(),
                };
                merge(&mut dst, src, deep);
                target.insert(k.clone(), Value::Object(dst));
            }
            _ => {
                target.insert(k.clone(), v.clone());
            }
        }
    }
}

/// `getByPath`: own flat key first unless null, then the dotted walk; arrays by index.
fn get_by_path<'a>(tree: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if path.is_empty() || path == "__proto__" {
        return None;
    }
    let flat = tree.get(path);
    if flat.is_some_and(|v| !v.is_null()) || !path.contains('.') {
        return flat.filter(|v| !v.is_null());
    }
    let mut node: Option<&Value> = None;
    for (i, part) in path.split('.').enumerate() {
        if part == "__proto__" {
            return None;
        }
        let next = match (i, node) {
            (0, _) => tree.get(part),
            (_, Some(Value::Object(m))) => m.get(part),
            (_, Some(Value::Array(a))) => {
                let canonical = !part.is_empty()
                    && (part == "0" || !part.starts_with('0'))
                    && part.bytes().all(|b| b.is_ascii_digit());
                if canonical {
                    part.parse::<usize>().ok().and_then(|i| a.get(i))
                } else {
                    None
                }
            }
            _ => None,
        };
        node = Some(next?);
    }
    node.filter(|v| !v.is_null())
}

/// Every dotted path a tree defines (objects, arrays, leaves), for drawing lookup keys.
fn paths(v: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    let mut add = |k: String, child: &Value| {
        let p = if prefix.is_empty() {
            k
        } else {
            format!("{prefix}.{k}")
        };
        out.insert(p.clone());
        paths(child, &p, out);
    };
    match v {
        Value::Object(m) => m.iter().for_each(|(k, c)| add(k.clone(), c)),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, c)| add(i.to_string(), c)),
        _ => {}
    }
}

/// What a lookup finds.
#[derive(Debug, PartialEq, Clone)]
enum Hit {
    /// A string: interpolated by `t_with`.
    Msg(String),
    /// A number or boolean as JS prints it: never interpolated.
    Value(String),
    /// A subtree or array.
    Node,
}

impl Hit {
    fn text(&self) -> Option<String> {
        match self {
            Hit::Msg(s) | Hit::Value(s) => Some(s.clone()),
            Hit::Node => None,
        }
    }
}

fn hit(v: &Value) -> Hit {
    match v {
        Value::String(s) => Hit::Msg(s.clone()),
        Value::Number(n) => Hit::Value(n.to_string()),
        Value::Bool(b) => Hit::Value(b.to_string()),
        _ => Hit::Node,
    }
}

/// JS `\s`: ECMAScript WhiteSpace and LineTerminator.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `text.replace(/\{([^{}\s]+)\}/g, (m, name) => params[name] ?? m)`.
fn interpolate(text: &str, kind: ArgsKind) -> String {
    interpolate_by(text, |name| kind.param(name))
}

/// [`interpolate`] with any parameter lookup.
fn interpolate_by(text: &str, param: impl Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            while j < chars.len() && !matches!(chars[j], '{' | '}') && !js_space(chars[j]) {
                j += 1;
            }
            if j < chars.len() && chars[j] == '}' && j > i + 1 {
                let name: String = chars[i + 1..j].iter().collect();
                match param(&name) {
                    Some(v) => out.push_str(&v),
                    None => out.extend(&chars[i..=j]),
                }
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

type Chunks = BTreeMap<(String, String), Map<String, Value>>;

/// The reference.
struct Model {
    chunks: Chunks,
    /// The same writes merged with objects replacing objects.
    shadow: Chunks,
    locale: String,
    version: u64,
    case: Case,
    /// Every key written so far.
    written: BTreeSet<String>,
}

impl Model {
    /// Applies the builder's files and messages, like `build` and the JS loader.
    fn new(case: &Case) -> Model {
        let start = match case.start {
            Start::Default => "en",
            _ => case.codes[0],
        };
        let mut m = Model {
            chunks: BTreeMap::new(),
            shadow: BTreeMap::new(),
            locale: start.to_owned(),
            version: 0,
            case: case.clone(),
            written: BTreeSet::new(),
        };
        // Files in JS path order (all ASCII here: byte order is UTF-16 order).
        let mut files: Vec<&(String, Value)> = case.files.iter().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, tree) in files {
            let (code, page) = classify(path);
            let folded = page.is_some() && !case.pages;
            let page = if case.pages { page } else { None };
            let key = (code.to_owned(), page.unwrap_or("index").to_owned());
            let src = tree.as_object().unwrap();
            for (chunks, deep) in [(&mut m.chunks, true), (&mut m.shadow, false)] {
                match chunks.get_mut(&key) {
                    // `storeLoadedTranslationFile`: the root file read after folded page
                    // files goes under them
                    Some(existing) if page.is_none() && !folded => {
                        let mut under = Map::new();
                        merge(&mut under, src, deep);
                        merge(&mut under, existing, deep);
                        *existing = under;
                    }
                    Some(existing) => merge(existing, src, deep),
                    None => {
                        let mut t = Map::new();
                        merge(&mut t, src, deep);
                        chunks.insert(key.clone(), t);
                    }
                }
            }
            paths(tree, "", &mut m.written);
        }
        for (code, page, tree) in &case.messages {
            m.write(code, *page, tree);
        }
        // With pages, an alias needs its target: the empty page messages of `run` give it.
        if case.pages {
            for (_, to) in &case.aliases {
                m.write(case.codes[0], Some(to), &Value::Object(Map::new()));
            }
        }
        m
    }

    fn code(&self, loc: Loc) -> &'static str {
        let n = self.case.codes.len();
        match loc as usize % (n + 1) {
            i if i == n => UNKNOWN,
            i => self.case.codes[i],
        }
    }

    fn disabled(code: &str) -> bool {
        code == "es"
    }

    fn configured(&self, code: &str) -> bool {
        self.case.configured.iter().any(|c| c.0 == code)
    }

    /// Names the app gave: start (builder or config, or the default `en`), the global
    /// fallback, both ends of every link (`fallback_for` and `i18n.json` `fallbackLocale`),
    /// builder `iso` codes (an `i18n.json` `iso` names a configured code). Each is known
    /// even with no data of its own.
    fn named(&self, code: &str) -> bool {
        let start = match self.case.start {
            Start::Default => "en",
            _ => self.case.codes[0],
        };
        code == start
            || self.case.fallback == Some(code)
            || self.links().iter().any(|(&a, &b)| a == code || b == code)
            || self.case.iso.is_some_and(|(c, _)| c == code)
    }

    fn known(&self, code: &str) -> bool {
        self.configured(code) || self.named(code) || self.chunks.keys().any(|(l, _)| l == code)
    }

    fn enabled(&self, code: &str) -> bool {
        self.known(code) && !Self::disabled(code)
    }

    fn links(&self) -> BTreeMap<&'static str, &'static str> {
        let mut links = BTreeMap::new();
        for (code, fb, _) in &self.case.configured {
            if let Some(fb) = fb {
                links.insert(*code, *fb);
            }
        }
        for &(a, b) in &self.case.links {
            links.insert(a, b);
        }
        links
    }

    /// `code`, its links until a repeat, then the global fallback alone: its own links are
    /// not followed (`buildFallbackLocaleChain`).
    fn chain(&self, code: &str) -> Vec<String> {
        let links = self.links();
        let mut out: Vec<String> = Vec::new();
        let mut c = Some(code);
        while let Some(x) = c {
            if out.iter().any(|o| o == x) {
                break;
            }
            out.push(x.to_owned());
            c = links.get(x).copied();
        }
        if let Some(fb) = self.case.fallback
            && !out.iter().any(|o| o == fb)
        {
            out.push(fb.to_owned());
        }
        out
    }

    fn layers(&self, view: Option<&str>) -> Vec<String> {
        match view {
            None => self.chain(&self.locale),
            // A disabled locale is still configured: it reads its own chain.
            Some(code) if self.known(code) => self.chain(code),
            // `buildFallbackLocaleChain` of an unconfigured code: the global fallback alone.
            Some(_) => self.case.fallback.map(str::to_owned).into_iter().collect(),
        }
    }

    /// The chunk page a name reads: `""` / `index` / none → root, an alias → its target.
    fn chunk_page(&self, page: Option<&str>) -> String {
        match page {
            None | Some("" | "index") => "index".to_owned(),
            Some(p) => {
                let alias = self.case.aliases.iter().rev().find(|(a, _)| *a == p);
                alias.map_or(p, |(_, t)| t).to_owned()
            }
        }
    }

    /// One locale: page layer, then root (or the reverse, for the layer-order counter),
    /// from the real chunks or the shadow ones.
    fn in_locale(
        &self,
        code: &str,
        page: &str,
        key: &str,
        root_first: bool,
        shadow: bool,
    ) -> Option<Hit> {
        let chunks = if shadow { &self.shadow } else { &self.chunks };
        let read = |p: &str| {
            chunks
                .get(&(code.to_owned(), p.to_owned()))
                .and_then(|t| get_by_path(t, key))
                .map(hit)
        };
        match (page, root_first) {
            ("index", _) => read("index"),
            (p, false) => read(p).or_else(|| read("index")),
            (p, true) => read("index").or_else(|| read(p)),
        }
    }

    fn lookup_as(
        &self,
        view: Option<&str>,
        page: Option<&str>,
        key: &str,
        root_first: bool,
        shadow: bool,
    ) -> Option<Hit> {
        let page = self.chunk_page(page);
        self.layers(view)
            .iter()
            .find_map(|code| self.in_locale(code, &page, key, root_first, shadow))
    }

    /// The real answer; counts the lookups that would differ under a broken layer order
    /// or a broken deep merge.
    fn lookup(&self, view: Option<&str>, page: Option<&str>, key: &str) -> Option<Hit> {
        let real = self.lookup_as(view, page, key, false, false);
        if self.lookup_as(view, page, key, true, false) != real {
            LAYER_ORDER_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        if self.lookup_as(view, page, key, false, true) != real {
            DEEP_MERGE_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        real
    }

    fn has(&self, view: Option<&str>, page: Option<&str>, key: &str) -> bool {
        let code = view.unwrap_or(&self.locale);
        self.known(code)
            && self
                .in_locale(code, &self.chunk_page(page), key, false, false)
                .is_some()
    }

    /// `t` (`args: None`) / `t_with`: the text, and the `missing` record it causes.
    fn t(
        &self,
        view: Option<&str>,
        page: Option<&str>,
        key: &str,
        args: Option<ArgsKind>,
    ) -> (String, Option<String>) {
        if key.is_empty() {
            return (String::new(), None);
        }
        match (self.lookup(view, page, key), args) {
            (Some(Hit::Msg(s)), Some(kind)) => (interpolate(&s, kind), None),
            (Some(Hit::Msg(s) | Hit::Value(s)), _) => (s, None),
            (Some(Hit::Node), _) => (key.to_owned(), None),
            (None, _) => {
                let code = view.unwrap_or(&self.locale);
                let p = page.filter(|p| !p.is_empty()).unwrap_or("index");
                let text = match args.and_then(ArgsKind::default) {
                    Some(d) => interpolate(d, args.unwrap()),
                    None => key.to_owned(),
                };
                (text, Some(format!("{code}|{p}|{key}")))
            }
        }
    }

    /// A written key whose text in this view has a `|`.
    fn plural_key(&self, view: Option<&str>, page: Option<&str>, pick: usize) -> Option<String> {
        let keys: Vec<&String> = self
            .written
            .iter()
            .filter(|k| {
                matches!(self.lookup_as(view, page, k, false, false), Some(Hit::Msg(s)) if s.contains('|'))
            })
            .collect();
        (!keys.is_empty()).then(|| keys[pick % keys.len()].clone())
    }

    /// The tag plurals of `code` go by: the builder's `iso`, else `i18n.json`'s, else the code.
    fn tag(&self, code: &str) -> String {
        if let Some((c, tag)) = self.case.iso
            && c == code
        {
            return tag.to_owned();
        }
        let file = self
            .case
            .configured
            .iter()
            .find(|c| c.0 == code)
            .and_then(|c| c.2);
        file.unwrap_or(code).to_owned()
    }

    /// `tc` / `tc_with` (JS `tc(key, { ...params, count }, default)` with `defaultPlural`):
    /// the text, and the `missing` record it causes.
    fn tc(
        &self,
        view: Option<&str>,
        page: Option<&str>,
        key: &str,
        count: (Count, &'static str, Option<u64>),
        args: Option<ArgsKind>,
    ) -> (String, Option<String>) {
        let (_, spelled, whole) = count;
        // `{ ...params, count }`: the count wins, a spread list keeps no `length`
        let param = |name: &str| match name {
            "count" => Some(spelled.to_owned()),
            "length" => None,
            _ => args.and_then(|k| k.param(name)),
        };
        // `defaultValue ? interpolate(defaultValue, values) : defaultValue ?? key`
        let fallback = || match args {
            Some(ArgsKind::NamedOr) => interpolate_by("D {x} {01}", param),
            Some(ArgsKind::EmptyOr) => String::new(),
            _ => key.to_owned(),
        };
        let code = view.unwrap_or(&self.locale);
        match self.lookup(view, page, key) {
            Some(Hit::Msg(s)) => {
                let form = plural_form(&s, whole, &self.tag(code));
                (interpolate_by(&form, param), None)
            }
            Some(Hit::Value(v)) if v != "true" && v != "false" => (v, None),
            Some(_) => (fallback(), None),
            None => {
                let p = page.filter(|p| !p.is_empty()).unwrap_or("index");
                (fallback(), Some(format!("{code}|{p}|{key}")))
            }
        }
    }

    /// `add_messages` / `messages`: deep-merge into the chunk the page name reads.
    fn write(&mut self, locale: &str, page: Option<&str>, tree: &Value) {
        let src = tree.as_object().unwrap();
        let key = (locale.to_owned(), self.chunk_page(page));
        merge(self.chunks.entry(key.clone()).or_default(), src, true);
        merge(self.shadow.entry(key).or_default(), src, false);
        paths(tree, "", &mut self.written);
    }

    /// `pages()`: every page chunk name and every alias, sorted.
    fn pages(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = self
            .chunks
            .keys()
            .filter(|(_, p)| p != "index")
            .map(|(_, p)| p.clone())
            .collect();
        out.extend(self.case.aliases.iter().map(|(a, _)| (*a).to_owned()));
        out.into_iter().collect()
    }

    /// `locales()`: configured and enabled in file order, then the other known ones
    /// sorted; with the `iso` of the builder, else of the file.
    fn locales(&self) -> Vec<(String, Option<String>)> {
        let iso = |code: &str, file: Option<&str>| match self.case.iso {
            Some((c, tag)) if c == code => Some(tag.to_owned()),
            _ => file.map(str::to_owned),
        };
        let mut out: Vec<(String, Option<String>)> = self
            .case
            .configured
            .iter()
            .filter(|c| !Self::disabled(c.0))
            .map(|c| (c.0.to_owned(), iso(c.0, c.2)))
            .collect();
        let mut rest: BTreeSet<String> = self.chunks.keys().map(|(l, _)| l.clone()).collect();
        rest.extend(
            POOL.iter()
                .chain(&[UNKNOWN])
                .filter(|c| self.named(c))
                .map(|c| (*c).to_owned()),
        );
        out.extend(rest.into_iter().filter(|c| !self.configured(c)).map(|c| {
            let i = iso(&c, None);
            (c, i)
        }));
        out
    }

    /// A lookup key: mostly one already written, sometimes a fixed one.
    fn key(&self, pick: usize) -> String {
        if pick.is_multiple_of(4) || self.written.is_empty() {
            FIXED_KEYS[pick / 4 % FIXED_KEYS.len()].to_owned()
        } else {
            self.written
                .iter()
                .nth(pick % self.written.len())
                .unwrap()
                .clone()
        }
    }

    /// `(locale, page, key)` where the page and the root of one enabled locale both answer,
    /// differently.
    fn both(&self, pick: usize) -> Option<(String, String, String)> {
        let mut found = Vec::new();
        for (code, page) in self.chunks.keys() {
            if page == "index" || !self.enabled(code) {
                continue;
            }
            for key in &self.written {
                let p = self.in_locale(code, page, key, false, false);
                let r = self.in_locale(code, "index", key, false, false);
                if p.is_some() && r.is_some() && p != r {
                    found.push((code.clone(), page.clone(), key.clone()));
                }
            }
        }
        (!found.is_empty()).then(|| found.swap_remove(pick % found.len()))
    }
}

/// `createLocalePlural` (`R` in the `text` chunk of `@i18n-micro/core` 2.1.0 `dist/`),
/// transcribed: `null` when `Intl` does not support the tag, else
/// `{ small, named, all, unmarked, shared }`. Categories are numbers in the order of
/// `CATEGORY_ORDER` (`zero` 0 … `other` 5); `rules.select` is the generated rule (checked
/// against `Intl` by `plural_props`), the only piece taken from the crate.
struct LocalePlural {
    select: fn(&plural::Operands) -> plural::Category,
    /// `Array.from({ length: 200 }, (_, n) => rules.select(n))`
    small: Vec<u8>,
    /// `order(small).filter((c) => c !== 'zero')`
    named: Vec<u8>,
    /// `order(rules.resolvedOptions().pluralCategories)`
    all: Vec<u8>,
    /// `named.length >= 3`
    unmarked: bool,
    /// `named.map((c) => small.indexOf(c) !== small.lastIndexOf(c))`
    shared: Vec<bool>,
}

fn create_locale_plural(tag: &str) -> Option<LocalePlural> {
    let (select, mask) = plural::rule(plural::rule_index(tag)?);
    let small: Vec<u8> = (0..200u64)
        .map(|n| select(&plural::Operands::int(n)) as u8)
        .collect();
    // `order(list)`: CATEGORY_ORDER filtered by the set of `list`
    let named: Vec<u8> = (0..6u8).filter(|c| small.contains(c) && *c != 0).collect();
    let all = (0..6u8).filter(|c| mask & (1 << c) != 0).collect();
    let index_of = |c: u8| small.iter().position(|&x| x == c);
    let last_index_of = |c: u8| small.iter().rposition(|&x| x == c);
    let shared = named
        .iter()
        .map(|&c| index_of(c) != last_index_of(c))
        .collect();
    Some(LocalePlural {
        select,
        unmarked: named.len() >= 3,
        small,
        named,
        all,
        shared,
    })
}

/// `categoryOf` (`B`): `small[n] ?? rules.select(n)`.
fn category_of(p: &LocalePlural, n: u64) -> u8 {
    match p.small.get(n as usize) {
        Some(&c) => c,
        None => (p.select)(&plural::Operands::int(n)) as u8,
    }
}

/// `unmarkedIndex` (`V`): -1 when the forms do not have the category shape.
fn unmarked_index(forms: &[&str], t: u64, p: &LocalePlural) -> i64 {
    if !p.unmarked
        || forms.len() != p.named.len() + 1
        || forms[0].contains('{')
        || p.shared
            .iter()
            .enumerate()
            .any(|(n, &s)| s && !forms[n + 1].contains("{count}"))
    {
        return -1;
    }
    if t == 0 {
        return 0;
    }
    match p.named.iter().position(|&c| c == category_of(p, t)) {
        None => forms.len() as i64 - 1,
        Some(o) => o as i64 + 1,
    }
}

/// `pluralIndex` (`H`); `t` is `None` when `!Number.isInteger(t) || t < 0`.
fn plural_index(forms: &[&str], t: Option<u64>, tag: &str) -> usize {
    let r = forms.len() - 1;
    let Some(t) = t else {
        return r;
    };
    let a = match create_locale_plural(tag) {
        Some(i) => unmarked_index(forms, t, &i),
        None => -1,
    };
    if a == -1 {
        (t as usize).min(r)
    } else {
        a as usize
    }
}

/// `markedIndex` (`U`), for a text that starts with `=0 `.
fn marked_index(forms: &[&str], t: Option<u64>, tag: &str) -> usize {
    let r = forms.len() - 1;
    let Some(t) = t else {
        return r;
    };
    if t == 0 {
        return 0;
    }
    let Some(i) = create_locale_plural(tag) else {
        return (t as usize).min(r);
    };
    let list = if r > i.named.len() { &i.all } else { &i.named };
    match list.iter().position(|&c| c == category_of(&i, t)) {
        None => r,
        Some(a) => (a + 1).min(r),
    }
}

/// `pluralForm(text, count, locale)` (`W`): `whole` is the count when it is a whole number
/// ≥ 0. Counts a category rule answers differently from the index rule.
fn plural_form(text: &str, whole: Option<u64>, tag: &str) -> String {
    let marked = text.starts_with("=0 ");
    let forms: Vec<&str> = (if marked { &text[3..] } else { text })
        .split('|')
        .collect();
    let index = if marked {
        marked_index(&forms, whole, tag)
    } else {
        plural_index(&forms, whole, tag)
    };
    if whole.is_some_and(|n| index != (n as usize).min(forms.len() - 1)) {
        CATEGORY_SEEN.fetch_add(1, Ordering::Relaxed);
    }
    // `.trim()`
    forms[index].trim_matches(js_space).to_owned()
}

/// `(locale, page)` of a file path of [`file_path`].
fn classify(path: &str) -> (&str, Option<&'static str>) {
    let stem = path.strip_suffix(".json").unwrap();
    let (dir, code) = stem.rsplit_once('/').unwrap_or(("", stem));
    let page = match dir {
        "" => None,
        "pages/p" => Some("p"),
        "pages/q" => Some("q"),
        _ => Some("n-m"),
    };
    (code, page)
}

/// The `i18n.json` of a case; `default` overrides `defaultLocale`.
fn config_json(case: &Case, default: Option<&str>) -> String {
    let mut locales = Vec::new();
    for (code, fb, iso) in &case.configured {
        let mut entry = Map::new();
        entry.insert("code".into(), Value::from(*code));
        if let Some(fb) = fb {
            entry.insert("fallbackLocale".into(), Value::from(*fb));
        }
        if let Some(iso) = iso {
            entry.insert("iso".into(), Value::from(*iso));
        }
        if *code == "es" {
            entry.insert("disabled".into(), Value::from(true));
        }
        locales.push(Value::Object(entry));
    }
    let mut config = Map::new();
    config.insert("locales".into(), Value::Array(locales));
    if let Some(d) = default {
        config.insert("defaultLocale".into(), Value::from(d));
    }
    Value::Object(config).to_string()
}

/// The builder of a case without its data: config, start, chains, iso, aliases, pages.
fn builder(case: &Case, default: Option<&str>) -> i18n_next::Builder {
    let mut b = I18n::builder()
        .config_json(&config_json(case, default))
        .pages(case.pages);
    for &(a, f) in &case.links {
        b = b.fallback_for(a, f);
    }
    if let Some(fb) = case.fallback {
        b = b.fallback(fb);
    }
    if let Some((code, tag)) = case.iso {
        b = b.iso(code, tag);
    }
    for &(from, to) in &case.aliases {
        b = b.page_alias(from, to);
    }
    b
}

fn infos(i18n: &I18n) -> Vec<(String, Option<String>)> {
    i18n.locales()
        .into_iter()
        .map(|l| (l.code, l.iso))
        .collect()
}

fn run(case: &Case) -> Result<(), TestCaseError> {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let start = case.codes[0];
    let mut b = builder(case, matches!(case.start, Start::Config).then_some(start))
        .files(
            case.files
                .iter()
                .map(|(p, t)| (p.clone(), t.to_string().into_bytes())),
        )
        .missing(move |l, k, p| log.lock().unwrap().push(format!("{l}|{p}|{k}")));
    if matches!(case.start, Start::Builder) {
        b = b.locale(start);
    }
    for (code, page, tree) in &case.messages {
        b = b.messages(code, *page, &tree.to_string());
    }
    if case.pages {
        for (_, to) in &case.aliases {
            b = b.messages(start, Some(to), "{}");
        }
    }
    let i18n = b
        .build()
        .map_err(|e| TestCaseError::fail(format!("build: {e}")))?;
    let mut m = Model::new(case);

    // Handed out before the session: a shared value and a live page view.
    let kept_key = m
        .written
        .iter()
        .find(|k| {
            matches!(
                m.lookup_as(None, None, k, false, false),
                Some(Hit::Msg(_) | Hit::Value(_))
            )
        })
        .cloned();
    let kept = kept_key.as_deref().and_then(|k| i18n.get(k));
    let kept_text = kept.as_deref().map(str::to_owned);
    let kept_view = i18n.page("p");
    let mut versions = vec![i18n.version()];
    let mut adds: Vec<(&'static str, Option<&'static str>, Value)> = Vec::new();

    prop_assert_eq!(i18n.pages(), m.pages(), "after build: pages()");
    prop_assert_eq!(infos(&i18n), m.locales(), "after build: locales()");
    prop_assert_eq!(&*i18n.locale(), m.locale.as_str(), "after build: locale()");

    for (step, action) in case.actions.iter().enumerate() {
        let mut expected_missing: Vec<String> = Vec::new();
        let add =
            |m: &mut Model, locale: &'static str, page: Option<&'static str>, tree: &Value| {
                i18n.add_messages(locale, page, &tree.to_string())
                    .map_err(|e| TestCaseError::fail(format!("step {step}: {e}")))?;
                m.write(locale, page, tree);
                m.version += 1;
                Ok::<(), TestCaseError>(())
            };
        match action {
            Action::Add { locale, page, tree } => {
                let code = m.code(*locale);
                add(&mut m, code, *page, tree)?;
                adds.push((code, *page, tree.clone()));
            }
            Action::ReAdd(i) => {
                if !adds.is_empty() {
                    let (code, page, tree) = adds[i % adds.len()].clone();
                    add(&mut m, code, page, &tree)?;
                }
            }
            Action::SetLocale(loc) => {
                let code = m.code(*loc);
                let got = i18n.set_locale(code);
                match &got {
                    Ok(()) if m.enabled(code) => {
                        if m.locale != code {
                            m.locale = code.to_owned();
                            m.version += 1;
                        }
                    }
                    Err(Error::DisabledLocale(_)) if Model::disabled(code) && m.known(code) => {}
                    Err(Error::UnknownLocale(_)) if !m.known(code) => {}
                    _ => prop_assert!(
                        false,
                        "step {step}: set_locale({code}) = {got:?}; known {}, disabled {}",
                        m.known(code),
                        Model::disabled(code)
                    ),
                }
            }
            Action::Lookup {
                via,
                key,
                page,
                locale,
            } => {
                let vloc = locale.map(|l| m.code(l));
                // `tc` mostly on a key that holds a plural template in this view
                let key = match via {
                    Via::Tc(..) if key % 4 != 0 => m.plural_key(vloc, *page, *key),
                    _ => None,
                }
                .unwrap_or_else(|| m.key(*key));
                expected_missing.extend(check_lookup(&i18n, &m, *via, vloc, *page, &key, step)?);
            }
            Action::Both { via, pick } => {
                if let Some((code, page, key)) = m.both(*pick) {
                    let vloc = (code != m.locale).then_some(code.as_str());
                    expected_missing.extend(check_lookup(
                        &i18n,
                        &m,
                        *via,
                        vloc,
                        Some(&page),
                        &key,
                        step,
                    )?);
                }
            }
            Action::BadName { name, kind } => match kind {
                0 => {
                    // A bad name is only ever a name: once `add_messages` made it a locale
                    // it switches like any other, never touching a path.
                    let got = i18n.set_locale(name);
                    if m.enabled(name) {
                        prop_assert!(got.is_ok(), "step {step}: {got:?}");
                        if m.locale != *name {
                            m.locale = (*name).to_owned();
                            m.version += 1;
                        }
                    } else {
                        prop_assert!(
                            matches!(got, Err(Error::UnknownLocale(_))),
                            "step {step}: {got:?}"
                        );
                    }
                }
                1 => {
                    let (want, miss) = m.t(None, Some(name), "a", None);
                    prop_assert_eq!(i18n.page(name).t("a"), want, "step {}", step);
                    expected_missing.extend(miss);
                }
                2 => {
                    let (want, miss) = m.t(Some(name), None, "a", None);
                    prop_assert_eq!(i18n.with_locale(name).t("a"), want, "step {}", step);
                    prop_assert_eq!(
                        i18n.with_locale(name).has("a"),
                        m.has(Some(name), None, "a")
                    );
                    expected_missing.extend(miss);
                }
                _ => {
                    let got = i18n.add_messages(name, None, r#"{"a": "bad"}"#);
                    if name.is_empty() || *name == "__proto__" {
                        prop_assert!(
                            matches!(got, Err(Error::UnknownLocale(_))),
                            "step {step}: {got:?}"
                        );
                    } else {
                        prop_assert!(got.is_ok(), "step {step}: {got:?}");
                        m.write(name, None, &serde_json::json!({"a": "bad"}));
                        m.version += 1;
                    }
                }
            },
        }
        let missing: Vec<String> = std::mem::take(&mut *seen.lock().unwrap());
        prop_assert_eq!(
            missing,
            expected_missing,
            "step {}: missing calls of {:?}",
            step,
            action
        );
        prop_assert_eq!(i18n.version(), m.version, "step {}: version", step);
        prop_assert_eq!(
            &*i18n.locale(),
            m.locale.as_str(),
            "step {}: locale()",
            step
        );
        prop_assert_eq!(i18n.pages(), m.pages(), "step {}: pages()", step);
        prop_assert_eq!(infos(&i18n), m.locales(), "step {}: locales()", step);
        versions.push(i18n.version());
    }

    // A value handed out before the session is unchanged; versions only grew.
    prop_assert_eq!(kept.as_deref().map(str::to_owned), kept_text);
    prop_assert!(versions.windows(2).all(|w| w[0] <= w[1]));

    // From scratch: the model's chunks through `messages`, the same config, the session's
    // start locale declared (so the same locales are known) and its active locale.
    let start_code = match case.start {
        Start::Default => "en",
        _ => start,
    };
    let mut fresh = builder(case, Some(start_code)).locale(&m.locale);
    for ((code, page), tree) in &m.chunks {
        let page = (page != "index").then_some(page.as_str());
        fresh = fresh.messages(code, page, &Value::Object(tree.clone()).to_string());
    }
    let fresh = fresh
        .build()
        .map_err(|e| TestCaseError::fail(format!("fresh build: {e}")))?;
    prop_assert_eq!(fresh.pages(), i18n.pages(), "fresh pages()");
    prop_assert_eq!(infos(&fresh), infos(&i18n), "fresh locales()");
    let mut keys: BTreeSet<String> = FIXED_KEYS.iter().map(|k| (*k).to_owned()).collect();
    keys.extend(m.written.iter().cloned());
    let mut codes: Vec<&str> = case.codes.clone();
    codes.extend([UNKNOWN, "en"]);
    for code in codes {
        for page in PAGES {
            let (a, b) = match page {
                Some(p) => (
                    i18n.with_locale(code).page(p),
                    fresh.with_locale(code).page(p),
                ),
                None => (i18n.with_locale(code), fresh.with_locale(code)),
            };
            for key in &keys {
                let want = m
                    .lookup_as(Some(code), *page, key, false, false)
                    .and_then(|h| h.text());
                prop_assert_eq!(
                    a.try_t(key),
                    want.clone(),
                    "end: {} {:?} {:?}",
                    code,
                    page,
                    key
                );
                prop_assert_eq!(b.try_t(key), want, "fresh: {} {:?} {:?}", code, page, key);
                prop_assert_eq!(
                    a.has(key),
                    b.has(key),
                    "fresh has: {} {:?} {:?}",
                    code,
                    page,
                    key
                );
            }
        }
    }
    // The view made before the session reads the live catalog.
    let now = i18n.page("p");
    for key in &keys {
        prop_assert_eq!(kept_view.try_t(key), now.try_t(key), "kept view: {:?}", key);
    }
    Ok(())
}

/// One lookup through `via` on the view `vloc` / `page`; returns the expected `missing`.
fn check_lookup(
    i18n: &I18n,
    m: &Model,
    via: Via,
    vloc: Option<&str>,
    page: Option<&str>,
    key: &str,
    step: usize,
) -> Result<Option<String>, TestCaseError> {
    let view = match (vloc, page) {
        (Some(code), Some(p)) => Some(i18n.with_locale(code).page(p)),
        (Some(code), None) => Some(i18n.with_locale(code)),
        (None, Some(p)) => Some(i18n.page(p)),
        (None, None) => None,
    };
    if let (Some(v), Some(code)) = (&view, vloc) {
        prop_assert_eq!(&*v.locale(), code);
    }
    let what = format!("step {step}: {via:?} {key:?} on {vloc:?} / {page:?}");
    match via {
        Via::T | Via::TWith(_) => {
            let kind = match via {
                Via::TWith(k) => Some(k),
                _ => None,
            };
            let args = kind.map(ArgsKind::args);
            let got = match (&view, &args) {
                (Some(v), None) => v.t(key).into(),
                (Some(v), Some(a)) => v.t_with(key, a),
                (None, None) => i18n.t(key).into(),
                (None, Some(a)) => i18n.t_with(key, a),
            };
            let (want, miss) = m.t(vloc, page, key, kind);
            prop_assert_eq!(got, want, "{}", what);
            Ok(miss)
        }
        Via::TryT | Via::Get => {
            let got = match (&view, via) {
                (Some(v), Via::TryT) => v.try_t(key),
                (Some(v), _) => v.get(key).map(|s| s.to_string()),
                (None, Via::TryT) => i18n.try_t(key),
                (None, _) => i18n.get(key).map(|s| s.to_string()),
            };
            let want = m.lookup(vloc, page, key).and_then(|h| h.text());
            prop_assert_eq!(got, want, "{}", what);
            Ok(None)
        }
        Via::Has => {
            let got = view.as_ref().map_or_else(|| i18n.has(key), |v| v.has(key));
            prop_assert_eq!(got, m.has(vloc, page, key), "{}", what);
            Ok(None)
        }
        Via::Tc(c, kind) => {
            let count = counts()[usize::from(c) % 16];
            let args = kind.map(ArgsKind::args);
            let got = match (&view, &args) {
                (Some(v), None) => v.tc(key, count.0),
                (Some(v), Some(a)) => v.tc_with(key, count.0, a),
                (None, None) => i18n.tc(key, count.0),
                (None, Some(a)) => i18n.tc_with(key, count.0, a),
            };
            let (want, miss) = m.tc(vloc, page, key, count, kind);
            prop_assert_eq!(got, want, "{} count {}", what, count.1);
            Ok(miss)
        }
    }
}

#[test]
fn catalog_matches_the_model() {
    let config = Config {
        cases: 384,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[11; 32]));
    runner.run(&case(), |c| run(&c)).unwrap();
    // The session space reached what the model must distinguish.
    let layer = LAYER_ORDER_SEEN.load(Ordering::Relaxed);
    let deep = DEEP_MERGE_SEEN.load(Ordering::Relaxed);
    let category = CATEGORY_SEEN.load(Ordering::Relaxed);
    eprintln!(
        "lookups that see the layer order: {layer}, the deep merge: {deep}, a category rule: {category}"
    );
    assert!(category > 0, "no tc answer depended on a category rule");
    assert!(
        layer > 0,
        "no lookup depended on the page layer coming before the root"
    );
    assert!(
        deep > 0,
        "no lookup depended on objects merging into objects"
    );
}
