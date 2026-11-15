//! `Text` model (DESIGN 7.3): random catalogs and random `Text` chains — built through
//! methods and through the `t!` / `tc!` macro forms, explicit or through a scope — checked
//! against a small reference written from the JS: `resolveText` (`lookup` with the miss
//! sentinel, `pluralText`, `escapeParams`, `keyText`, `hideIfEmpty`), `interpolate`
//! (`tokenize` = `split(/\{([^{}\s]+)\}/)`), `pluralForm` for `en` / `de` and an unknown
//! code, and `foldTemplate`; plus what `<i18n-t>` shows when hidden (`defaultValue || ''`).
//!
//! Each case: a catalog for `en` (also the fallback) and `de`, root and page `p`, random
//! templates (placeholders, pipes, the `=0 ` marker, markup, blanks), numbers, booleans and
//! subtrees; an active locale; a scope (or none) with a locale and page; then texts. Each
//! text is checked by `to_string` (with its `missing` calls), `Display`, `write_to` into a
//! reused buffer, `== &str`, `resolve`, `parts`, `fold`, `Serialize` (feature `serde`),
//! then again after `set_locale` to the other locale: a stored `Text` follows it.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{I18n, Part, Text, scope, t, tc};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What the sessions reached: checks run, texts hidden, custom rules, keys standing in.
static SEEN: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

const KEYS: &[&str] = &["k0", "k1", "k2", "k3", "k4", "k5"];
/// Keys a text uses: the catalog's, a miss, the empty key, a key with markup and a hole.
const TEXT_KEYS: &[&str] = &["k0", "k1", "k2", "k3", "k4", "k5", "nope", "", "<x&'{a}\">"];

#[derive(Debug, Clone)]
enum Val {
    S(String),
    N(i64),
    B(bool),
    Tree,
}

/// `(locale, page)` → key → value; `nest` is a plain string in `en`'s root, maybe also in
/// `de` and the page `p` (nested texts read it in the outer text's locale and page, and
/// never miss).
type Cat = BTreeMap<(&'static str, Option<&'static str>), BTreeMap<&'static str, Val>>;

#[derive(Debug, Clone)]
enum Arg {
    S(String),
    /// Borrowed text: `Arg::Str`.
    Str(&'static str),
    I(i64),
    F(f64),
    B(bool),
    Null,
    /// `i18n.text("nest")`.
    Nested,
}

#[derive(Debug, Clone, Copy)]
enum Rule {
    Echo,
    Get,
    None,
}

#[derive(Debug, Clone, Copy)]
enum Form {
    Methods,
    /// `t!(i18n => key, …)` with the named args as macro arguments.
    Macro,
    /// `t!(key)` / `tc!(key, n)` on the scope's instance.
    Implicit,
}

#[derive(Debug, Clone)]
struct Spec {
    key: &'static str,
    args: Vec<(&'static str, Arg)>,
    list: Option<Vec<Arg>>,
    count: Option<f64>,
    default: Option<String>,
    html: bool,
    hide: bool,
    page: Option<&'static str>,
    locale: Option<&'static str>,
    rule: Option<Rule>,
    form: Form,
    /// `fold` slots.
    slots: Vec<&'static str>,
}

#[derive(Debug, Clone)]
struct Case {
    cat: Cat,
    active: &'static str,
    scope_locale: Option<&'static str>,
    scope_page: Option<&'static str>,
    texts: Vec<Spec>,
}

fn template() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("{a}"),
        Just("{b}"),
        Just("{count}"),
        Just("{0}"),
        Just("{1}"),
        Just("{length}"),
        Just("{link}"),
        Just(" | "),
        Just("|"),
        Just(" "),
        Just("\u{a0}"),
        Just("x"),
        Just("Hi"),
        Just("<b>"),
        Just("&"),
        Just("{"),
        Just("}"),
        Just("{ a }"),
    ];
    (any::<bool>(), prop::collection::vec(piece, 0..7)).prop_map(|(marked, p)| {
        let body: String = p.concat();
        if marked { format!("=0 {body}") } else { body }
    })
}

fn value() -> impl Strategy<Value = Val> {
    prop_oneof![
        6 => template().prop_map(Val::S),
        1 => (-5i64..50).prop_map(Val::N),
        1 => any::<bool>().prop_map(Val::B),
        1 => Just(Val::Tree),
    ]
}

