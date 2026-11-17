//! The compat goldens (`fixtures/compat`, DESIGN 7.2) replayed through the public API.
//! Shared by `tests/compat.rs` (native) and `tests/wasm.rs` (wasm-bindgen-test): fixtures
//! and goldens are compiled in, so the same comparison runs where there is no file system.
//!
//! Every call line of `load.md`, `lookup.md`, `interpolate.md` and `options.md`
//! (`pages(false)`, `page_alias`, fallback chains from `i18n.json`) is executed and printed
//! back as `call → result`; the produced file is compared with the golden as a whole.
//! The Rust contract of `fixtures/compat/README.md` maps the markers on both sides:
//! - a number or boolean result is the JS spelling as a string (`1e+21` → `"1e+21"`);
//! - `<object>` and `<array>` are both `<node>`: `get` is `None` and `t` returns the key
//!   without calling `missing` (Rust cannot tell the two apart, and need not);
//! - `<null>` is `<missing>`: `get` is `None` and `t` calls `missing` (or the key is
//!   `""`, which `t` answers with `""` and never reports).
#![allow(dead_code)]

use i18n_next::{Arg, Args, Count, Error, I18n, Text, View};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

macro_rules! fixtures {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../../../../fixtures/compat/locales/", $path)) as &[u8])),*]
    };
}

/// Every file of `fixtures/compat/locales`, in path order (`tests/compat.rs` checks the
/// list against the directory, so a new fixture cannot be forgotten here).
pub const FILES: &[(&str, &[u8])] = fixtures![
    ".json",
    "__proto__.json",
    "broken.json",
    "de-at.json",
    "de.json",
    "en.json",
    "list.json",
    "nested/en.json",
    "pages/__proto__/en.json",
    "pages/about/de.json",
    "pages/about/en.json",
    "pages/about/fr.json",
    "pages/en.json",
    "pages/fold/a/ru.json",
    "pages/fold/b/ru.json",
    "pages/settings/en.json",
    "pages/user-profile/en.json",
    "pages/user/profile/de.json",
    "pages/user/profile/en.json",
    "pages/\u{ff61}/en.json",
    "pages/\u{1f600}/en.json",
    "ru.json",
];

/// `routesLocaleLinks` of `options.md` (`scripts/compat.mjs` `LINKS`), as `page_alias`.
const LINKS: &[(&str, &str)] = &[
    ("about-us", "about"),
    ("profile", "user-profile"),
    ("cfg", "settings"),
];

/// The chain config of `options.md` (`scripts/compat.mjs` `CHAIN_LOCALES`, global `en`).
const CHAIN_CONFIG: &str = r#"{"fallbackLocale": "en", "locales": [
    {"code": "en", "fallbackLocale": "de-at"}, {"code": "de", "fallbackLocale": "ru"},
    {"code": "de-at", "fallbackLocale": "de"},
    {"code": "ru", "fallbackLocale": "de-at", "disabled": true},
    {"code": "fr"}, {"code": "pt", "fallbackLocale": "zz"}]}"#;

pub const GOLDENS: &[(&str, &str)] = &[
    (
        "load.md",
        include_str!("../../../../fixtures/compat/__golden__/load.md"),
    ),
    (
        "lookup.md",
        include_str!("../../../../fixtures/compat/__golden__/lookup.md"),
    ),
    (
        "interpolate.md",
        include_str!("../../../../fixtures/compat/__golden__/interpolate.md"),
    ),
    (
        "options.md",
        include_str!("../../../../fixtures/compat/__golden__/options.md"),
    ),
    (
        "plural.md",
        include_str!("../../../../fixtures/compat/__golden__/plural.md"),
    ),
    (
        "text.md",
        include_str!("../../../../fixtures/compat/__golden__/text.md"),
    ),
];

/// The catalog of `tc_in` lines (`fixtures/compat/plural`, `scripts/compat.mjs`
/// `PLURAL_DIR`); `tests/compat.rs` checks the list against the directory.
pub const PLURAL_FILES: &[(&str, &[u8])] = &[
    (
        "en.json",
        include_bytes!("../../../../fixtures/compat/plural/en.json"),
    ),
    (
        "pages/about/en.json",
        include_bytes!("../../../../fixtures/compat/plural/pages/about/en.json"),
    ),
    (
        "ru.json",
        include_bytes!("../../../../fixtures/compat/plural/ru.json"),
    ),
];

