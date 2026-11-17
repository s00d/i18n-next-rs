//! IPC model of the plugin (DESIGN 7.3): random sessions of a front end and Rust over one
//! mock app with two windows: locale switches from JS and from Rust, `load` / `load_page`,
//! `add_messages`, protocol requests (the pure handler: the current tag from `meta`, a tag
//! handed out earlier, a wrong one; foreign origins),
//! names that are not in the catalog (`BadName`, 8b.2), `bind` / `unbind` of native labels.
//! After each step every answer is compared with a small model: status, payload chunks,
//! the events each window got and the label of each bound target. A tag handed out earlier
//! must still serve exactly when its chunk's content is the same as then (content tags). Each payload is also
//! applied to a guest catalog (`payload::apply`), which must answer like the server.
//! At the end: both windows saw the same events, the last one carries the app's version,
//! and unbound targets never changed after `unbind`.

mod common;

use common::{App, Value, drain, json, origin, plugin};
use i18n_next::{I18n, payload};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tauri_plugin_i18n_next::{BindId, EVENT_CHANGED, Relabel};

const LOCALES: [&str; 3] = ["en", "de", "ar"];
const PAGES: [&str; 3] = ["index", "settings", "about"];
const KEYS: [&str; 3] = ["a", "b", "c"];
/// Names that are no locale or page of the catalog.
const BAD: [&str; 7] = ["../../etc/passwd", "CON", "", "ü", "__proto__", "es", "x/y"];

#[derive(Debug, Clone)]
enum Action {
    SetLocaleJs(usize),
    SetLocaleRust(usize),
    Load(Option<usize>),
    LoadPage(usize),
    Add(usize, usize, usize, u8),
    Get {
        locale: usize,
        page: usize,
        /// 0: the current tag (from `meta`), 1: one handed out earlier for the pair (the
        /// n-th, wrapping), 2: a tag nothing has.
        tag: (u8, usize),
        foreign: bool,
    },
    Bundle(usize, Option<usize>),
    BadName(usize, u8),
    Bind(usize),
    Unbind(usize),
}

fn action() -> impl Strategy<Value = Action> {
    let l = 0..LOCALES.len();
    let p = 0..PAGES.len();
    prop_oneof![
        3 => l.clone().prop_map(Action::SetLocaleJs),
        2 => l.clone().prop_map(Action::SetLocaleRust),
        2 => proptest::option::of(p.clone()).prop_map(Action::Load),
        2 => p.clone().prop_map(Action::LoadPage),
        3 => (l.clone(), p.clone(), 0..KEYS.len(), 0..4u8).prop_map(|(l, p, k, v)| Action::Add(l, p, k, v)),
        4 => (l.clone(), p.clone(), (0..3u8, 0..8usize), prop::bool::weighted(0.15))
            .prop_map(|(locale, page, tag, foreign)| Action::Get { locale, page, tag, foreign }),
        1 => (l, proptest::option::of(p)).prop_map(|(l, p)| Action::Bundle(l, p)),
        2 => (0..BAD.len(), 0..5u8).prop_map(|(n, w)| Action::BadName(n, w)),
        2 => (0..KEYS.len()).prop_map(Action::Bind),
        1 => (0..4usize).prop_map(Action::Unbind),
    ]
}

/// What the catalog holds: `(locale, page) → key → text`, flat keys only.
#[derive(Debug, Clone)]
struct Model {
    chunks: BTreeMap<(usize, usize), BTreeMap<&'static str, String>>,
    active: usize,
    version: u64,
    events: Vec<Value>,
}