fn chunk() -> impl Strategy<Value = BTreeMap<&'static str, Val>> {
    prop::collection::btree_map(prop::sample::select(KEYS), value(), 0..5)
}

const SAMPLES: &[&str] = &["Ann", "", "  ", "<i>", "a&'\"b", "{a}"];

fn arg() -> impl Strategy<Value = Arg> {
    prop_oneof![
        2 => prop::sample::select(SAMPLES).prop_map(|s| Arg::S(s.to_owned())),
        1 => prop::sample::select(SAMPLES).prop_map(Arg::Str),
        1 => (-3i64..30).prop_map(Arg::I),
        1 => Just(Arg::F(1.5)),
        1 => any::<bool>().prop_map(Arg::B),
        1 => Just(Arg::Null),
        1 => Just(Arg::Nested),
    ]
}

fn spec() -> impl Strategy<Value = Spec> {
    let names = prop::sample::select(vec!["a", "b", "count", "link", "0"]);
    (
        prop::sample::select(TEXT_KEYS),
        prop::collection::vec((names, arg()), 0..6),
        prop::option::of(prop::collection::vec(arg(), 0..3)),
        prop::option::of(prop::sample::select(vec![
            0.0, 1.0, 2.0, 3.0, 7.0, -1.0, 1.5,
        ])),
        prop::option::of(prop::sample::select(vec![
            "",
            "Def {a}",
            "  ",
            "<d>{count}",
        ])),
        (any::<bool>(), any::<bool>()),
        (
            prop::option::of(prop::sample::select(vec!["p", "q", "index"])),
            prop::option::of(prop::sample::select(vec!["en", "de", "xx"])),
        ),
        prop::option::weighted(
            0.15,
            prop::sample::select(vec![Rule::Echo, Rule::Get, Rule::None]),
        ),
        prop::sample::select(vec![Form::Methods, Form::Macro, Form::Implicit]),
        prop::collection::vec(prop::sample::select(vec!["a", "link", "count", "0"]), 0..3),
    )
        .prop_map(
            |(key, args, list, count, default, (html, hide), (page, locale), rule, form, slots)| {
                Spec {
                    key,
                    args,
                    list,
                    count,
                    default: default.map(str::to_owned),
                    html,
                    hide,
                    page,
                    locale,
                    rule,
                    form,
                    slots,
                }
            },
        )
}

fn case() -> impl Strategy<Value = Case> {
    (
        (chunk(), chunk(), chunk(), chunk()),
        (
            template(),
            prop::option::of(template()),
            prop::option::of(template()),
            prop::option::of(template()),
        ),
        prop::sample::select(vec!["en", "de"]),
        prop::option::of(prop::sample::select(vec!["en", "de", "xx"])),
        prop::option::of(prop::sample::select(vec!["p", "index"])),
        prop::collection::vec(spec(), 1..8),
    )
        .prop_map(
            |((er, ep, dr, dp), nests, active, scope_locale, scope_page, texts)| {
                let mut cat = Cat::new();
                let (mut er, mut ep, mut dr, mut dp) = (er, ep, dr, dp);
                er.insert("nest", Val::S(nests.0));
                for (chunk, nest) in [(&mut ep, nests.1), (&mut dr, nests.2), (&mut dp, nests.3)] {
                    if let Some(n) = nest {
                        chunk.insert("nest", Val::S(n));
                    }
                }
                cat.insert(("en", None), er);
                cat.insert(("en", Some("p")), ep);
                cat.insert(("de", None), dr);
                cat.insert(("de", Some("p")), dp);
                Case {
                    cat,
                    active,
                    scope_locale,
                    scope_page,
                    texts,
                }
            },
        )
}

// ---- the reference ----

/// JS `\s`.
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