/// The catalog of `text.md` (`fixtures/compat/text`, `scripts/compat.mjs` `TEXT_DIR`);
/// `tests/compat.rs` checks the list against the directory.
pub const TEXT_FILES: &[(&str, &[u8])] = &[
    (
        "en.json",
        include_bytes!("../../../../fixtures/compat/text/en.json"),
    ),
    (
        "pages/about/en.json",
        include_bytes!("../../../../fixtures/compat/text/pages/about/en.json"),
    ),
    (
        "ru.json",
        include_bytes!("../../../../fixtures/compat/text/ru.json"),
    ),
];

/// Golden lines Rust does not reproduce, with the reason. Keep it minimal.
pub const EXCLUDED: &[(&str, &str)] = &[];

/// A value of a call: JSON plus the JS values JSON cannot spell.
#[derive(Debug, Clone, PartialEq)]
enum V {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    Undef,
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
    /// `"de-at"|"de"`: locale and fallback.
    Pair(String, String),
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, w: &str) -> bool {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> String {
        assert!(self.eat(b'"'));
        let mut units: Vec<u16> = Vec::new();
        loop {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.s[self.i];
                    self.i += 1;
                    let u = match e {
                        b'n' => 10,
                        b't' => 9,
                        b'r' => 13,
                        b'b' => 8,
                        b'f' => 12,
                        b'u' => {
                            let hex = std::str::from_utf8(&self.s[self.i..self.i + 4]).unwrap();
                            self.i += 4;
                            u16::from_str_radix(hex, 16).unwrap()
                        }
                        other => u16::from(other),
                    };
                    units.push(u);
                }
                _ => {
                    // Copy one whole UTF-8 character.
                    let start = self.i - 1;
                    let len = match c {
                        0..0x80 => 1,
                        0xc0..0xe0 => 2,
                        0xe0..0xf0 => 3,
                        _ => 4,
                    };
                    self.i = start + len;
                    let ch = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                    units.extend(ch.encode_utf16());
                }
            }
        }
        String::from_utf16(&units).unwrap()
    }

    fn value(&mut self) -> V {
        self.ws();
        let first = self.s[self.i];
        let v = match first {
            b'"' => {
                let a = self.string();
                if self.s.get(self.i) == Some(&b'|') {
                    self.i += 1;
                    V::Pair(a, self.string())
                } else {
                    V::Str(a)
                }
            }
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                while !self.eat(b'}') {
                    self.eat(b',');
                    let k = self.string();
                    assert!(self.eat(b':'));
                    items.push((k, self.value()));
                }
                V::Obj(items)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                while !self.eat(b']') {
                    self.eat(b',');
                    items.push(self.value());
                }
                V::Arr(items)
            }
            _ if self.word("undefined") => V::Undef,
            _ if self.word("null") => V::Null,
            _ if self.word("true") => V::Bool(true),
            _ if self.word("false") => V::Bool(false),
            _ if self.word("NaN") => V::Num(f64::NAN),
            _ if self.word("Infinity") => V::Num(f64::INFINITY),
            _ if self.word("-Infinity") => V::Num(f64::NEG_INFINITY),
            _ => {
                let start = self.i;
                while self.i < self.s.len() && b"+-.0123456789eE".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                V::Num(text.parse().unwrap_or_else(|_| panic!("number {text:?}")))
            }
        };
        self.ws();
        v
    }
}

/// `name(args…)` → name and the parsed arguments.
fn parse_call(call: &str) -> (&str, Vec<V>) {
    let open = call.find('(').unwrap();
    let inner = &call[open + 1..call.len() - 1];
    let mut p = P {
        s: inner.as_bytes(),
        i: 0,
    };
    let mut args = Vec::new();
    while p.i < p.s.len() {
        args.push(p.value());
        p.eat(b',');
    }
    (&call[..open], args)
}