impl Model {
    fn new() -> Self {
        let mut chunks = BTreeMap::new();
        let mut put = |l, p, kv: &[(&'static str, &str)]| {
            chunks.insert(
                (l, p),
                kv.iter().map(|(k, v)| (*k, (*v).to_owned())).collect(),
            );
        };
        put(0, 0, &[("a", "A"), ("b", "B")]);
        put(1, 0, &[("a", "A-de")]);
        put(0, 1, &[("a", "A-settings"), ("c", "C-settings")]);
        put(1, 1, &[("c", "C-settings-de")]);
        put(0, 2, &[("b", "B-about")]);
        Model {
            chunks,
            active: 0,
            version: 0,
            events: Vec::new(),
        }
    }

    /// The active chain: the locale, then the global fallback `en`.
    fn chain(&self, locale: usize) -> Vec<usize> {
        if locale == 0 {
            vec![0]
        } else {
            vec![locale, 0]
        }
    }

    fn data(&self, l: usize, p: usize) -> Value {
        self.chunks.get(&(l, p)).map_or(json!({}), |m| json!(m))
    }

    /// The chunks of a payload: per chain locale, root (with `roots`) then `page`.
    fn payload_chunks(&self, locale: usize, page: Option<usize>, roots: bool) -> Value {
        let mut out = Vec::new();
        for l in self.chain(locale) {
            let pairs = [roots.then_some(0), page.filter(|&p| p != 0 || !roots)];
            for p in pairs.into_iter().flatten() {
                if let Some(m) = self.chunks.get(&(l, p)) {
                    out.push(json!({ "locale": LOCALES[l], "page": PAGES[p], "data": m }));
                }
            }
        }
        Value::Array(out)
    }

    /// What `t(key)` answers on the root in the active locale.
    fn t(&self, key: &str) -> String {
        self.chain(self.active)
            .into_iter()
            .find_map(|l| self.chunks.get(&(l, 0))?.get(key).cloned())
            .unwrap_or_else(|| key.to_owned())
    }

    fn change(&mut self) {
        self.version += 1;
        self.events
            .push(json!({ "locale": LOCALES[self.active], "version": self.version }));
    }
}

#[derive(Clone, Default)]
struct Label(Arc<Mutex<Vec<String>>>);

impl Relabel for Label {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        self.0.lock().unwrap().push(label.to_owned());
        Ok(())
    }
}

struct Bound {
    id: BindId,
    key: &'static str,
    label: Label,
    live: bool,
    /// Labels it had when unbound.
    frozen: usize,
}