/// `template.split(/\{([^{}\s]+)\}/)`: text at even indexes, names at odd ones.
fn tokenize(t: &str) -> Vec<String> {
    let chars: Vec<char> = t.chars().collect();
    let mut out = vec![String::new()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '{' && chars[j] != '}' && !js_space(chars[j]) {
                j += 1;
            }
            if j > i + 1 && j < chars.len() && chars[j] == '}' {
                out.push(chars[i + 1..j].iter().collect());
                out.push(String::new());
                i = j + 1;
                continue;
            }
        }
        out.last_mut().unwrap().push(chars[i]);
        i += 1;
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The params object `resolveText` interpolates with, name → `String(value)`.
type Params = BTreeMap<String, String>;

struct Model<'c> {
    cat: &'c Cat,
    active: &'static str,
    scope_locale: Option<&'static str>,
    scope_page: Option<&'static str>,
    misses: Vec<String>,
}

impl Model<'_> {
    fn chain(locale: &str) -> Vec<&'static str> {
        match locale {
            "en" => vec!["en"],
            "de" => vec!["de", "en"],
            _ => vec!["en"],
        }
    }

    fn lookup(&self, locale: &str, page: Option<&str>, key: &str) -> Option<Val> {
        let page = page.filter(|p| *p != "index");
        for l in Self::chain(locale) {
            let layers = [page.map(|p| (l, p)), Some((l, ""))];
            for (l, p) in layers.into_iter().flatten() {
                let p = (!p.is_empty()).then_some(p);
                let p = match p {
                    Some("p") => Some("p"),
                    Some(_) => continue,
                    None => None,
                };
                if let Some(v) = self.cat.get(&(l, p)).and_then(|c| c.get(key)) {
                    return Some(v.clone());
                }
            }
        }
        None
    }

    /// What a nested `i18n.text("nest")` shows: no locale or page of its own, so the outer
    /// text's (its own, else the scope's, else the active locale and the root).
    fn nested(&self, s: &Spec) -> String {
        let locale = s.locale.or(self.scope_locale).unwrap_or(self.active);
        let page = s.page.or(self.scope_page);
        match self.lookup(locale, page, "nest") {
            Some(Val::S(s)) => s,
            other => panic!("nest is a string: {other:?}"),
        }
    }

    fn arg(&self, a: &Arg, s: &Spec) -> String {
        let html = s.html;
        match a {
            Arg::S(t) if html => escape(t),
            Arg::S(t) => t.clone(),
            Arg::Str(t) if html => escape(t),
            Arg::Str(t) => (*t).to_owned(),
            Arg::I(n) => n.to_string(),
            Arg::F(_) => "1.5".to_owned(),
            Arg::B(b) => b.to_string(),
            Arg::Null => "null".to_owned(),
            Arg::Nested if html => escape(&self.nested(s)),
            Arg::Nested => self.nested(s),
        }
    }

    fn params(&self, s: &Spec) -> Params {
        let mut p = Params::new();
        if let Some(list) = &s.list {
            for (i, a) in list.iter().enumerate() {
                p.insert(i.to_string(), self.arg(a, s));
            }
            // a JS array's own `length`; `escapeParams` and a spread drop it
            if !s.html && s.count.is_none() {
                p.insert("length".to_owned(), list.len().to_string());
            }
        }
        for (name, a) in &s.args {
            p.insert((*name).to_owned(), self.arg(a, s));
        }
        if let Some(n) = s.count {
            p.insert("count".to_owned(), js_num(n));
        }
        p
    }

    fn key_text(s: &Spec) -> String {
        if s.html {
            escape(s.key)
        } else {
            s.key.to_owned()
        }
    }

    fn miss(&mut self, locale: &str, page: Option<&str>, key: &str) {
        self.misses
            .push(format!("{locale}|{}|{key}", page.unwrap_or("index")));
    }

    /// `resolveText`: the template (a literal one when a custom rule answered) and whether
    /// the key stands in for the text.
    fn template(&mut self, s: &Spec) -> (String, bool, bool) {
        let locale = s.locale.or(self.scope_locale).unwrap_or(self.active);
        let page = s.page.or(self.scope_page);
        let fallback = |_: &Self| match s.default.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => (d.to_owned(), false),
            None => (Self::key_text(s), !s.key.is_empty()),
        };
        if let (Some(n), Some(rule)) = (s.count, s.rule) {
            let answer = match rule {
                Rule::Echo => Some(format!("{}|{}|{locale}", s.key, js_num(n))),
                Rule::Get => Some(if s.key.is_empty() {
                    String::new()
                } else {
                    match self.lookup(locale, page, s.key) {
                        Some(Val::S(t)) => t,
                        Some(Val::N(n)) => n.to_string(),
                        Some(Val::B(b)) => b.to_string(),
                        Some(Val::Tree) => Self::key_text(s),
                        None => {
                            self.miss(locale, page, s.key);
                            fallback(self).0
                        }
                    }
                }),
                Rule::None => None,
            };
            return (answer.unwrap_or_default(), false, true);
        }
        if s.key.is_empty() {
            return (String::new(), false, false);
        }
        let found = self.lookup(locale, page, s.key);
        match s.count {
            None => match found {
                Some(Val::S(t)) => (t, false, false),
                Some(Val::N(n)) => (n.to_string(), false, true),
                Some(Val::B(b)) => (b.to_string(), false, true),
                Some(Val::Tree) => (Self::key_text(s), true, false),
                None => {
                    self.miss(locale, page, s.key);
                    let (t, keyed) = fallback(self);
                    (t, keyed, false)
                }
            },
            Some(n) => match found {
                Some(Val::S(t)) => (plural_form(&t, n, locale), false, false),
                Some(Val::N(v)) => (v.to_string(), false, true),
                Some(_) => {
                    let (t, keyed) = fallback(self);
                    (t, keyed, false)
                }
                None => {
                    self.miss(locale, page, s.key);
                    let (t, keyed) = fallback(self);
                    (t, keyed, false)
                }
            },
        }
    }

    /// `(shown, resolve(), parts, fold)`.
    fn run(&mut self, s: &Spec) -> (String, Option<String>, Vec<Part<'static>>, Vec<String>) {
        let (tpl, keyed, literal) = self.template(s);
        let params = if literal {
            Params::new()
        } else {
            self.params(s)
        };
        let tokens = tokenize(&tpl);
        let shown: String = if literal {
            tpl.clone()
        } else {
            interpolate(&tokens, &params)
        };
        let default = s.default.clone().filter(|d| !d.is_empty());
        SEEN[0].fetch_add(1, Ordering::Relaxed);
        if literal && s.rule.is_some() {
            SEEN[2].fetch_add(1, Ordering::Relaxed);
        }
        if keyed {
            SEEN[3].fetch_add(1, Ordering::Relaxed);
        }
        if s.hide && shown.chars().all(js_space) {
            SEEN[1].fetch_add(1, Ordering::Relaxed);
            let parts = default
                .iter()
                .map(|d| Part::Text(d.clone().into()))
                .collect();
            let fold = default.iter().cloned().collect();
            return (default.unwrap_or_default(), None, parts, fold);
        }
        let resolve = (!keyed).then(|| shown.clone());
        if literal {
            let parts = [tpl.clone()].into_iter().filter(|t| !t.is_empty());
            return (
                shown,
                resolve,
                parts.clone().map(|t| Part::Text(t.into())).collect(),
                parts.collect(),
            );
        }
        // `parts`: a placeholder without a param is a slot
        let mut parts = Vec::new();
        let mut text = tokens[0].clone();
        for pair in tokens[1..].chunks(2) {
            match params.get(&pair[0]) {
                Some(v) => text.push_str(v),
                None => {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text).into()));
                    }
                    parts.push(Part::Slot(pair[0].clone().into()));
                }
            }
            text.push_str(&pair[1]);
        }
        if !text.is_empty() {
            parts.push(Part::Text(text.into()));
        }
        // `foldTemplate`: a slot wins over a param, a param over the token as written
        let mut fold = Vec::new();
        let mut text = tokens[0].clone();
        for pair in tokens[1..].chunks(2) {
            if s.slots.contains(&pair[0].as_str()) {
                if !text.is_empty() {
                    fold.push(std::mem::take(&mut text));
                }
                fold.push(format!("<{}>", pair[0]));
            } else {
                match params.get(&pair[0]) {
                    Some(v) => text.push_str(v),
                    None => text.push_str(&format!("{{{}}}", pair[0])),
                }
            }
            text.push_str(&pair[1]);
        }
        if !text.is_empty() {
            fold.push(text);
        }
        (shown, resolve, parts, fold)
    }
}