/// JS `String(v)` kept as a param.
fn arg(v: &V) -> Option<Arg<'_>> {
    Some(match v {
        V::Str(s) => Arg::Str(s),
        V::Num(n) => Arg::Float(*n),
        V::Bool(b) => Arg::Bool(*b),
        V::Null => Arg::Null,
        V::Undef => return None,
        V::Obj(_) => Arg::Str("[object Object]"),
        other => panic!("unsupported param {other:?}"),
    })
}

/// Params of `interpolate` / `t`: an object by own key, an array as a list.
fn args(v: &V) -> Option<Args<'_>> {
    match v {
        V::Undef | V::Null => None,
        V::Obj(items) => Some(items.iter().fold(Args::new(), |a, (k, v)| match arg(v) {
            Some(value) => a.arg(k, value),
            None => a,
        })),
        V::Arr(items) => Some(Args::list(items.iter().map(|v| arg(v).unwrap()))),
        other => panic!("unsupported params {other:?}"),
    }
}

/// `JSON.stringify(s)` plus the README's invisible code units as `\uXXXX`.
pub fn spell(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        let u = c as u32;
        let invisible = matches!(u,
            0x7f..=0xa0 | 0xad | 0x1680 | 0x180e | 0x2000..=0x200f | 0x2028..=0x202f
            | 0x205f..=0x206f | 0x3000 | 0xfe00..=0xfe0f | 0xfeff | 0xfff9..=0xfffb);
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            _ if u < 0x20 || invisible => out.push_str(&format!("\\u{u:04x}")),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The golden result in the Rust contract's terms.
fn normalize(call: &str, result: &str) -> String {
    match result {
        "<object>" | "<array>" => "<node>".to_owned(),
        "<null>" => "<missing>".to_owned(),
        r if r.starts_with('"') || r == "<missing>" || r == "<node>" => r.to_owned(),
        // a `fold` result: a list of spelled strings
        r if r.starts_with('[') => r.to_owned(),
        r if call.starts_with("has(") => r.to_owned(),
        // a number or boolean value: Rust returns its JS spelling as text
        r => spell(r),
    }
}

/// An instance and its `missing` call counter.
struct Probe {
    i18n: I18n,
    misses: Arc<AtomicUsize>,
}

impl Probe {
    fn build(b: i18n_next::Builder) -> Result<Probe, Error> {
        let misses = Arc::new(AtomicUsize::new(0));
        let counter = misses.clone();
        let i18n = b
            .missing(move |_, _, _| {
                counter.fetch_add(1, Ordering::Relaxed);
            })
            .build()?;
        Ok(Probe { i18n, misses })
    }

    /// Runs `t`-like `f` and tells a miss from a subtree when `get` found nothing.
    fn answer(&self, view: &View, key: &str, f: impl FnOnce() -> String) -> String {
        let before = self.misses.load(Ordering::Relaxed);
        let out = f();
        let missed = self.misses.load(Ordering::Relaxed) > before;
        if missed {
            return spell(&out);
        }
        if view.get(key).map(str::to_owned).is_none() && out == key {
            return if key.is_empty() {
                "<missing>"
            } else {
                "<node>"
            }
            .to_owned();
        }
        spell(&out)
    }

    /// Raw lookup: `get`, or which kind of nothing.
    fn lookup(&self, view: &View, key: &str) -> String {
        match view.get(key).map(str::to_owned) {
            Some(s) => spell(&s),
            None => {
                let before = self.misses.load(Ordering::Relaxed);
                let _ = view.t(key);
                if self.misses.load(Ordering::Relaxed) > before || key.is_empty() {
                    "<missing>".to_owned()
                } else {
                    "<node>".to_owned()
                }
            }
        }
    }
}

/// Runs every golden; `Err` holds the diff (`-` golden, `+` Rust) of every file.
pub fn run() -> Result<usize, String> {
    // Each file alone: its fate, as `file()` / `file_nopages()` report it.
    let fate = fates(false);
    let fate_nopages = fates(true);
    // The catalog: the fixture tree minus the files Rust refuses (JS skips them).
    let valid: Vec<(&str, &[u8])> = FILES
        .iter()
        .copied()
        .filter(|(p, _)| !fate[p].starts_with("skip"))
        .collect();
    let mut contexts: BTreeMap<(String, String), Probe> = BTreeMap::new();
    let mut templates: BTreeMap<String, Probe> = BTreeMap::new();
    let mut folded: BTreeMap<String, Probe> = BTreeMap::new();
    let aliased = {
        let mut b = I18n::builder()
            .locale("de")
            .fallback("en")
            .files(valid.iter().copied());
        for (from, to) in LINKS {
            b = b.page_alias(from, to);
        }
        Probe::build(b).unwrap()
    };
    let chained = Probe::build(
        I18n::builder()
            .config_json(CHAIN_CONFIG)
            .locale("en")
            .files(valid.iter().copied()),
    )
    .unwrap();

    let mut texts: BTreeMap<(String, String), (I18n, Log)> = BTreeMap::new();
    let mut singles: BTreeMap<(bool, String, String), I18n> = BTreeMap::new();
    let mut counted: BTreeMap<(String, String), (I18n, Log)> = BTreeMap::new();

    let mut diff = String::new();
    let mut total = 0;
    for (name, text) in GOLDENS {
        let mut expected = Vec::new();
        let mut produced = Vec::new();
        for line in text.lines() {
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with('>')
                || line.starts_with("```")
            {
                continue;
            }
            let (call, result) = line.split_once(" → ").unwrap();
            if EXCLUDED.iter().any(|(c, _)| *c == call) {
                continue;
            }
            total += 1;
            expected.push(format!("{call} → {}", normalize(call, result)));
            let (fname, a) = parse_call(call);
            let got = match (fname, a.as_slice()) {
                ("file", [V::Str(path)]) => spell(&fate[path.as_str()]),
                ("file_nopages", [V::Str(path)]) => spell(&fate_nopages[path.as_str()]),
                ("load_nopages", [V::Str(locale), V::Str(key)]) => {
                    let probe = folded.entry(locale.clone()).or_insert_with(|| {
                        let b = I18n::builder()
                            .pages(false)
                            .locale(locale)
                            .files(valid.iter().copied());
                        Probe::build(b).unwrap()
                    });
                    let view = probe.i18n.page("index");
                    probe.lookup(&view, key)
                }
                ("alias_t", [V::Str(page), V::Str(key)]) => {
                    aliased.lookup(&aliased.i18n.page(page), key)
                }
                ("chain_t", [V::Str(locale), V::Str(page), V::Str(key)]) => {
                    chained.lookup(&chained.i18n.with_locale(locale).page(page), key)
                }
                ("tc" | "tc_code", [V::Str(tag), V::Str(template), V::Num(n)]) => {
                    let iso = fname == "tc";
                    let i18n = singles
                        .entry((iso, tag.clone(), template.clone()))
                        .or_insert_with(|| single(iso, tag, template));
                    let out = i18n.tc("k", count(*n));
                    // An integer past 2^53 as a u64 / i64 count: what JS holds, the same answer.
                    match int_count(*n).map(|c| i18n.tc("k", c)) {
                        Some(int) if int != out => format!("<f64 {out:?}, integer {int:?}>"),
                        _ => spell(&out),
                    }
                }
                (
                    "tc_in" | "tc_missing",
                    [
                        V::Pair(locale, fallback),
                        V::Str(page),
                        V::Str(key),
                        V::Num(n),
                        params,
                        default,
                    ],
                ) => {
                    let (i18n, missed) = counted
                        .entry((locale.clone(), fallback.clone()))
                        .or_insert_with(|| plural_catalog(locale, fallback));
                    missed.lock().unwrap().clear();
                    let view = i18n.page(page);
                    let mut a = args(params);
                    if let V::Str(d) = default {
                        a = Some(a.unwrap_or_default().or(d));
                    }
                    let out = match &a {
                        None => view.tc(key, count(*n)),
                        Some(a) => view.tc_with(key, count(*n), a),
                    };
                    if fname == "tc_in" {
                        spell(&out)
                    } else {
                        spell(&missed.lock().unwrap().join("; "))
                    }
                }
                (
                    "text" | "text_missing",
                    [V::Pair(locale, fallback), V::Str(page), V::Str(key), opts],
                ) => {
                    let (i18n, missed) = texts
                        .entry((locale.clone(), fallback.clone()))
                        .or_insert_with(|| catalog_of(TEXT_FILES, locale, fallback));
                    missed.lock().unwrap().clear();
                    let view = i18n.page(page);
                    let out = text_of(&view, key, opts).to_string();
                    let plural = field(opts, "plural").is_some();
                    let get = field(opts, "customPluralRule") == Some(&V::Str("get".into()));
                    let node = !key.is_empty()
                        && view.get(key).map(str::to_owned).is_none()
                        && missed.lock().unwrap().is_empty();
                    if fname == "text_missing" {
                        spell(&missed.lock().unwrap().join("; "))
                    } else if node && (!plural || get) && out == *key {
                        "<node>".to_owned()
                    } else {
                        spell(&out)
                    }
                }
                ("fold", [V::Str(key), V::Arr(names), opts]) => {
                    let (i18n, _) = texts
                        .entry(("en".to_owned(), "en".to_owned()))
                        .or_insert_with(|| catalog_of(TEXT_FILES, "en", "en"));
                    let names: Vec<String> = names
                        .iter()
                        .map(|n| match n {
                            V::Str(n) => n.clone(),
                            other => panic!("slot {other:?}"),
                        })
                        .collect();
                    let fns: Vec<Box<dyn Fn() -> String>> = names
                        .iter()
                        .map(|n| {
                            let marker = format!("<{n}>");
                            Box::new(move || marker.clone()) as Box<dyn Fn() -> String>
                        })
                        .collect();
                    let slots: Vec<(&str, &dyn Fn() -> String)> = names
                        .iter()
                        .zip(&fns)
                        .map(|(n, f)| (n.as_str(), &**f))
                        .collect();
                    let nodes = text_of(&i18n.page("index"), key, opts).fold(str::to_owned, &slots);
                    let spelled: Vec<String> = nodes.iter().map(|n| spell(n)).collect();
                    format!("[{}]", spelled.join(","))
                }
                _ => answer(
                    call,
                    fname,
                    &a,
                    &fate,
                    &valid,
                    &mut contexts,
                    &mut templates,
                ),
            };
            produced.push(format!("{call} → {got}"));
        }
        expected.sort();
        produced.sort();
        if expected != produced {
            diff.push_str(&format!("--- {name} (golden)\n+++ {name} (rust)\n"));
            for line in expected.iter().filter(|l| !produced.contains(l)) {
                diff.push_str(&format!("-{line}\n"));
            }
            for line in produced.iter().filter(|l| !expected.contains(l)) {
                diff.push_str(&format!("+{line}\n"));
            }
        }
    }
    if diff.is_empty() {
        Ok(total)
    } else {
        Err(diff)
    }
}

/// A count as the golden spells it: a safe integer through the integer `Count`, anything
/// else (`1.5`, `-0`, `NaN`, `1e+21`) through `f64`.
fn count(n: f64) -> Count {
    let int = n.fract() == 0.0
        && n.abs() < 9_007_199_254_740_992.0
        && !(n == 0.0 && n.is_sign_negative());
    if int {
        Count::from(n as i64)
    } else {
        Count::from(n)
    }
}

/// A whole count past ±2^53 that a `u64` (up to 2^64, saturating) or an `i64` holds, as
/// that integer: `Count` rounds it to the same `f64` JS holds.
fn int_count(n: f64) -> Option<Count> {
    let safe = 9_007_199_254_740_992.0;
    if n.fract() != 0.0 || n.abs() <= safe || n.abs() > 18_446_744_073_709_551_616.0 {
        return None;
    }
    if n > 0.0 {
        Some(Count::from(n as u64))
    } else if n >= i64::MIN as f64 {
        Some(Count::from(n as i64))
    } else {
        None
    }
}

/// The instance of a `tc` / `tc_code` line: locale `p` with `iso` `tag`, or the code `tag`
/// itself with no iso, and the template as message `k`.
fn single(iso: bool, tag: &str, template: &str) -> I18n {
    let json = format!("{{\"k\": {}}}", spell(template));
    let b = if iso {
        I18n::builder()
            .locale("p")
            .iso("p", tag)
            .messages("p", None, &json)
    } else {
        I18n::builder().locale(tag).messages(tag, None, &json)
    };
    b.build().unwrap()
}

/// The `missing` calls an instance recorded.
type Log = Arc<Mutex<Vec<String>>>;

/// The `tc_in` catalog in one context, with the `missing` calls it records.
fn plural_catalog(locale: &str, fallback: &str) -> (I18n, Log) {
    let missed = Arc::new(Mutex::new(Vec::new()));
    let log = missed.clone();
    let mut b = I18n::builder()
        .locale(locale)
        .files(PLURAL_FILES.iter().copied())
        .missing(move |l, k, p| log.lock().unwrap().push(format!("{l} {k} {p}")));
    if fallback != locale {
        b = b.fallback(fallback);
    }
    (b.build().unwrap(), missed)
}

/// A catalog of `files` in one context, with the `missing` calls it records.
fn catalog_of(files: &[(&str, &[u8])], locale: &str, fallback: &str) -> (I18n, Log) {
    let missed = Arc::new(Mutex::new(Vec::new()));
    let log = missed.clone();
    let mut b = I18n::builder()
        .locale(locale)
        .files(files.iter().copied())
        .missing(move |l, k, p| log.lock().unwrap().push(format!("{l} {k} {p}")));
    if fallback != locale {
        b = b.fallback(fallback);
    }
    (b.build().unwrap(), missed)
}

/// An option of a `text` line.
fn field<'v>(opts: &'v V, name: &str) -> Option<&'v V> {
    match opts {
        V::Obj(items) => items.iter().find(|(k, _)| k == name).map(|(_, v)| v),
        other => panic!("options {other:?}"),
    }
}

