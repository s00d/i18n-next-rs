//! The front-end payload (feature `payload`, DESIGN 6.1 / 6.2) against the server it comes
//! from:
//!
//! - a Rust guest that applies the payloads of every view answers `t` / `tc` / `has` like
//!   the server, for random catalogs with linked messages over fallback chains and pages
//!   (the linked answers a view reads from weaker layers travel in `linked`);
//! - the front end's lookup (the JS core reads one locale: M8 premerges a view's chunks
//!   along `chains` from the meta, weakest first, then reads one flat object) answers
//!   like the server for `de-at → de → en`;
//! - what JSON cannot hold (`1e400`) is `null`, so the chunk still parses.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{I18n, payload};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const LOCALES: [&str; 4] = ["en", "de", "de-at", "tr"];
const PAGES: [Option<&str>; 2] = [None, Some("settings")];
const KEYS: [&str; 5] = ["a", "b", "c", "d.e", "n"];
/// Texts a key can hold: plain, links in every form, a plural, a number.
const TEXTS: [&str; 10] = [
    "plain {l}",
    "Hi @:b",
    "@.upper:c",
    "@:{'a'}!",
    "@:d.e and @.capitalize:b",
    "istanbul {l}",
    "one {l} | many {l}",
    "@.lower:n",
    "#3",
    "@:missing.key",
];

fn builder() -> i18n_next::Builder {
    I18n::builder()
        .fallback("en")
        .fallback_for("de-at", "de")
        .iso("tr", "tr-TR")
        .iso("de-at", "de-AT")
}

/// `(locale, page) → key → text index`.
type Catalog = BTreeMap<(usize, usize), BTreeMap<usize, usize>>;

fn catalog() -> impl Strategy<Value = Catalog> {
    let chunk = prop::collection::btree_map(0..KEYS.len(), 0..TEXTS.len(), 0..=KEYS.len());
    prop::collection::btree_map((0..LOCALES.len(), 0..PAGES.len()), chunk, 1..=8)
}

/// The JSON file of one chunk: `d.e` nested, `#3` the number 3.
fn file(l: usize, keys: &BTreeMap<usize, usize>) -> String {
    let mut root = Map::new();
    for (&k, &t) in keys {
        let text = TEXTS[t].replace("{l}", LOCALES[l]);
        let value = match text.strip_prefix('#') {
            Some(n) => json!(n.parse::<u32>().unwrap()),
            None => json!(text),
        };
        match KEYS[k].split_once('.') {
            Some((a, b)) => {
                root.insert(a.to_owned(), json!({ b: value }));
            }
            None => {
                root.insert(KEYS[k].to_owned(), value);
            }
        }
    }
    Value::Object(root).to_string()
}

fn server(c: &Catalog) -> I18n {
    let mut b = builder().linked_messages(true);
    for ((l, p), keys) in c {
        b = b.messages(LOCALES[*l], PAGES[*p], &file(*l, keys));
    }
    b.build().unwrap()
}

fn view(i: &I18n, l: &str, p: Option<&str>) -> i18n_next::View {
    match p {
        Some(p) => i.with_locale(l).page(p),
        None => i.with_locale(l),
    }
}