fn catalog() -> I18n {
    I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                {"code": "en"}, {"code": "de"}, {"code": "ar", "dir": "rtl"},
                {"code": "es", "disabled": true}]}"#,
        )
        .messages("en", None, r#"{"a": "A", "b": "B"}"#)
        .messages("de", None, r#"{"a": "A-de"}"#)
        .messages(
            "en",
            Some("settings"),
            r#"{"a": "A-settings", "c": "C-settings"}"#,
        )
        .messages("de", Some("settings"), r#"{"c": "C-settings-de"}"#)
        .messages("en", Some("about"), r#"{"b": "B-about"}"#)
        .build()
        .unwrap()
}

fn kind(err: &str) -> String {
    serde_json::from_str::<Value>(err).unwrap()["kind"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

/// Applies `bytes` to a fresh guest and checks it answers like the server for every key
/// of the chunks it got.
fn guest_agrees(app: &App, bytes: &[u8], page: Option<&str>) -> Result<(), TestCaseError> {
    let guest = I18n::builder().fallback("en").build().unwrap();
    payload::apply(&guest, bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
    let server = app.handle();
    let view = |i: &I18n| match page {
        Some(p) => i.page(p),
        None => i.with_locale(&i.locale()),
    };
    for key in KEYS {
        let want = view(server).with_locale(&guest.locale()).t(key);
        prop_assert_eq!(view(&guest).t(key), want, "guest {}", key);
    }
    Ok(())
}

fn run(actions: &[Action]) -> Result<(), TestCaseError> {
    let app = App::with(plugin(catalog()).system_locale(false));
    let windows = [
        app.events(&app.w, EVENT_CHANGED),
        app.events(&app.second, EVENT_CHANGED),
    ];
    let mut seen: [Vec<Value>; 2] = Default::default();
    let mut m = Model::new();
    let mut bound: Vec<Bound> = Vec::new();
    // tags handed out, with the data of the pair at that time
    let mut issued: BTreeMap<(usize, usize), Vec<(String, Value)>> = BTreeMap::new();
    let h = app.handle();
    for a in actions {
        match *a {
            Action::SetLocaleJs(l) | Action::SetLocaleRust(l) => {
                if matches!(a, Action::SetLocaleJs(_)) {
                    let v = app.ok("set_locale", json!({ "locale": LOCALES[l] }));
                    let switched = l != m.active;
                    m.active = l;
                    if switched {
                        m.change();
                    }
                    prop_assert_eq!(v, json!(m.version));
                } else {
                    h.set_locale(LOCALES[l]).unwrap();
                    if l != m.active {
                        m.active = l;
                        m.change();
                    }
                }
            }
            Action::Load(page) => {
                let mut args = json!({});
                if let Some(p) = page {
                    args["page"] = json!(PAGES[p]);
                }
                let bytes = app.call_raw("load", args).unwrap();
                let got: Value = serde_json::from_slice(&bytes).unwrap();
                prop_assert_eq!(&got["version"], &json!(m.version));
                prop_assert_eq!(&got["locale"], &json!(LOCALES[m.active]));
                prop_assert_eq!(&got["chunks"], &m.payload_chunks(m.active, page, true));
                prop_assert_eq!(got["locales"].as_array().map(Vec::len), Some(3));
                guest_agrees(&app, &bytes, page.map(|p| PAGES[p]))?;
            }
            Action::LoadPage(p) => {
                let bytes = app
                    .call_raw("load_page", json!({ "page": PAGES[p] }))
                    .unwrap();
                let got: Value = serde_json::from_slice(&bytes).unwrap();
                prop_assert_eq!(&got["chunks"], &m.payload_chunks(m.active, Some(p), false));
            }
            Action::Add(l, p, k, v) => {
                let text = format!("{}-{}{v}", KEYS[k], LOCALES[l]);
                let json = json!({ KEYS[k]: text }).to_string();
                h.add_messages(LOCALES[l], Some(PAGES[p]), &json).unwrap();
                m.chunks.entry((l, p)).or_default().insert(KEYS[k], text);
                m.change();
            }
            Action::Get {
                locale,
                page,
                tag: (way, nth),
                foreign,
            } => {
                let (l, p) = (LOCALES[locale], PAGES[page]);
                let current = app.chunk_path(l, p)[1..17].to_owned();
                // `meta` lists the same tag for a pair with data
                let meta = app.meta(Some(l), Some(p));
                if let Some(t) = meta["tags"].get(format!("{l}/{p}")) {
                    prop_assert_eq!(t, &json!(current));
                }
                let history = issued.entry((locale, page)).or_default();
                let (tag, then) = match way {
                    0 => (current.clone(), Some(m.data(locale, page))),
                    1 if !history.is_empty() => {
                        let (t, d) = &history[nth % history.len()];
                        (t.clone(), Some(d.clone()))
                    }
                    _ => ("0000000000000000".to_owned(), None),
                };
                if way == 0 {
                    history.push((current.clone(), m.data(locale, page)));
                }
                let path = format!("/{tag}/{l}/{p}.json");
                let from = if foreign {
                    "https://ads.example"
                } else {
                    origin()
                };
                let r = app.request(&path, Some(from));
                let fresh = then.as_ref() == Some(&m.data(locale, page));
                let want = if foreign {
                    403
                } else if fresh {
                    200
                } else {
                    410
                };
                prop_assert_eq!(r.status().as_u16(), want, "{}", path);
                if want == 200 {
                    let got: Value = serde_json::from_slice(r.body()).unwrap();
                    prop_assert_eq!(&got["data"], &m.data(locale, page));
                    prop_assert_eq!(&tag, &current);
                }
                if want == 410 {
                    let h = r.headers();
                    prop_assert_eq!(h["x-i18n-tag"].to_str().unwrap(), current.as_str());
                    prop_assert_eq!(h["x-i18n-version"].to_str().unwrap(), m.version.to_string());
                }
            }
            Action::Bundle(l, p) => {
                let path = app.bundle_path(LOCALES[l], p.map(|p| PAGES[p]));
                let (status, body) = app.get(&path);
                prop_assert_eq!(status, 200);
                let got: Value = serde_json::from_slice(&body).unwrap();
                prop_assert_eq!(&got["chunks"], &m.payload_chunks(l, p, true));
                prop_assert!(got.get("locales").is_none());
            }
            Action::BadName(n, way) => {
                let name = BAD[n];
                match way {
                    0 => {
                        let e = app.call("load_page", json!({ "page": name })).unwrap_err();
                        prop_assert_eq!(kind(&e), "UnknownPage");
                    }
                    1 => {
                        let e = app.call("load", json!({ "page": name })).unwrap_err();
                        prop_assert_eq!(kind(&e), "UnknownPage");
                    }
                    2 => {
                        let e = app
                            .call("set_locale", json!({ "locale": name }))
                            .unwrap_err();
                        prop_assert_eq!(kind(&e), "UnknownLocale");
                    }
                    3 => {
                        let esc: String = name.bytes().map(|b| format!("%{b:02X}")).collect();
                        let path = format!(
                            "{}/{esc}/index.json",
                            app.chunk_path("en", "index")[..17].to_owned()
                        );
                        prop_assert_eq!(app.get(&path).0, 404);
                    }
                    _ => {
                        let esc: String = name.bytes().map(|b| format!("%{b:02X}")).collect();
                        let path = format!(
                            "{}/en/{esc}.json",
                            app.chunk_path("en", "index")[..17].to_owned()
                        );
                        prop_assert_eq!(app.get(&path).0, 404);
                    }
                }
            }
            Action::Bind(k) => {
                let label = Label::default();
                let id = h.bind(&label, h.text(KEYS[k]));
                bound.push(Bound {
                    id,
                    key: KEYS[k],
                    label,
                    live: true,
                    frozen: 0,
                });
            }
            Action::Unbind(i) => {
                if let Some(b) = bound.get_mut(i) {
                    prop_assert_eq!(h.unbind(b.id), b.live);
                    if b.live {
                        b.live = false;
                        b.frozen = b.label.0.lock().unwrap().len();
                    }
                }
            }
        }
        // every step: events of both windows, the label of every bound target
        for (w, rx) in windows.iter().enumerate() {
            seen[w].extend(drain(rx));
            prop_assert_eq!(&seen[w], &m.events, "window {}", w);
        }
        for b in &bound {
            let shown = b.label.0.lock().unwrap().clone();
            if b.live {
                prop_assert_eq!(shown.last().cloned(), Some(m.t(b.key)), "label {}", b.key);
                // relabeled only when the text changed
                prop_assert!(shown.windows(2).all(|w| w[0] != w[1]));
            } else {
                prop_assert_eq!(shown.len(), b.frozen, "unbound label changed");
            }
        }
    }
    // end of session: one version everywhere
    prop_assert_eq!(h.version(), m.version);
    prop_assert_eq!(app.ok("get_locale", json!({})), json!(LOCALES[m.active]));
    if let Some(last) = seen[0].last() {
        prop_assert_eq!(&last["version"], &json!(h.version()));
    }
    Ok(())
}

#[test]
fn plugin_matches_the_model() {
    let config = Config {
        cases: 96,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[7; 32]));
    runner
        .run(&prop::collection::vec(action(), 1..40), |a| run(&a))
        .unwrap();
}