/// A JS param value as an owned `Text` argument; `undefined` is none.
fn text_arg(v: &V) -> Option<Arg<'static>> {
    Some(match v {
        V::Str(s) => Arg::from(s.clone()),
        V::Num(n) => Arg::Float(*n),
        V::Bool(b) => Arg::Bool(*b),
        V::Null => Arg::Null,
        V::Undef => return None,
        other => panic!("unsupported param {other:?}"),
    })
}

/// The `Text` of a `text` / `fold` line: the props of `<i18n-t>` as `Text` options, the named
/// `customPluralRule` stand-ins of `scripts/compat.mjs` (`RULES`) as closures.
fn text_of(view: &View, key: &str, opts: &V) -> Text {
    let mut t = view.text(key.to_owned());
    let V::Obj(items) = opts else {
        panic!("options {opts:?}")
    };
    for (name, v) in items {
        t = match (name.as_str(), v) {
            ("params", V::Obj(params)) => params.iter().fold(t, |t, (k, v)| match text_arg(v) {
                Some(a) => t.arg(k.clone(), a),
                None => t,
            }),
            ("params", V::Arr(items)) => t.args(items.iter().filter_map(text_arg)),
            ("html", V::Bool(true)) => t.html(),
            ("hideIfEmpty", V::Bool(true)) => t.hide_if_empty(),
            ("defaultValue", V::Str(d)) => t.or(d.clone()),
            ("plural", V::Num(n)) => t.count(count(*n)),
            ("customPluralRule", V::Str(rule)) => match rule.as_str() {
                "echo" => t.plural_rule(|c| Some(format!("{}|{}|{}", c.key, c.count, c.locale))),
                "get" => t.plural_rule(|c| Some(c.get(c.key))),
                "other" => t.plural_rule(|c| Some(c.get("missing.other"))),
                "none" => t.plural_rule(|_| None),
                other => panic!("rule {other}"),
            },
            other => panic!("unsupported option {other:?}"),
        };
    }
    t
}