fn interpolate(tokens: &[String], params: &Params) -> String {
    let mut out = tokens[0].clone();
    for pair in tokens[1..].chunks(2) {
        match params.get(&pair[0]) {
            Some(v) => out.push_str(v),
            None => out.push_str(&format!("{{{}}}", pair[0])),
        }
        out.push_str(&pair[1]);
    }
    out
}

/// JS `String(n)` for the counts drawn.
fn js_num(n: f64) -> String {
    format!("{n}")
}

/// `pluralForm` for `en` / `de` (`one` is 1, everything else `other`: two categories, so
/// an unmarked template always takes the index rule) and for a code `Intl` does not know
/// (the index rule, marked or not).
fn plural_form(t: &str, n: f64, locale: &str) -> String {
    let marked = t.starts_with("=0 ");
    let body = if marked { &t[3..] } else { t };
    let forms: Vec<&str> = body.split('|').collect();
    let last = forms.len() - 1;
    let i = if n.fract() != 0.0 || n < 0.0 {
        last
    } else {
        let n = n as usize;
        if marked && n == 0 {
            0
        } else if marked && locale != "xx" {
            if n == 1 { 1.min(last) } else { 2.min(last) }
        } else {
            n.min(last)
        }
    };
    forms[i].trim_matches(js_space).to_owned()
}

