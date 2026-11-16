//! Stories of the plugin (DESIGN 7.4), driven over IPC and the protocol like guest-js, on
//! tauri's mock runtime. Events are sent by the writing thread before the write returns,
//! so each step reads what arrived without waiting.

mod common;

use common::{App, Value, data_dir, drain, json, origin, plugin};
use i18n_next::{I18n, Text};
use std::sync::{Arc, Mutex};
use tauri_plugin_i18n_next::{EVENT_CHANGED, Relabel};

fn catalog() -> I18n {
    I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                {"code": "en", "iso": "en-US", "displayName": "English"},
                {"code": "de", "iso": "de-DE"},
                {"code": "ar", "dir": "rtl"},
                {"code": "es", "disabled": true}]}"#,
        )
        .messages("en", None, r#"{"title": "App", "menu": {"quit": "Quit"}}"#)
        .messages(
            "de",
            None,
            r#"{"title": "Anwendung", "menu": {"quit": "Beenden"}}"#,
        )
        .messages("es", None, r#"{"title": "Aplicación"}"#)
        .messages("en", Some("settings"), r#"{"h": "Settings"}"#)
        .messages("de", Some("settings"), r#"{"h": "Einstellungen"}"#)
        .messages("en", Some("tray"), r#"{"quit": "Quit app"}"#)
        .messages("de", Some("tray"), r#"{"quit": "App beenden"}"#)
        .build()
        .unwrap()
}

/// A native label the test can read: every label it was given, in order.
#[derive(Clone, Default)]
struct Label(Arc<Mutex<Vec<String>>>);

impl Relabel for Label {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        self.0.lock().unwrap().push(label.to_owned());
        Ok(())
    }
}

impl Label {
    fn shown(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn kind(err: &str) -> String {
    serde_json::from_str::<Value>(err).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Given a running app, when the front end sends names that are not in the catalog (path
/// tricks, device names, empty, unicode, `__proto__`, a disabled locale), then every
/// command and protocol route refuses them with `{kind, message}` and nothing changes.
#[test]
fn names_from_the_front_end_are_checked_against_the_catalog() {
    let app = App::new(catalog());
    let bad = [
        "../../etc/passwd",
        "CON",
        "",
        "设置",
        "__proto__",
        "en/../de",
        "es",
    ];
    for name in bad {
        // step 1: commands → UnknownPage / UnknownLocale
        let page = app.call("load_page", json!({ "page": name })).unwrap_err();
        assert_eq!(kind(&page), "UnknownPage", "{name:?}");
        let page = app.call("load", json!({ "page": name })).unwrap_err();
        assert_eq!(kind(&page), "UnknownPage", "{name:?}");
        let locale = app
            .call("set_locale", json!({ "locale": name }))
            .unwrap_err();
        assert_eq!(kind(&locale), "UnknownLocale", "{name:?}");
        // step 2: protocol → 404 with the same body shape
        let escaped: String = name.bytes().map(|b| format!("%{b:02X}")).collect();
        for path in [
            format!("/v0/{escaped}/index.json"),
            format!("/v0/en/{escaped}.json"),
            format!("/v0/bundle?locale={escaped}"),
            format!("/v0/bundle?locale=en&page={escaped}"),
        ] {
            let (status, body) = app.get(&path);
            assert_eq!(status, 404, "{path}");
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert!(
                body["kind"].is_string() && body["message"].is_string(),
                "{path}"
            );
        }
    }
    // step 3: nothing changed
    assert_eq!(app.ok("get_locale", json!({})), json!("en"));
    assert_eq!(app.handle().version(), 0);
}

/// Given the app, when the front end starts, navigates and reads by protocol, then `load`
/// brings the roots of the chain and the metadata, `load_page` only the page chunks, and
/// the protocol serves the same chunks under versioned, cacheable URLs.
#[test]
fn navigation_loads_roots_once_and_pages_on_demand() {
    let app = App::new(catalog());
    // step 1: load → roots + meta, no page chunk
    let start = app.ok("load", json!({}));
    assert_eq!(start["locale"], "en");
    assert_eq!(start["defaultLocale"], "en");
    assert_eq!(start["locales"][2], json!({ "code": "ar", "dir": "rtl" }));
    let pages: Vec<&Value> = start["chunks"].as_array().unwrap().iter().collect();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page"], "index");
    // step 2: load_page → the page chunk alone
    let page = app.ok("load_page", json!({ "page": "settings" }));
    assert_eq!(
        page["chunks"],
        json!([{ "locale": "en", "page": "settings", "data": { "h": "Settings" } }])
    );
    // step 3: the protocol serves the same chunk, cacheable, from the app's origin
    let r = app.request("/v0/en/settings.json", Some(origin()));
    assert_eq!(r.status(), 200);
    let headers = r.headers();
    assert_eq!(headers["cache-control"], "max-age=31536000, immutable");
    assert_eq!(headers["access-control-allow-origin"], origin());
    assert_eq!(headers["content-type"], "application/json");
    let etag = headers["etag"].clone();
    assert_eq!(r.body(), br#"{"h":"Settings"}"#);
    // step 4: a revalidation with the ETag → 304, no body
    let req = tauri::http::Request::get("i18n-next://localhost/v0/en/settings.json")
        .header("if-none-match", etag)
        .body(())
        .unwrap();
    let origins = [origin().to_owned()];
    let r = tauri_plugin_i18n_next::protocol::handle(app.handle(), &origins, &req);
    assert_eq!((r.status().as_u16(), r.body().len()), (304, 0));
    // step 5: the bundle is roots + page in one response
    let (status, body) = app.get("/v0/bundle?locale=de&page=settings");
    assert_eq!(status, 200);
    let bundle: Value = serde_json::from_slice(&body).unwrap();
    let names: Vec<(&str, &str)> = bundle["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["locale"].as_str().unwrap(), c["page"].as_str().unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            ("de", "index"),
            ("de", "settings"),
            ("en", "index"),
            ("en", "settings")
        ]
    );
}

/// Given the protocol, when a request comes from a foreign frame, with another method, or
/// to an unknown route, then it is refused; a request without `Origin` is served.
#[test]
fn protocol_refuses_foreign_origins_and_methods() {
    let app = App::new(catalog());
    // step 1: foreign origin → 403, no CORS header
    let r = app.request("/v0/en/index.json", Some("https://evil.example"));
    assert_eq!(r.status(), 403);
    assert!(r.headers().get("access-control-allow-origin").is_none());
    // step 2: no origin → served
    assert_eq!(app.request("/v0/en/index.json", None).status(), 200);
    // step 3: POST → 405 with Allow
    let req = tauri::http::Request::post("i18n-next://localhost/v0/en/index.json")
        .body(())
        .unwrap();
    let r = tauri_plugin_i18n_next::protocol::handle(app.handle(), &[], &req);
    assert_eq!(
        (r.status().as_u16(), r.headers()["allow"].to_str().unwrap()),
        (405, "GET")
    );
    // step 4: unknown routes → 404
    for path in [
        "/",
        "/v0",
        "/vx/en/index.json",
        "/v0/en/index",
        "/v0/bundle",
    ] {
        assert_eq!(app.get(path).0, 404, "{path}");
    }
}

/// Given two windows, a bound menu label, a bound tray label and a bound window title,
/// when the locale changes from the tray (Rust), then both windows get `changed` with the
/// new version, the labels show the new locale, the old URLs answer 410, and an unbound
/// label stays as it was.
#[test]
fn locale_switch_from_the_tray_reaches_windows_and_labels() {
    let app = App::new(catalog());
    let main = app.events(&app.w, EVENT_CHANGED);
    let second = app.events(&app.second, EVENT_CHANGED);
    let h = app.handle();
    let (menu, tray, gone) = (Label::default(), Label::default(), Label::default());
    h.bind(&menu, h.text("menu.quit"));
    h.bind(&tray, h.page("tray").text("quit"));
    let unbound = h.bind(&gone, h.text("title"));
    h.bind(&app.w, h.text("title"));
    assert!(h.unbind(unbound));
    assert!(!h.unbind(unbound));
    // step 1: bind shows the current text at once
    assert_eq!(
        (menu.shown(), tray.shown()),
        (vec!["Quit".to_owned()], vec!["Quit app".to_owned()])
    );
    // step 2: switch from Rust → one event per window, labels relabeled
    h.set_locale("de").unwrap();
    let event = json!({ "locale": "de", "version": 1 });
    assert_eq!(
        (drain(&main), drain(&second)),
        (vec![event.clone()], vec![event])
    );
    assert_eq!(menu.shown(), ["Quit", "Beenden"]);
    assert_eq!(tray.shown(), ["Quit app", "App beenden"]);
    assert_eq!(gone.shown(), ["App"]);
    // step 3: the old version is gone, the new one serves the new locale
    let r = app.request("/v0/de/index.json", Some(origin()));
    assert_eq!(
        (
            r.status().as_u16(),
            r.headers()["x-i18n-version"].to_str().unwrap()
        ),
        (410, "1")
    );
    assert_eq!(app.get("/v1/de/index.json").0, 200);
    // step 4: the same switch again → no event, no relabel
    assert_eq!(app.ok("set_locale", json!({ "locale": "de" })), json!(1));
    assert!(drain(&main).is_empty());
    assert_eq!(menu.shown().len(), 2);
    // step 5: switching from the front end works the same
    assert_eq!(app.ok("set_locale", json!({ "locale": "en" })), json!(2));
    assert_eq!(drain(&second), [json!({ "locale": "en", "version": 2 })]);
    assert_eq!(menu.shown(), ["Quit", "Beenden", "Quit"]);
}

#[tauri::command]
fn x_title(state: tauri::State<'_, tauri_plugin_i18n_next::Handle>) -> Text {
    state.text("title")
}

/// Given a command that returns a `Text`, when the front end calls it, then it gets the
/// string in the active locale, also after a switch.
#[test]
fn text_returned_by_a_command_arrives_translated() {
    let plugin = plugin(catalog());
    let app = App::with_handler(plugin, tauri::generate_handler![x_title]);
    // step 1: in en
    assert_eq!(app.ok("x_title", json!({})), json!("App"));
    // step 2: after a switch
    app.ok("set_locale", json!({ "locale": "de" }));
    assert_eq!(app.ok("x_title", json!({})), json!("Anwendung"));
}

/// Given no saved locale, when the first `load` brings `navigator.languages`, then the
/// best available locale becomes active (disabled ones are never picked); later loads do
/// not pick again.
#[test]
fn first_load_picks_the_system_locale() {
    let app = App::new(catalog());
    // step 1: es is disabled, de-AT → de
    let start = app.ok("load", json!({ "languages": ["es-ES", "de-AT", "en"] }));
    assert_eq!(start["locale"], "de");
    assert_eq!(start["chunks"][0]["data"]["title"], "Anwendung");
    // step 2: a later load with other languages changes nothing
    let again = app.ok("load", json!({ "languages": ["ar"] }));
    assert_eq!(again["locale"], "de");
    // step 3: with system_locale(false) nothing is picked
    let off = App::with(plugin(catalog()).system_locale(false));
    assert_eq!(
        off.ok("load", json!({ "languages": ["de"] }))["locale"],
        "en"
    );
}

/// Given persist on, when the user switches to `de` and the app restarts on the same config
/// dir, then it starts in `de`, before and over the system locale; a saved locale that is
/// no longer available is ignored. A system pick is not saved.
#[test]
fn restart_keeps_the_chosen_locale() {
    let dir = data_dir("persist");
    let plugin = || plugin(catalog()).persist(true).data_dir(&dir);
    let file = dir.join("i18n-next.json");
    // step 1: a system pick is not remembered
    {
        let app = App::with(plugin());
        assert_eq!(
            app.ok("load", json!({ "languages": ["ar"] }))["locale"],
            "ar"
        );
        assert!(!file.exists());
        // step 2: an explicit switch is
        app.ok("set_locale", json!({ "locale": "de" }));
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved, json!({ "locale": "de" }));
    }
    // step 3: restart → de, the system languages do not override it
    {
        let app = App::with(plugin());
        assert_eq!(app.ok("get_locale", json!({})), json!("de"));
        assert_eq!(
            app.ok("load", json!({ "languages": ["ar"] }))["locale"],
            "de"
        );
    }
    // step 4: a saved locale that is now disabled → default, system pick applies again
    std::fs::write(&file, r#"{"locale": "es"}"#).unwrap();
    let app = App::with(plugin());
    assert_eq!(app.ok("get_locale", json!({})), json!("en"));
    assert_eq!(
        app.ok("load", json!({ "languages": ["ar"] }))["locale"],
        "ar"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Given a `dir` catalog (what `watch` reloads in dev), when a file changes and the catalog
/// reloads (the real watch is off: its timing is covered with a virtual clock in the core
/// crate, here the test calls the same `reload` itself), then both windows get `changed`, bound labels show the new text, and the next
/// load serves it under the new version.
#[test]
fn reload_after_a_file_change_reaches_every_window() {
    let dir = data_dir("watch");
    std::fs::write(dir.join("en.json"), r#"{"title": "Old"}"#).unwrap();
    let i18n = I18n::builder().dir(&dir).build().unwrap();
    let app = App::new(i18n);
    let (main, second) = (
        app.events(&app.w, EVENT_CHANGED),
        app.events(&app.second, EVENT_CHANGED),
    );
    let label = Label::default();
    let h = app.handle();
    h.bind(&label, h.text("title"));
    // step 1: the file changes; watch would call reload, the test does it at once
    std::fs::write(dir.join("en.json"), r#"{"title": "New"}"#).unwrap();
    h.reload().unwrap();
    let event = json!({ "locale": "en", "version": 1 });
    assert_eq!(
        (drain(&main), drain(&second)),
        (vec![event.clone()], vec![event])
    );
    assert_eq!(label.shown(), ["Old", "New"]);
    // step 2: the data under the new version
    assert_eq!(app.get("/v1/en/index.json").1, br#"{"title":"New"}"#);
    assert_eq!(
        app.ok("load", json!({}))["chunks"][0]["data"]["title"],
        "New"
    );
    drop(app);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Given the plugin, when the front end asks for the lists, then it gets the available
/// locales (no disabled ones, `i18n.json` fields camelCase) and the known pages.
#[test]
fn locales_and_pages_for_the_switcher() {
    let app = App::new(catalog());
    assert_eq!(
        app.ok("available_locales", json!({})),
        json!([
            { "code": "en", "iso": "en-US", "displayName": "English" },
            { "code": "de", "iso": "de-DE" },
            { "code": "ar", "dir": "rtl" }
        ])
    );
    assert_eq!(app.ok("pages", json!({})), json!(["settings", "tray"]));
}