/// What each fixture file becomes when loaded alone (`pages` = `.pages(..)`).
fn fates(no_pages: bool) -> BTreeMap<&'static str, String> {
    let mut fate: BTreeMap<&str, String> = BTreeMap::new();
    for (path, bytes) in FILES {
        // A named start locale, so no default locale joins `locales()`; it is filtered out.
        let b = I18n::builder()
            .pages(!no_pages)
            .locale(PROBE)
            .files([(path, bytes)]);
        let status = match b.build() {
            Err(Error::Json { .. }) => "skip: invalid JSON".to_owned(),
            Err(Error::NotObject { .. }) => "skip: not an object".to_owned(),
            Err(e) => panic!("{path}: {e}"),
            Ok(i) => {
                let locales: Vec<_> = i
                    .locales()
                    .into_iter()
                    .map(|l| l.code)
                    .filter(|c| c != PROBE)
                    .collect();
                let pages = i.pages();
                match (locales.as_slice(), pages.as_slice()) {
                    ([], []) => "ignore".to_owned(),
                    ([l], []) => format!("root {l}"),
                    ([l], [p]) => format!("page {p} {l}"),
                    other => panic!("{path}: {other:?}"),
                }
            }
        };
        fate.insert(path, status);
    }
    fate
}

/// A start locale that names no fixture, so `locales()` of a single file shows the file's.
const PROBE: &str = "zz-compat-probe";

