//! Catalog model (DESIGN 7.3): random sessions of `add_messages`, `set_locale`, lookups
//! through every entry point (`t` / `t_with` / `try_t` / `has` / `get` / `page()` /
//! `with_locale()`) and bad names, checked after every step against a small model: JSON
//! trees in a `BTreeMap`, merged like `deepMergeTranslationsRecursive`, read like
//! `getByPath`, with the layer walk written out. Builder variants (global fallback, the
//! `de-at → de` link, start locale) come with each case.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{Args, Error, I18n};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const LOCALES: &[&str] = &["en", "de", "de-at", "ru", "es", "xx"];
const PAGES: &[Option<&str>] = &[
    None,
    Some("p"),
    Some("q"),
    Some("alias"),
    Some("index"),
    Some(""),
];
const KEYS: &[&str] = &[
    "a",
    "n",
    "n.a",
    "n.n",
    "n.x.y",
    "x.y",
    "x.y.a",
    "l",
    "l.0",
    "l.1",
    "l.0.a",
    "l.length",
    "a.n",
    "",
    ".a",
    "n.",
    "__proto__",
    "n.__proto__",
    "01",
    "l.01",
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

#[derive(Debug, Clone)]
enum Via {
    T,
    TWith,
    TryT,
    Has,
    Get,
    WithLocale(&'static str),
}

#[derive(Debug, Clone)]
enum Action {
    Add {
        locale: &'static str,
        page: Option<&'static str>,
        tree: Value,
    },
    SetLocale(&'static str),
    Lookup {
        via: Via,
        key: &'static str,
        page: Option<&'static str>,
    },
    /// A name from outside: 0 `set_locale`, 1 `page().t`, 2 `with_locale().t`, 3 `add_messages`.
    BadName {
        name: &'static str,
        kind: u8,
    },
}

#[derive(Debug, Clone)]
struct Case {
    fallback: Option<&'static str>,
    link: bool,
    start: &'static str,
    actions: Vec<Action>,
}

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        prop::sample::select(vec!["v1", "{x}!", "", "w {y}", "{x}{x}"]).prop_map(Value::from),
        (0i64..3).prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        Just(Value::Null),
    ]
}

fn key() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["a", "n", "x.y", "l", "__proto__", "", "0"]).prop_map(str::to_owned)
}

