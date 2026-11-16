//! The contract between the plugin and guest-js (DESIGN 6.2): a fixture of what the plugin
//! really serves (`meta`, protocol chunks, the raw `load` fallback) for a catalog with a
//! regional chain, pages, linked messages, `null`s, own dotted keys and leaf/object
//! conflicts, plus what Rust `t` answers in every view. guest-js
//! (`tests/guest-js/contract.test.ts`) replays it through `connect` with a real
//! `@i18n-micro/runtime` instance and must answer the same.
//! `UPDATE_GOLDEN=1` rewrites `tests/guest-js/fixtures/contract.json`.

mod common;

use common::{App, Value, json};
use i18n_next::I18n;
use std::collections::BTreeMap;

const LOCALES: [&str; 4] = ["en", "de", "de-at", "ar"];
const PAGES: [&str; 3] = ["index", "settings", "about"];

fn catalog() -> I18n {
    I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                {"code": "en", "iso": "en-US"}, {"code": "de", "iso": "de-DE"},
                {"code": "de-at", "iso": "de-AT", "fallbackLocale": "de"},
                {"code": "ar", "iso": "ar-EG", "dir": "rtl"}],
                "numberFormats": {"en": {"money": {"style": "currency", "currency": "USD"}}}}"#,
        )
        .messages(
            "en",
            None,
            r#"{"brand": "Acme", "hi": "Hi @:brand", "loud": "@.upper:brand!",
                "nav": {"home": "Home", "x": "X", "deep": {"a": "A"}}, "a.b": "own dotted",
                "conf": {"x": "conf.x en"}, "rev": "rev en", "n": 1.5, "t": true,
                "list": ["l0", {"k": "l1"}], "cart": "=0 empty | {count} item | {count} items",
                "greet": "Hello, {name}!", "only_en": "only en", "nul": "nul en"}"#,
        )
        .messages(
            "de",
            None,
            r#"{"brand": "AcmeDE", "nav": {"home": "Start"}, "conf": "conf de",
                "rev": {"y": "rev.y de"}, "a": {"b": "nested a.b de"}, "only_de": "nur de"}"#,
        )
        .messages(
            "de-at",
            None,
            r#"{"nav": {"x": "X-at"}, "nul": null, "at": "Servus @:nav.home"}"#,
        )
        .messages("ar", None, r#"{"brand": "أكمي", "nav": {"home": "الرئيسية"}}"#)
        .messages(
            "en",
            Some("settings"),
            r#"{"title": "Settings", "brand": "Acme settings", "s": {"x": "sx en"}, "back": "Back to @:nav.home"}"#,
        )
        .messages("de", Some("settings"), r#"{"title": "Einstellungen", "s": "s de"}"#)
        .messages("de-at", Some("settings"), r#"{"only_at": "nur at"}"#)
        .messages("ar", Some("settings"), r#"{"title": "الإعدادات"}"#)
        .messages("en", Some("about"), r#"{"title": "About"}"#)
        .linked_messages(true)
        .build()
        .unwrap()
}

/// Every leaf key of the catalog, plus misses and object paths.
const KEYS: &[&str] = &[
    "brand",
    "hi",
    "loud",
    "nav.home",
    "nav.x",
    "nav.deep.a",
    "a.b",
    "conf.x",
    "conf",
    "rev",
    "rev.y",
    "n",
    "t",
    "list.0",
    "list.1.k",
    "cart",
    "greet",
    "only_en",
    "only_de",
    "nul",
    "at",
    "title",
    "s.x",
    "s",
    "back",
    "only_at",
    "nope",
    "nav.nope",
];

fn view(i: &I18n, l: &str, p: &str) -> i18n_next::View {
    if p == "index" {
        i.with_locale(l)
    } else {
        i.with_locale(l).page(p)
    }
}

#[test]
fn guest_contract_fixture_is_up_to_date() {
    let i18n = catalog();
    let app = App::new(i18n.clone());
    let mut metas = BTreeMap::new();
    let mut loads = BTreeMap::new();
    let mut answers = BTreeMap::new();
    let mut chunks = BTreeMap::new();
    for l in LOCALES {
        app.ok("set_locale", json!({ "locale": l }));
        for p in PAGES {
            let id = format!("{l}|{p}");
            let mut meta = if p == "index" {
                app.meta(None, None)
            } else {
                app.meta(None, Some(p))
            };
            // the version moves with every switch: the replay sets its own
            meta["version"] = json!(0);
            for (key, tag) in meta["tags"].as_object().unwrap() {
                let path = format!("{}/{key}.json", tag.as_str().unwrap());
                let (status, body) = app.get(&format!("/{}", path.replace('/', "%2F")));
                assert_eq!(status, 200, "{path}");
                chunks.insert(path, serde_json::from_slice::<Value>(&body).unwrap());
            }
            let args = if p == "index" {
                json!({})
            } else {
                json!({ "page": p })
            };
            let mut load: Value =
                serde_json::from_slice(&app.call_raw("load", args).unwrap()).unwrap();
            load["version"] = json!(0);
            loads.insert(id.clone(), load);
            metas.insert(id.clone(), meta);
            let v = view(&i18n, l, p);
            let a: BTreeMap<&str, Option<String>> = KEYS.iter().map(|k| (*k, v.try_t(k))).collect();
            answers.insert(id, a);
        }
    }
    let fixture = json!({ "keys": KEYS, "metas": metas, "loads": loads, "chunks": chunks, "answers": answers });
    let text = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/guest-js/fixtures/contract.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == text,
        "guest-js contract fixture differs (UPDATE_GOLDEN=1 rewrites it)"
    );
}