// ---- the crate ----

fn json_of(chunk: &BTreeMap<&'static str, Val>) -> String {
    let items: Vec<String> = chunk
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Val::S(s) => serde_json::to_string(s).unwrap(),
                Val::N(n) => n.to_string(),
                Val::B(b) => b.to_string(),
                Val::Tree => r#"{"x": "y"}"#.to_owned(),
            };
            format!("\"{k}\": {v}")
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

fn crate_arg(i18n: &I18n, a: &Arg) -> i18n_next::Arg<'static> {
    match a {
        Arg::S(s) => s.clone().into(),
        Arg::Str(s) => i18n_next::Arg::Str(s),
        Arg::I(n) => (*n).into(),
        Arg::F(v) => (*v).into(),
        Arg::B(b) => (*b).into(),
        Arg::Null => i18n_next::Arg::Null,
        Arg::Nested => i18n.text("nest").into(),
    }
}

fn build(i18n: &I18n, s: &Spec) -> Text {
    let arg = |a: &Arg| crate_arg(i18n, a);
    let c = s.count.map(count);
    // the macro form takes the first named arg (when it is an identifier) as a macro argument
    let first = match (s.form, s.args.first()) {
        (Form::Macro, Some((n @ ("a" | "b" | "link"), a))) => Some((*n, arg(a))),
        _ => None,
    };
    let rest = &s.args[usize::from(first.is_some())..];
    let mut t = match (s.form, first, c) {
        (Form::Implicit, _, Some(c)) => tc!(s.key, c),
        (Form::Implicit, _, None) => t!(s.key),
        (Form::Macro, Some(("a", v)), Some(c)) => t!(i18n => s.key, a = v, count = c),
        (Form::Macro, Some(("a", v)), None) => t!(i18n => s.key, a = v),
        (Form::Macro, Some(("b", v)), Some(c)) => t!(i18n => s.key, b = v, count = c),
        (Form::Macro, Some(("b", v)), None) => t!(i18n => s.key, b = v),
        (Form::Macro, Some((_, v)), Some(c)) => t!(i18n => s.key, link = v, count = c),
        (Form::Macro, Some((_, v)), None) => t!(i18n => s.key, link = v),
        (Form::Macro, None, Some(c)) => tc!(i18n => s.key, c),
        (Form::Macro, None, None) => t!(i18n => s.key),
        (Form::Methods, _, Some(c)) => i18n.text(s.key).count(c),
        (Form::Methods, _, None) => i18n.text(s.key),
    };
    if let Some(list) = &s.list {
        t = t.args(list.iter().map(arg));
    }
    for (name, a) in rest {
        t = t.arg(*name, arg(a));
    }
    if let Some(d) = &s.default {
        t = t.or(d.clone());
    }
    if s.html {
        t = t.html();
    }
    if s.hide {
        t = t.hide_if_empty();
    }
    if let Some(p) = s.page {
        t = t.page(p);
    }
    if let Some(l) = s.locale {
        t = t.locale(l);
    }
    match s.rule {
        Some(Rule::Echo) => t.plural_rule(|c| Some(format!("{}|{}|{}", c.key, c.count, c.locale))),
        Some(Rule::Get) => t.plural_rule(|c| Some(c.get(c.key))),
        Some(Rule::None) => t.plural_rule(|_| None),
        None => t,
    }
}