fn round_trip(c: &Catalog) -> Result<(), TestCaseError> {
    let server = server(c);
    // a guest built the way `tauri_guest::connect` builds one: from the server's meta
    let guest = payload::builder(&payload::meta(&server, None, None))
        .unwrap()
        .build()
        .unwrap();
    // every view's payload: chunks are per pair, so the guest ends with all of them
    for l in LOCALES {
        for p in PAGES {
            let (_, bytes) = payload::encode(&server, Some(l), p, false);
            payload::apply(&guest, &bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
    }
    for l in LOCALES {
        for p in PAGES {
            let (s, g) = (view(&server, l, p), view(&guest, l, p));
            for key in KEYS.iter().chain(&["d", "missing.key"]) {
                prop_assert_eq!(g.t(key), s.t(key), "t {} {:?} {}", l, p, key);
                prop_assert_eq!(g.tc(key, 2), s.tc(key, 2), "tc {} {:?} {}", l, p, key);
                prop_assert_eq!(g.has(key), s.has(key), "has {} {:?} {}", l, p, key);
            }
        }
    }
    Ok(())
}

/// Given random catalogs with linked messages, when a guest applies the payload of every
/// view, then it answers every lookup of every view like the server.
#[test]
fn a_guest_answers_like_the_server_with_linked_messages() {
    let config = Config {
        cases: 160,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[31; 32]));
    runner.run(&catalog(), |c| round_trip(&c)).unwrap();
}

/// `"chains":{…}` of a meta answer.
fn chains(meta: &[u8]) -> String {
    let text = std::str::from_utf8(meta).unwrap();
    let start = text.find("\"chains\":").unwrap();
    let end = start + text[start..].find('}').unwrap();
    text[start..=end].to_owned()
}

/// Given configs with and without a global fallback, links into the fallback and a fallback
/// with links of its own, when a guest is built from the meta, then it has the server's
/// locales, chains and start locale (a meta without a fallback says `null`).
#[test]
fn a_guest_built_from_the_meta_has_the_servers_chains() {
    let configs = [
        r#"{"locales": [{"code": "en"}, {"code": "de"}]}"#,
        r#"{"fallbackLocale": "en", "locales": [{"code": "x", "fallbackLocale": "de-at"},
            {"code": "de-at", "fallbackLocale": "de"}, {"code": "de"}]}"#,
        // `a` links to the fallback, which links on: `a → en → b`; `c` reads `c → en` only
        r#"{"fallbackLocale": "en", "locales": [{"code": "a", "fallbackLocale": "en"},
            {"code": "en", "fallbackLocale": "b"}, {"code": "b"}, {"code": "c"}]}"#,
        r#"{"defaultLocale": "de", "locales": [{"code": "de", "iso": "de-DE", "dir": "ltr"},
            {"code": "ar", "dir": "rtl", "displayName": "العربية"}]}"#,
    ];
    for config in configs {
        let server = I18n::builder().config_json(config).build().unwrap();
        let meta = payload::meta(&server, None, None);
        let guest = payload::builder(&meta).unwrap().build().unwrap();
        let again = payload::meta(&guest, None, None);
        assert_eq!(chains(&again), chains(&meta), "{config}");
        assert_eq!(guest.locales(), server.locales(), "{config}");
        assert_eq!(guest.locale(), server.locale(), "{config}");
    }
    let err = payload::builder(br#"{"version": 0, "locale": 1}"#).unwrap_err();
    assert_eq!(err.to_string(), "payload: locale must be a string");
}

/// Given a meta another plugin version wrote by hand, with `defaultLocale` and
/// `fallbackLocale` as `null` (no value, as JS reads them), then a guest builds from it and
/// starts in the meta's locale; `i18n.json` reads `null` the same way.
#[test]
fn a_meta_with_null_default_and_fallback_builds() {
    let meta = br#"{"version": 3, "locale": "de", "defaultLocale": null, "fallbackLocale": null,
        "locales": [{"code": "en"}, {"code": "de"}], "chains": {"en": ["en"], "de": ["de"]},
        "tags": {}}"#;
    let guest = payload::builder(meta).unwrap().build().unwrap();
    assert_eq!(&*guest.locale(), "de");
    let config = I18n::builder()
        .config_json(r#"{"defaultLocale": null, "locales": [{"code": "en"}]}"#)
        .build()
        .unwrap();
    assert_eq!(&*config.locale(), "en");
}