/// The calls of `load.md`, `lookup.md` and `interpolate.md`.
fn answer(
    call: &str,
    fname: &str,
    a: &[V],
    fate: &BTreeMap<&str, String>,
    valid: &[(&str, &[u8])],
    contexts: &mut BTreeMap<(String, String), Probe>,
    templates: &mut BTreeMap<String, Probe>,
) -> String {
    match (fname, a) {
        ("file", [V::Str(path)]) => spell(&fate[path.as_str()]),
        ("load", [V::Str(locale), V::Str(page), V::Str(key)]) => {
            let mine: Vec<(&str, &[u8])> = valid
                .iter()
                .copied()
                .filter(|(p, _)| {
                    let f = &fate[p];
                    if page == "index" {
                        *f == format!("root {locale}")
                    } else {
                        *f == format!("page {page} {locale}")
                    }
                })
                .collect();
            let probe = Probe::build(I18n::builder().locale(locale).files(mine)).unwrap();
            let view = probe.i18n.page(page);
            probe.lookup(&view, key)
        }
        (
            "t" | "has",
            [
                V::Pair(locale, fallback),
                V::Str(page),
                V::Str(key),
                rest @ ..,
            ],
        ) => {
            let probe = contexts
                .entry((locale.clone(), fallback.clone()))
                .or_insert_with(|| {
                    let mut b = I18n::builder().locale(locale).files(valid.iter().copied());
                    if fallback != locale {
                        b = b.fallback(fallback);
                    }
                    Probe::build(b).unwrap()
                });
            let view = probe.i18n.page(page);
            match (fname, rest) {
                ("has", []) => view.has(key).to_string(),
                ("t", []) => probe.lookup(&view, key),
                ("t", [params, default @ ..]) => {
                    let default = match default {
                        [V::Str(d)] => Some(d.as_str()),
                        [] => None,
                        other => panic!("default {other:?}"),
                    };
                    let mut a = args(params).unwrap_or_default();
                    if let Some(d) = default {
                        a = a.or(d);
                    }
                    probe.answer(&view, key, || {
                        if args(params).is_none() && default.is_none() {
                            view.t(key).to_owned()
                        } else {
                            view.t_with(key, &a)
                        }
                    })
                }
                _ => panic!("unknown call {call}"),
            }
        }
        ("interpolate", [V::Str(template), params]) => {
            let probe = templates.entry(template.clone()).or_insert_with(|| {
                let json = format!("{{\"k\": {}}}", spell(template));
                Probe::build(I18n::builder().locale("en").messages("en", None, &json)).unwrap()
            });
            let view = probe.i18n.page("index");
            match args(params) {
                None => spell(view.t("k")),
                Some(a) => spell(&view.t_with("k", &a)),
            }
        }
        _ => panic!("unknown call {call}"),
    }
}