fn count(n: f64) -> i18n_next::Count {
    if n.fract() == 0.0 {
        i18n_next::Count::from(n as i64)
    } else {
        i18n_next::Count::from(n)
    }
}

fn check(
    i18n: &I18n,
    case: &Case,
    seen: &Mutex<Vec<String>>,
    text: &Text,
    s: &Spec,
    buf: &mut String,
) -> Result<(), TestCaseError> {
    let active: &'static str = if &*i18n.locale() == "de" { "de" } else { "en" };
    let mut model = Model {
        cat: &case.cat,
        active,
        scope_locale: case.scope_locale,
        scope_page: case.scope_page,
        misses: Vec::new(),
    };
    let (shown, resolved, parts, fold) = model.run(s);
    seen.lock().unwrap().clear();
    prop_assert_eq!(
        text.to_string(),
        shown.clone(),
        "to_string of {:?} in {}",
        s,
        active
    );
    prop_assert_eq!(
        seen.lock().unwrap().clone(),
        model.misses,
        "missing calls of {:?}",
        s
    );
    prop_assert_eq!(format!("{text}"), shown.clone());
    buf.clear();
    text.write_to(buf).unwrap();
    prop_assert_eq!(&*buf, &shown);
    prop_assert!(*text == shown.as_str());
    let longer = format!("{}~", shown);
    prop_assert!(*text != longer.as_str());
    prop_assert_eq!(text.resolve(), resolved);
    prop_assert_eq!(text.parts().collect::<Vec<_>>(), parts);
    let markers: Vec<(String, Box<dyn Fn() -> String>)> = s
        .slots
        .iter()
        .map(|n| {
            let m = format!("<{n}>");
            (
                (*n).to_owned(),
                Box::new(move || m.clone()) as Box<dyn Fn() -> String>,
            )
        })
        .collect();
    let slots: Vec<(&str, &dyn Fn() -> String)> =
        markers.iter().map(|(n, f)| (n.as_str(), &**f)).collect();
    prop_assert_eq!(text.fold(str::to_owned, &slots), fold);
    #[cfg(feature = "serde")]
    prop_assert_eq!(
        serde_json::to_string(text).unwrap(),
        serde_json::to_string(&shown).unwrap()
    );
    Ok(())
}

fn run(case: &Case) -> Result<(), TestCaseError> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let mut b = I18n::builder()
        .locale(case.active)
        .fallback("en")
        .missing(move |l, k, p| log.lock().unwrap().push(format!("{l}|{p}|{k}")));
    for ((locale, page), chunk) in &case.cat {
        b = b.messages(locale, *page, &json_of(chunk));
    }
    let i18n = b.build().unwrap();
    let other = if case.active == "en" { "de" } else { "en" };
    let mut sc = scope().instance(i18n.clone());
    if let Some(l) = case.scope_locale {
        sc = sc.locale(l);
    }
    if let Some(p) = case.scope_page {
        sc = sc.page(p);
    }
    let mut buf = String::new();
    sc.run(|| {
        for s in &case.texts {
            let text = build(&i18n, s);
            check(&i18n, case, &seen, &text, s, &mut buf)?;
            // a stored `Text` follows the active locale
            i18n.set_locale(other).unwrap();
            check(&i18n, case, &seen, &text, s, &mut buf)?;
            i18n.set_locale(case.active).unwrap();
        }
        Ok(())
    })
}

#[test]
fn text_matches_the_model() {
    let config = Config {
        cases: 512,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[44; 32]));
    runner.run(&case(), |c| run(&c)).unwrap();
    let seen: Vec<usize> = SEEN.iter().map(|n| n.load(Ordering::Relaxed)).collect();
    eprintln!("checks, hidden, custom rules, keyed: {seen:?}");
    assert!(
        seen[0] > 2000 && seen[1..].iter().all(|&n| n > 20),
        "{seen:?}"
    );
}