fn tree() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(3, 16, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            prop::collection::btree_map(key(), inner, 0..3)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn action() -> impl Strategy<Value = Action> {
    let via = prop_oneof![
        Just(Via::T),
        Just(Via::TWith),
        Just(Via::TryT),
        Just(Via::Has),
        Just(Via::Get),
        prop::sample::select(LOCALES).prop_map(Via::WithLocale),
    ];
    prop_oneof![
        3 => (prop::sample::select(LOCALES), prop::sample::select(PAGES),
              prop::collection::btree_map(key(), tree(), 0..4))
            .prop_map(|(locale, page, m)| Action::Add { locale, page, tree: Value::Object(m.into_iter().collect()) }),
        1 => prop::sample::select(LOCALES).prop_map(Action::SetLocale),
        6 => (via, prop::sample::select(KEYS), prop::sample::select(PAGES))
            .prop_map(|(via, key, page)| Action::Lookup { via, key, page }),
        1 => (prop::sample::select(BAD), 0u8..4).prop_map(|(name, kind)| Action::BadName { name, kind }),
    ]
}

fn case() -> impl Strategy<Value = Case> {
    (
        prop::sample::select(vec![None, Some("en"), Some("ru")]),
        any::<bool>(),
        prop::sample::select(vec!["en", "de-at"]),
        prop::collection::vec(action(), 1..40),
    )
        .prop_map(|(fallback, link, start, actions)| Case {
            fallback,
            link,
            start,
            actions,
        })
}

/// The reference: trees per `(locale, page)`, the walk written out.
struct Model {
    chunks: BTreeMap<(String, String), Map<String, Value>>,
    locale: String,
    version: u64,
    fallback: Option<&'static str>,
    link: bool,
    start: &'static str,
}

/// Root of the page chunk a name reads (`""` / `index` / none → root, alias → `p`).
fn chunk_page(page: Option<&str>) -> String {
    match page {
        None | Some("" | "index") => "index".to_owned(),
        Some("alias") => "p".to_owned(),
        Some(p) => p.to_owned(),
    }
}

/// `deepMergeTranslationsRecursive`.
fn merge(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (k, v) in source {
        if k == "__proto__" {
            continue;
        }
        match v {
            Value::Object(src) => {
                let mut dst = match target.remove(k) {
                    Some(Value::Object(d)) => d,
                    _ => Map::new(),
                };
                merge(&mut dst, src);
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

/// What a lookup finds: text, a subtree, or nothing.
#[derive(Debug, PartialEq)]
enum Hit {
    Text(String),
    Node,
}

fn hit(v: &Value) -> Hit {
    match v {
        Value::String(s) => Hit::Text(s.clone()),
        Value::Number(n) => Hit::Text(n.to_string()),
        Value::Bool(b) => Hit::Text(b.to_string()),
        _ => Hit::Node,
    }
}

impl Model {
    fn configured(code: &str) -> bool {
        code == "de-at" || code == "es"
    }

    fn known(&self, code: &str) -> bool {
        Self::configured(code) || code == self.start || self.chunks.keys().any(|(l, _)| l == code)
    }

    fn enabled(&self, code: &str) -> bool {
        code == self.locale || (self.known(code) && code != "es")
    }

    fn chain(&self, code: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut follow = |start: &str| {
            let mut c = Some(start.to_owned());
            while let Some(x) = c {
                if out.contains(&x) {
                    break;
                }
                c = (self.link && x == "de-at").then(|| "de".to_owned());
                out.push(x);
            }
        };
        follow(code);
        if let Some(fb) = self.fallback {
            follow(fb);
        }
        out
    }

    fn layers(&self, view: Option<&str>) -> Vec<String> {
        match view {
            None => self.chain(&self.locale),
            Some(code) if self.enabled(code) => self.chain(code),
            Some(_) => self.fallback.map(|fb| self.chain(fb)).unwrap_or_default(),
        }
    }

    fn in_locale(&self, code: &str, page: &str, key: &str) -> Option<Hit> {
        let read = |p: &str| {
            self.chunks
                .get(&(code.to_owned(), p.to_owned()))
                .and_then(|t| get_by_path(t, key))
                .map(hit)
        };
        if page != "index"
            && let Some(h) = read(page)
        {
            return Some(h);
        }
        read("index")
    }

    fn lookup(&self, view: Option<&str>, page: Option<&str>, key: &str) -> Option<Hit> {
        let page = chunk_page(page);
        self.layers(view)
            .iter()
            .find_map(|code| self.in_locale(code, &page, key))
    }

    fn has(&self, view: Option<&str>, page: Option<&str>, key: &str) -> bool {
        let code = view.unwrap_or(&self.locale);
        self.enabled(code) && self.in_locale(code, &chunk_page(page), key).is_some()
    }

    fn add(&mut self, locale: &str, page: Option<&str>, tree: &Value) {
        let Value::Object(src) = tree else {
            unreachable!()
        };
        let chunk = self
            .chunks
            .entry((locale.to_owned(), chunk_page(page)))
            .or_default();
        merge(chunk, src);
        self.version += 1;
    }

    /// `t` / `t_with`: the text, and the `missing` record it causes.
    fn t(
        &self,
        view: Option<&str>,
        page: Option<&str>,
        key: &str,
        with: bool,
    ) -> (String, Option<String>) {
        if key.is_empty() {
            return (String::new(), None);
        }
        match self.lookup(view, page, key) {
            Some(Hit::Text(s)) if with => (s.replace("{x}", "X"), None),
            Some(Hit::Text(s)) => (s, None),
            Some(Hit::Node) => (key.to_owned(), None),
            None => {
                let code = view.unwrap_or(&self.locale);
                let p = page.filter(|p| !p.is_empty()).unwrap_or("index");
                (key.to_owned(), Some(format!("{code}|{p}|{key}")))
            }
        }
    }
}

fn run(case: &Case) -> Result<(), TestCaseError> {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let link = if case.link {
        r#", "fallbackLocale": "de""#
    } else {
        ""
    };
    let config = format!(
        r#"{{"locales": [{{"code": "de-at"{link}}}, {{"code": "es", "disabled": true}}]}}"#
    );
    let mut b = I18n::builder()
        .config_json(&config)
        .locale(case.start)
        .messages("en", Some("p"), "{}")
        .page_alias("alias", "p")
        .missing(move |l, k, p| log.lock().unwrap().push(format!("{l}|{p}|{k}")));
    if let Some(fb) = case.fallback {
        b = b.fallback(fb);
    }
    let i18n = b.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
    let mut m = Model {
        chunks: BTreeMap::from([(("en".to_owned(), "p".to_owned()), Map::new())]),
        locale: case.start.to_owned(),
        version: 0,
        fallback: case.fallback,
        link: case.link,
        start: case.start,
    };
    let kept = i18n.get("a");
    let kept_text = kept.as_deref().map(str::to_owned);
    let mut versions = vec![i18n.version()];
    let args = Args::new().str("x", "X");

    for (step, action) in case.actions.iter().enumerate() {
        let mut expected_missing: Vec<String> = Vec::new();
        match action {
            Action::Add { locale, page, tree } => {
                i18n.add_messages(locale, *page, &tree.to_string())
                    .map_err(|e| TestCaseError::fail(format!("step {step}: {e}")))?;
                m.add(locale, *page, tree);
            }
            Action::SetLocale(code) => {
                let got = i18n.set_locale(code);
                let want = if *code == "es" {
                    Err("disabled")
                } else if !m.known(code) {
                    Err("unknown")
                } else {
                    Ok(())
                };
                match (&got, want) {
                    (Ok(()), Ok(())) => {
                        if m.locale != *code {
                            m.locale = (*code).to_owned();
                            m.version += 1;
                        }
                    }
                    (Err(Error::DisabledLocale(_)), Err("disabled"))
                    | (Err(Error::UnknownLocale(_)), Err("unknown")) => {}
                    _ => prop_assert!(
                        false,
                        "step {step}: set_locale({code}) = {got:?}, want {want:?}"
                    ),
                }
            }
            Action::Lookup { via, key, page } => {
                let view = match (via, page) {
                    (Via::WithLocale(code), Some(p)) => Some(i18n.with_locale(code).page(p)),
                    (Via::WithLocale(code), None) => Some(i18n.with_locale(code)),
                    (_, Some(p)) => Some(i18n.page(p)),
                    (_, None) => None,
                };
                let vloc = match via {
                    Via::WithLocale(code) => Some(*code),
                    _ => None,
                };
                match via {
                    Via::T | Via::WithLocale(_) | Via::TWith => {
                        let with = matches!(via, Via::TWith);
                        let got = match (&view, with) {
                            (Some(v), false) => v.t(key),
                            (Some(v), true) => v.t_with(key, &args),
                            (None, false) => i18n.t(key),
                            (None, true) => i18n.t_with(key, &args),
                        };
                        let (want, miss) = m.t(vloc, *page, key, with);
                        prop_assert_eq!(got, want, "step {}: {:?}", step, action);
                        expected_missing.extend(miss);
                    }
                    Via::TryT | Via::Get => {
                        let got = match (&view, via) {
                            (Some(v), Via::TryT) => v.try_t(key),
                            (Some(v), _) => v.get(key).map(|s| s.to_string()),
                            (None, Via::TryT) => i18n.try_t(key),
                            (None, _) => i18n.get(key).map(|s| s.to_string()),
                        };
                        let want = match m.lookup(vloc, *page, key) {
                            Some(Hit::Text(s)) => Some(s),
                            _ => None,
                        };
                        prop_assert_eq!(got, want, "step {}: {:?}", step, action);
                    }
                    Via::Has => {
                        let got = view.as_ref().map_or_else(|| i18n.has(key), |v| v.has(key));
                        prop_assert_eq!(
                            got,
                            m.has(vloc, *page, key),
                            "step {}: {:?}",
                            step,
                            action
                        );
                    }
                }
            }
            Action::BadName { name, kind } => match kind {
                0 => {
                    // A bad name is only ever a name: once `add_messages` made it a locale
                    // it switches like any other, never touching a path.
                    let known = m.known(name);
                    let got = i18n.set_locale(name);
                    if known {
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
                    let (want, miss) = m.t(None, Some(name), "a", false);
                    prop_assert_eq!(i18n.page(name).t("a"), want, "step {}", step);
                    expected_missing.extend(miss);
                }
                2 => {
                    let (want, miss) = m.t(Some(name), None, "a", false);
                    prop_assert_eq!(i18n.with_locale(name).t("a"), want, "step {}", step);
                    prop_assert!(!i18n.with_locale(name).has("a") || m.has(Some(name), None, "a"));
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
                        m.add(name, None, &serde_json::json!({"a": "bad"}));
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
        prop_assert_eq!(i18n.locale(), m.locale.clone());
        versions.push(i18n.version());
    }
    // A value handed out before the session is unchanged; versions only grew.
    prop_assert_eq!(kept.as_deref().map(str::to_owned), kept_text);
    prop_assert!(versions.windows(2).all(|w| w[0] <= w[1]));
    Ok(())
}

#[test]
fn catalog_matches_the_model() {
    let config = Config {
        cases: 512,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[11; 32]));
    runner.run(&case(), |c| run(&c)).unwrap();
}
