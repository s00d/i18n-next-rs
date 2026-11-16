//! `tauri_guest` against a fake `__TAURI_INTERNALS__` (DESIGN 6.2, 7.6), the Rust twin of
//! guest-js's contract, random and connect tests: wasm-bindgen-test in Node and headless
//! Chrome. The fake (`support/tauri_fake.js`) serves either the payloads a live `I18n` encodes
//! with `payload` (`meta`, protocol chunks, raw `load`) or the fixtures the plugin recorded
//! for guest-js (`crates/tauri-plugin-i18n-next/tests/guest-js/fixtures`: `contract.json`
//! from `tests/guest_contract.rs`, `random.json` from `tests/guest_random.rs`), and the
//! guest must answer what Rust answered there, over the protocol and over raw invoke.
//!
//! Every test connects and disconnects: one guest per WebView, and the tests share one.
//! Events are delivered by the test; a queued call (`set_page` of the shown page, which
//! costs no call when nothing changed) waits for them. No timers.
#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "tauri-guest"))]

use i18n_next::{Error, I18n, payload, tauri_guest};
use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

const CONTRACT: &str =
    include_str!("../../tauri-plugin-i18n-next/tests/guest-js/fixtures/contract.json");
const RANDOM: &str =
    include_str!("../../tauri-plugin-i18n-next/tests/guest-js/fixtures/random.json");

thread_local! {
    static LIB: JsValue = Function::new_with_args("", include_str!("support/tauri_fake.js"))
        .call0(&JsValue::NULL)
        .unwrap();
}

fn lib(name: &str) -> JsValue {
    LIB.with(|l| get(l, name))
}

fn get(o: &JsValue, key: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(key)).unwrap()
}

fn put(o: &JsValue, key: &str, v: &JsValue) {
    Reflect::set(o, &JsValue::from_str(key), v).unwrap();
}

fn call(o: &JsValue, name: &str, args: &[JsValue]) -> JsValue {
    let f: Function = get(o, name).dyn_into().unwrap();
    Reflect::apply(&f, o, &args.iter().collect::<Array>()).unwrap()
}

fn new(class: &str, args: &[JsValue]) -> JsValue {
    let ctor: Function = lib(class).dyn_into().unwrap();
    Reflect::construct(&ctor, &args.iter().collect::<Array>()).unwrap()
}

/// The fake plugin, installed while it lives.
struct Fake(JsValue);

impl Fake {
    fn new(source: &JsValue, locale: &str) -> Fake {
        let fake = Fake(new("Fake", &[source.clone(), JsValue::from_str(locale)]));
        call(&fake.0, "install", &[]);
        fake
    }

    fn set(&self, key: &str, v: impl Into<JsValue>) {
        put(&self.0, key, &v.into());
    }

    fn count(&self, suffix: &str) -> usize {
        call(&self.0, "count", &[JsValue::from_str(suffix)])
            .as_f64()
            .unwrap() as usize
    }

    fn fetches(&self) -> usize {
        Array::from(&get(&self.0, "fetches")).length() as usize
    }

    fn bump(&self) {
        call(&self.0, "bump", &[]);
    }

    fn switch(&self, locale: &str) {
        call(&self.0, "switch", &[JsValue::from_str(locale)]);
    }

    fn deliver(&self, index: u32) {
        call(&self.0, "deliver", &[JsValue::from(index)]);
    }

    /// Runs `f` once, right after the next `meta` answer is built.
    fn after_meta(&self, f: impl FnOnce() + 'static) {
        self.set("afterMeta", Closure::once_into_js(f));
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        tauri_guest::disconnect();
        call(&self.0, "uninstall", &[]);
    }
}