/// The repro of the audit: a link in the fallback resolves to the active locale's target
/// on the server, and the payload carries that answer.
#[test]
fn a_link_in_the_fallback_resolves_in_the_active_locale() {
    let server = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"brand": "Acme", "greet": "Hi @:brand", "city": "@.upper:c", "c": "istanbul"}"#,
        )
        .messages("de", None, r#"{"brand": "AcmeDE"}"#)
        .messages("tr", None, r#"{"x": "y"}"#)
        .iso("tr", "tr-TR")
        .linked_messages(true)
        .build()
        .unwrap();
    assert_eq!(server.t("greet"), "Hi AcmeDE");
    // step 1: the de root carries the answer for its view, apart from its data
    let (_, de) = payload::chunk(&server, "de", None);
    assert_eq!(
        &*de,
        r#"{"data":{"brand":"AcmeDE"},"linked":{"greet":"Hi AcmeDE"}}"#
    );
    // step 2: a guest reads it like the server, `has` does not see it
    let guest = I18n::builder().fallback("en").build().unwrap();
    payload::apply(&guest, &payload::encode(&server, None, None, false).1).unwrap();
    assert_eq!((guest.t("greet"), guest.has("greet")), ("Hi AcmeDE", false));
    // step 3: Turkish case rules in the tr view
    assert_eq!(server.with_locale("tr").t("city"), "İSTANBUL");
    payload::apply(&guest, &payload::encode(&server, Some("tr"), None, false).1).unwrap();
    assert_eq!(guest.with_locale("tr").t("city"), "İSTANBUL");
}

/// What the M8 front end does with a payload (DESIGN 6.2): per view, the chunks of the
/// locale's chain from `chains`, weakest first (root, then page, per locale), each `data`
/// flattened into one object over the weaker ones (`null` skipped), then its `linked`.
fn js_view(payload: &Value, locale: &str, page: Option<&str>) -> BTreeMap<String, String> {
    fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    let path = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    flatten(&path, v, out);
                }
            }
            Value::Null => {}
            Value::String(s) => {
                out.insert(prefix.to_owned(), s.clone());
            }
            other => {
                out.insert(prefix.to_owned(), other.to_string());
            }
        }
    }
    let chain: Vec<&str> = payload["chains"][locale]
        .as_array()
        .expect("chain of the locale in the meta")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    let chunks = payload["chunks"].as_array().unwrap();
    let mut out = BTreeMap::new();
    for c in chain.iter().rev() {
        for p in [Some("index"), page] {
            let Some(chunk) = chunks
                .iter()
                .find(|x| x["locale"] == *c && Some(x["page"].as_str().unwrap()) == p)
            else {
                continue;
            };
            flatten("", &chunk["data"], &mut out);
            if let Some(linked) = chunk.get("linked") {
                for (k, v) in linked.as_object().unwrap() {
                    flatten(k, v, &mut out);
                }
            }
        }
    }
    out
}

/// Given `de-at → de → en` with pages and a linked message, when the front end premerges
/// each view along the chain the meta lists, then every key reads what the server reads.
#[test]
fn the_front_end_premerges_the_chain_from_the_meta() {
    let server = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en",
                "locales": [{"code": "en"}, {"code": "de"}, {"code": "de-at", "fallbackLocale": "de"}]}"#,
        )
        .messages("en", None, r#"{"a": "A", "b": "B", "c": "C", "brand": "Acme", "hi": "Hi @:brand", "nav": {"x": "X"}}"#)
        .messages("de", None, r#"{"b": "B-de", "brand": "AcmeDE", "nav": {"x": "X-de"}}"#)
        .messages("de-at", None, r#"{"c": "C-at", "n": null}"#)
        .messages("en", Some("settings"), r#"{"a": "A-settings", "t": "T"}"#)
        .messages("de", Some("settings"), r#"{"t": "T-de"}"#)
        .linked_messages(true)
        .build()
        .unwrap();
    for page in [None, Some("settings")] {
        let (_, bytes) = payload::encode(&server, Some("de-at"), page, true);
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["chains"]["de-at"], json!(["de-at", "de", "en"]));
        let js = js_view(&payload, "de-at", page);
        let s = view(&server, "de-at", page);
        for key in ["a", "b", "c", "brand", "hi", "nav.x", "t", "n"] {
            let want = s.get(key).map(str::to_owned);
            assert_eq!(js.get(key), want.as_ref(), "{page:?} {key}");
        }
    }
}

/// Given a number JSON cannot hold, when its chunk is written, then it is `null` (as
/// `JSON.stringify` writes `Infinity`) and the whole chunk still parses.
#[test]
fn non_finite_numbers_are_null_in_the_payload() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"big": 1e400, "neg": -1e400, "ok": 2}"#)
        .build()
        .unwrap();
    let (_, bytes) = payload::encode(&i18n, None, None, false);
    let v: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(
        v["chunks"][0]["data"],
        json!({ "big": null, "neg": null, "ok": 2 })
    );
}