/// A plugin over a live catalog: what `payload` encodes, as the plugin's commands and
/// protocol send it (an unknown page is refused, a stale chunk tag answers 410).
fn rust_source(server: &Rc<RefCell<I18n>>) -> JsValue {
    let f = Object::new();
    let hex = |t: u64| format!("{t:016x}");
    let s = server.clone();
    let meta = Closure::<dyn Fn(String, JsValue) -> JsValue>::new(move |l: String, p: JsValue| {
        let s = s.borrow();
        let page = p.as_string();
        if page
            .as_deref()
            .is_some_and(|p| p != "index" && !s.has_page(p))
        {
            return JsValue::UNDEFINED;
        }
        let bytes = payload::meta(&s, Some(&l), page.as_deref());
        JsValue::from_str(std::str::from_utf8(&bytes).unwrap())
    });
    let s = server.clone();
    let chunk = Closure::<dyn Fn(String) -> JsValue>::new(move |path: String| {
        let (tag, rest) = path.split_at(16);
        let (l, p) = rest[1..rest.len() - 5].split_once('/').unwrap();
        let (now, json) = payload::chunk(&s.borrow(), l, Some(p));
        if hex(now) == tag {
            JsValue::from_str(&json)
        } else {
            JsValue::UNDEFINED
        }
    });
    let s = server.clone();
    let tag = Closure::<dyn Fn(String) -> String>::new(move |key: String| {
        let (l, p) = key.split_once('/').unwrap();
        hex(payload::chunk(&s.borrow(), l, Some(p)).0)
    });
    let s = server.clone();
    let load = Closure::<dyn Fn(String, JsValue) -> String>::new(move |l: String, p: JsValue| {
        let page = p.as_string().filter(|p| p != "index");
        let (_, bytes) = payload::encode(&s.borrow(), Some(&l), page.as_deref(), false);
        String::from_utf8(bytes).unwrap()
    });
    let s = server.clone();
    let locales = Closure::<dyn Fn() -> String>::new(move || {
        let codes: Vec<String> = s
            .borrow()
            .locales()
            .into_iter()
            .map(|l| format!("{:?}", l.code))
            .collect();
        format!("[{}]", codes.join(","))
    });
    put(&f, "meta", &meta.into_js_value());
    put(&f, "chunk", &chunk.into_js_value());
    put(&f, "tag", &tag.into_js_value());
    put(&f, "load", &load.into_js_value());
    put(&f, "locales", &locales.into_js_value());
    call(&LIB.with(Clone::clone), "rustSource", &[f.into()])
}

fn server() -> Rc<RefCell<I18n>> {
    Rc::new(RefCell::new(
        I18n::builder()
            .config_json(
                r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                    {"code": "en", "iso": "en-US"}, {"code": "de", "iso": "de-DE"},
                    {"code": "de-at", "iso": "de-AT", "fallbackLocale": "de"}]}"#,
            )
            .messages(
                "en",
                None,
                r#"{"title": "App", "nav": {"home": "Home"}, "apples": "no apples | one apple | {count} apples"}"#,
            )
            .messages("de", None, r#"{"title": "Anwendung"}"#)
            .messages("de-at", None, r#"{"nav": {"home": "Daham"}}"#)
            .messages("en", Some("p1"), r#"{"h": "One"}"#)
            .messages("en", Some("p2"), r#"{"h": "Two"}"#)
            .messages("de", Some("p1"), r#"{"h": "Eins"}"#)
            .build()
            .unwrap(),
    ))
}

/// Lets queued microtasks run (a batch of misses goes out in one).
async fn microtasks() {
    for _ in 0..5 {
        JsFuture::from(Promise::resolve(&JsValue::NULL))
            .await
            .unwrap();
    }
}

/// What a guest view reads, as the fixtures record it: `null` for a miss or a subtree.
fn answer(i18n: &I18n, page: &str, key: &str) -> JsValue {
    let got = if page == "index" {
        i18n.try_t(key)
    } else {
        i18n.page(page).try_t(key)
    };
    got.map_or(JsValue::NULL, |s| JsValue::from_str(&s))
}

/// Given the contract the plugin recorded for guest-js (a regional chain, pages, linked
/// messages, `null`s, own dotted keys, leaf/object conflicts, formats in the catalog's zone),
/// when the guest walks every locale and page over the protocol and then over raw invoke,
/// then every key reads what Rust `try_t` read and dates show in `Asia/Tokyo`.
#[wasm_bindgen_test]
async fn contract_fixture_reads_as_rust_over_both_transports() {
    let fx = JSON::parse(CONTRACT).unwrap();
    let keys = Array::from(&get(&fx, "keys"));
    let answers = get(&fx, "answers");
    let at = get(&fx, "at").as_f64().unwrap() as i64;
    let source = call(
        &LIB.with(Clone::clone),
        "contractSource",
        std::slice::from_ref(&fx),
    );
    for protocol in [true, false] {
        let fake = Fake::new(&source, "en");
        fake.set("protocol", protocol);
        let i18n = tauri_guest::connect("index").await.unwrap();
        let mut wrong = Vec::new();
        for l in ["en", "de", "de-at", "ar"] {
            tauri_guest::set_locale(l).await.unwrap();
            for p in ["index", "settings", "about"] {
                tauri_guest::set_page(p).await.unwrap();
                let want = get(&answers, &format!("{l}|{p}"));
                for k in keys.iter() {
                    let k = k.as_string().unwrap();
                    let (got, want) = (answer(&i18n, p, &k), get(&want, &k));
                    if got != want {
                        wrong.push(format!("{l}|{p} {k}: {got:?} != {want:?}"));
                    }
                }
            }
            let dates = Array::from(&get(&get(&fx, "dates"), l));
            assert_eq!(
                (i18n.td(at), i18n.td_as(at, "stamp")),
                (
                    dates.get(0).as_string().unwrap(),
                    dates.get(1).as_string().unwrap()
                ),
                "{l}"
            );
        }
        assert!(wrong.is_empty(), "protocol={protocol}: {wrong:#?}");
        // invoke: the one fetch that failed, then never again
        assert_eq!(fake.fetches() == 1, !protocol, "{}", fake.fetches());
        assert_eq!(fake.count("|load") == 0, protocol);
    }
}

/// Given 200 random catalogs over `x → de-at → de → en` (pages, `null`s, conflicts, arrays,
/// linked messages, plurals) as the plugin served them to guest-js, when the guest walks every
/// view of each over the protocol and over raw invoke, then every key reads what Rust read.
#[wasm_bindgen_test]
async fn random_fixture_reads_as_rust_over_both_transports() {
    let fx = JSON::parse(RANDOM).unwrap();
    let keys: Vec<String> = Array::from(&get(&fx, "keys"))
        .iter()
        .map(|k| k.as_string().unwrap())
        .collect();
    let base = get(&fx, "base");
    let catalogs = Array::from(&get(&fx, "catalogs"));
    assert_eq!(catalogs.length(), 200);
    let lib = LIB.with(Clone::clone);
    for protocol in [true, false] {
        let mut wrong = Vec::new();
        for (n, c) in catalogs.iter().enumerate() {
            let fake = Fake::new(
                &call(&lib, "randomSource", &[base.clone(), c.clone()]),
                "en",
            );
            fake.set("protocol", protocol);
            let i18n = tauri_guest::connect("index").await.unwrap();
            let answers = get(&c, "answers");
            for l in ["x", "de-at", "de", "en"] {
                tauri_guest::set_locale(l).await.unwrap();
                for p in ["index", "p1", "p2"] {
                    tauri_guest::set_page(p).await.unwrap();
                    let want = get(&answers, &format!("{l}|{p}"));
                    for k in &keys {
                        let got = answer(&i18n, p, k);
                        let want = Some(get(&want, k))
                            .filter(|w| !w.is_undefined())
                            .unwrap_or(JsValue::NULL);
                        if got != want {
                            wrong.push(format!("#{n} {l}|{p} {k}: {got:?} != {want:?}"));
                        }
                    }
                }
            }
            drop(fake);
        }
        assert!(wrong.is_empty(), "protocol={protocol}: {wrong:#?}");
    }
}

/// Given a live catalog behind the plugin, when the guest connects, then it asks `meta` with
/// the WebView's languages and zone, builds its instance from it (locales, chains, `iso`) and
/// reads every chunk of the chain once; switching `en → de → en` reads `en` from the tag
/// cache, and the plugin's locale follows `set_locale`.
#[wasm_bindgen_test]
async fn connect_builds_from_meta_and_caches_chunks_by_tag() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("p1").await.unwrap();
    let args = get(&fake.0, "lastMetaArgs");
    assert!(Array::is_array(&get(&args, "languages")));
    assert!(get(&args, "timeZone").as_string().is_some());
    assert_eq!(
        (
            i18n.page("p1").t("h"),
            i18n.t("title"),
            i18n.tc("apples", 3)
        ),
        ("One".into(), "App".into(), "3 apples".into())
    );
    assert_eq!(fake.fetches(), 2); // en/index, en/p1
    assert_eq!(
        i18n.locales()
            .iter()
            .map(|l| (l.code.as_str(), l.iso.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("en", Some("en-US")),
            ("de", Some("de-DE")),
            ("de-at", Some("de-AT"))
        ]
    );
    tauri_guest::set_locale("de-at").await.unwrap();
    assert_eq!(get(&fake.0, "locale").as_string().unwrap(), "de-at");
    // de-at → de → en: own, then de, then en
    assert_eq!(
        (i18n.t("nav.home"), i18n.t("title"), i18n.page("p1").t("h")),
        ("Daham".into(), "Anwendung".into(), "Eins".into())
    );
    assert_eq!(fake.fetches(), 5); // + de-at/index, de/index, de/p1
    tauri_guest::set_locale("en").await.unwrap();
    assert_eq!(
        (&*i18n.locale(), i18n.page("p1").t("h"), fake.fetches()),
        ("en", "One".into(), 5)
    );
    // a page checked at the version shown: no call at all
    let calls = Array::from(&get(&fake.0, "ipc")).length();
    tauri_guest::set_page("p1").await.unwrap();
    assert_eq!(Array::from(&get(&fake.0, "ipc")).length(), calls);
    // an unknown locale: the plugin refuses, nothing changes
    assert!(matches!(
        tauri_guest::set_locale("xx").await,
        Err(Error::UnknownLocale(code)) if code == "xx"
    ));
    assert_eq!(&*i18n.locale(), "en");
}

/// Given a connected guest, when other windows switch the locale and the events arrive out of
/// order, then the newer one is shown and the late one is dropped without a call; two events
/// that arrive before the first is handled cost one read.
#[wasm_bindgen_test]
async fn changed_events_apply_in_version_order() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    // step 1: another window switches to de: the event brings it
    fake.switch("de");
    fake.deliver(0);
    tauri_guest::set_page("index").await.unwrap();
    assert_eq!(
        (&*i18n.locale(), i18n.t("title")),
        ("de", "Anwendung".into())
    );
    // step 2: en, then de-at; de-at's event comes first, en's late
    fake.switch("en");
    fake.switch("de-at");
    let metas = fake.count("|meta");
    fake.deliver(1);
    tauri_guest::set_page("index").await.unwrap();
    assert_eq!((&*i18n.locale(), fake.count("|meta")), ("de-at", metas + 1));
    fake.deliver(0);
    tauri_guest::set_page("index").await.unwrap();
    assert_eq!((&*i18n.locale(), fake.count("|meta")), ("de-at", metas + 1));
    // step 3: a catalog change twice, both events in before the first is handled: one meta
    server
        .borrow()
        .add_messages("de-at", None, r#"{"title": "Servus"}"#)
        .unwrap();
    fake.bump();
    fake.bump();
    fake.deliver(0);
    fake.deliver(0);
    tauri_guest::set_page("index").await.unwrap();
    assert_eq!(
        (i18n.t("title"), fake.count("|meta")),
        ("Servus".into(), metas + 2)
    );
}

/// Given a chunk that changes between `meta` and its read, then the stale URL answers 410
/// and a new `meta` reads the newest; a catalog that changes under every read fails the call,
/// says why and keeps what is shown; over raw invoke a stale `load` is read again.
#[wasm_bindgen_test]
async fn gone_chunks_read_meta_again() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    // step 1: p2 changes, and again under the read
    server
        .borrow()
        .add_messages("en", Some("p2"), r#"{"h": "Two!"}"#)
        .unwrap();
    fake.bump();
    let s = server.clone();
    let f = fake.0.clone();
    fake.after_meta(move || {
        s.borrow()
            .add_messages("en", Some("p2"), r#"{"h": "Two!!"}"#)
            .unwrap();
        call(&f, "bump", &[]);
    });
    let fetches = fake.fetches();
    tauri_guest::set_page("p2").await.unwrap();
    assert_eq!(
        (i18n.page("p2").t("h"), fake.fetches() - fetches),
        ("Two!!".into(), 2)
    );
    // step 2: the catalog changes under every read
    fn edit(server: Rc<RefCell<I18n>>, fake: JsValue, n: u32) {
        server
            .borrow()
            .add_messages("en", Some("p1"), &format!(r#"{{"h": "One {n}"}}"#))
            .unwrap();
        call(&fake, "bump", &[]);
        let again = fake.clone();
        let next = Closure::once_into_js(move || edit(server, again, n + 1));
        put(&fake, "afterMeta", &next);
    }
    edit(server.clone(), fake.0.clone(), 0);
    let err = tauri_guest::set_page("p1").await.unwrap_err();
    assert!(err.to_string().contains("kept changing"), "{err}");
    fake.set("afterMeta", JsValue::UNDEFINED);
    assert_eq!(i18n.page("p2").t("h"), "Two!!");
    tauri_guest::set_page("p1").await.unwrap();
    assert!(i18n.page("p1").t("h").starts_with("One "));
    drop(fake);
    // step 3: raw invoke: a load landing after a newer change is read again
    let fake = Fake::new(&rust_source(&server), "en");
    fake.set("protocol", false);
    let i18n = tauri_guest::connect("index").await.unwrap();
    server
        .borrow()
        .add_messages("en", None, r#"{"title": "B"}"#)
        .unwrap();
    fake.bump();
    let s = server.clone();
    let f = fake.0.clone();
    fake.after_meta(move || {
        s.borrow()
            .add_messages("en", None, r#"{"title": "C"}"#)
            .unwrap();
        call(&f, "bump", &[]);
    });
    tauri_guest::refresh().await.unwrap();
    assert_eq!(i18n.t("title"), "C");
}

/// Given the protocol, when it answers an HTTP error, then the read fails and the protocol
/// stays; when `fetch` rejects (no scheme in this WebView), then raw invoke `load` carries the
/// chunks from then on and no fetch is tried again.
#[wasm_bindgen_test]
async fn falls_back_to_invoke_only_when_fetch_rejects() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    fake.set("status", 500);
    let err = tauri_guest::set_page("p1").await.unwrap_err();
    assert!(
        matches!(&err, Error::Fetch { message, .. } if message == "HTTP 500"),
        "{err}"
    );
    fake.set("status", JsValue::UNDEFINED);
    tauri_guest::set_page("p1").await.unwrap();
    assert_eq!(
        (i18n.page("p1").t("h"), fake.count("|load")),
        ("One".into(), 0)
    );
    fake.set("protocol", false);
    tauri_guest::set_page("p2").await.unwrap();
    let fetches = fake.fetches();
    assert_eq!(
        (i18n.page("p2").t("h"), fake.count("|load")),
        ("Two".into(), 1)
    );
    tauri_guest::set_locale("de").await.unwrap();
    assert_eq!(
        (i18n.t("title"), fake.fetches(), fake.count("|load")),
        ("Anwendung".into(), fetches, 2)
    );
}

/// Given a route the catalog has no page for, then the root answers it; a failing command
/// fails the call and keeps what is shown; a page whose only chunk goes reads the root again.
#[wasm_bindgen_test]
async fn unknown_pages_failures_and_removed_chunks() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("p1").await.unwrap();
    tauri_guest::set_page("ghost").await.unwrap();
    assert_eq!(i18n.page("ghost").t("title"), "App");
    // the plugin fails: the call says so, the texts stay
    let source = get(&fake.0, "source");
    let meta = get(&source, "meta");
    put(
        &source,
        "meta",
        &Function::new_with_args("", "throw new Error('boom')"),
    );
    let err = tauri_guest::set_page("p2").await.unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
    assert_eq!(i18n.page("p1").t("h"), "One");
    put(&source, "meta", &meta);
    // p1 loses its chunk in Rust: the guest clears it
    *server.borrow_mut() = I18n::builder()
        .messages("en", None, r#"{"title": "App", "h": "Root h"}"#)
        .messages("en", Some("p2"), r#"{"h": "Two"}"#)
        .build()
        .unwrap();
    fake.bump();
    fake.deliver(0);
    tauri_guest::set_page("p1").await.unwrap();
    assert_eq!(i18n.page("p1").t("h"), "Root h");
}

/// Given a page preloaded and one loaded by a router guard, when the locale switches, then
/// both follow it; the shown page stays until `set_page` shows the loaded one.
#[wasm_bindgen_test]
async fn preload_and_load_follow_the_locale() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    tauri_guest::preload("p1").await.unwrap();
    tauri_guest::load("p2").await.unwrap();
    server
        .borrow()
        .add_messages("de", Some("p2"), r#"{"h": "Zwei"}"#)
        .unwrap();
    tauri_guest::set_locale("de").await.unwrap();
    assert_eq!(
        (i18n.page("p1").t("h"), i18n.page("p2").t("h")),
        ("Eins".into(), "Zwei".into())
    );
    let calls = fake.count("|meta");
    tauri_guest::set_page("p2").await.unwrap();
    assert_eq!(fake.count("|meta"), calls);
}

/// Given a debug build of the app, when keys miss, then each is reported once, in one batch
/// per microtask; misses while chunks load are none; a release build (no command) turns
/// reporting off after the first refusal.
#[wasm_bindgen_test]
async fn misses_go_to_report_missing_once() {
    let server = server();
    let fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    let _ = (i18n.t("nope"), i18n.t("nope"), i18n.page("p1").t("gone"));
    microtasks().await;
    let missing = JSON::stringify(&get(&fake.0, "missing")).unwrap();
    assert_eq!(
        String::from(missing),
        r#"[{"locale":"en","page":"index","key":"nope"},{"locale":"en","page":"p1","key":"gone"}]"#
    );
    assert_eq!(fake.count("|report_missing"), 1);
    fake.set("dev", false);
    let _ = i18n.t("other");
    microtasks().await;
    let _ = i18n.t("third");
    microtasks().await;
    assert_eq!(fake.count("|report_missing"), 2);
}

/// Given the guest installed as the global instance, then `t!` / `tc!` in components read
/// the plugin's texts and follow `set_locale`; outside Tauri `connect` fails and says why.
#[wasm_bindgen_test]
async fn installed_guest_serves_t_macros() {
    assert!(matches!(
        tauri_guest::connect("index").await,
        Err(Error::Fetch { path, .. }) if path == "__TAURI_INTERNALS__"
    ));
    let server = server();
    let _fake = Fake::new(&rust_source(&server), "en");
    let i18n = tauri_guest::connect("index").await.unwrap();
    i18n_next::install(i18n).unwrap();
    assert_eq!(
        (
            i18n_next::t!("title").to_string(),
            i18n_next::tc!("apples", 1).to_string()
        ),
        ("App".to_owned(), "one apple".to_owned())
    );
    tauri_guest::set_page("p1").await.unwrap();
    tauri_guest::set_locale("de").await.unwrap();
    assert_eq!(
        (
            i18n_next::t!("title").to_string(),
            i18n_next::t!("h", @page = "p1").to_string()
        ),
        ("Anwendung".to_owned(), "Eins".to_owned())
    );
}
