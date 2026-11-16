//! Stories of the plugin (DESIGN 7.4), driven over IPC and the protocol like guest-js, on
//! tauri's mock runtime. With no other thread delivering, events are sent by the writing
//! thread before the write returns, so each step reads what arrived without waiting; the
//! one story with two threads orders them with channel gates.

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
        .messages("en", Some("設定"), r#"{"h": "Unicode page"}"#)
        .build()
        .unwrap()
}

/// What `convertFileSrc(path, 'i18n-next')` of tauri 2 returns: the whole path through
/// `encodeURIComponent`, on `i18n-next://localhost` (macOS, iOS, Linux) or
/// `http://i18n-next.localhost` (Windows, Android).
fn convert_file_src(path: &str, windows: bool) -> String {
    let mut enc = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            enc.push(b as char);
        } else {
            enc += &format!("%{b:02X}");
        }
    }
    if windows {
        format!("http://i18n-next.localhost/{enc}")
    } else {
        format!("i18n-next://localhost/{enc}")
    }
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
        let tag = "0123456789abcdef";
        for path in [
            format!("/{tag}/{escaped}/index.json"),
            format!("/{tag}/en/{escaped}.json"),
            format!("/{tag}/bundle?locale={escaped}"),
            format!("/{tag}/bundle?locale=en&page={escaped}"),
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
/// the protocol serves the same chunks under content-tagged, cacheable URLs.
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
    // step 3: the protocol serves the same chunk, cacheable, from the app's origin, under
    // the tag `meta` lists
    let tags = app.meta(None, Some("settings"))["tags"].clone();
    let tag = tags["en/settings"].as_str().unwrap().to_owned();
    let path = format!("/{tag}/en/settings.json");
    assert_eq!(path, app.chunk_path("en", "settings"));
    let r = app.request(&path, Some(origin()));
    assert_eq!(r.status(), 200);
    let headers = r.headers();
    assert_eq!(headers["cache-control"], "max-age=31536000, immutable");
    assert_eq!(headers["access-control-allow-origin"], origin());
    assert_eq!(headers["content-type"], "application/json");
    let etag = headers["etag"].clone();
    assert_eq!(etag, format!("\"{tag}\"").as_str());
    assert_eq!(r.body(), br#"{"data":{"h":"Settings"}}"#);
    // step 4: a revalidation with the ETag → 304, no body, the ETag again
    let uri = format!("i18n-next://localhost{path}");
    let revalidate = |tag: &str| {
        app.send(
            tauri::http::Request::get(&uri).header("if-none-match", tag),
            Some(origin()),
        )
    };
    let r = revalidate(etag.to_str().unwrap());
    assert_eq!((r.status().as_u16(), r.body().len()), (304, 0));
    assert_eq!(r.headers()["etag"], etag);
    // step 5: another ETag (or a list without ours) → 200 with the body
    let r = revalidate("\"0000000000000000\", W/\"1\"");
    assert_eq!((r.status().as_u16(), !r.body().is_empty()), (200, true));
    // step 6: the bundle is roots + page in one response
    let (status, body) = app.get(&app.bundle_path("de", Some("settings")));
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
    let path = app.chunk_path("en", "index");
    let r = app.request(&path, Some("https://evil.example"));
    assert_eq!(r.status(), 403);
    assert!(r.headers().get("access-control-allow-origin").is_none());
    // step 2: no origin → served
    assert_eq!(app.request(&path, None).status(), 200);
    // step 3: POST → 405 with Allow
    let uri = format!("i18n-next://localhost{path}");
    let r = app.send(tauri::http::Request::post(&uri), None);
    assert_eq!(
        (r.status().as_u16(), r.headers()["allow"].to_str().unwrap()),
        (405, "GET, HEAD, OPTIONS")
    );
    // step 4: unknown routes → 404
    let tag = &path[1..17];
    for path in [
        "/".to_owned(),
        "/v0".to_owned(),
        "/v0/en/index.json".to_owned(),
        "/zzzzzzzzzzzzzzzz/en/index.json".to_owned(),
        format!("/{tag}/en/index"),
        format!("/{tag}/bundle"),
        format!("/{tag}%ZZ/en/index.json"),
    ] {
        assert_eq!(app.get(&path).0, 404, "{path}");
    }
}

/// Given the protocol, when a page sends a CORS preflight (a custom header) or a `HEAD`,
/// then the preflight is allowed for the app's origins only, with the asked headers, and
/// `HEAD` answers as `GET` without a body.
#[test]
fn protocol_answers_preflight_and_head() {
    let app = App::new(catalog());
    let path = app.chunk_path("en", "index");
    let uri = format!("i18n-next://localhost{path}");
    let preflight = || {
        tauri::http::Request::builder()
            .method("OPTIONS")
            .uri(&uri)
            .header("access-control-request-method", "GET")
            .header("access-control-request-headers", "x-trace, if-none-match")
    };
    // step 1: the app's origin → 204 with methods, headers and the origin
    let r = app.send(preflight(), Some(origin()));
    let h = r.headers();
    assert_eq!(r.status(), 204);
    assert_eq!(h["access-control-allow-origin"], origin());
    assert_eq!(h["access-control-allow-methods"], "GET, HEAD, OPTIONS");
    assert_eq!(h["access-control-allow-headers"], "x-trace, if-none-match");
    // step 2: a foreign origin → 403
    assert_eq!(
        app.send(preflight(), Some("https://evil.example")).status(),
        403
    );
    // step 3: HEAD → the GET's status and headers, no body
    let get = app.request(&path, Some(origin()));
    let head = app.send(
        tauri::http::Request::builder().method("HEAD").uri(&uri),
        Some(origin()),
    );
    assert_eq!(
        (head.status(), head.headers()),
        (get.status(), get.headers())
    );
    assert!(head.body().is_empty() && !get.body().is_empty());
}

/// Given the front end on macOS / Linux and on Windows / Android, when it builds URLs with
/// `convertFileSrc` (the whole path percent-encoded, `?`, `=` and `&` included), then the
/// chunk, a non-ASCII page and the bundle are served; a hand-written escape (`%65n`) is
/// the name it encodes.
#[test]
fn urls_from_convert_file_src_are_served_on_every_platform() {
    let app = App::new(catalog());
    let tag = |l: &str, p: &str| app.chunk_path(l, p)[1..17].to_owned();
    let bundle = app.meta(Some("de"), Some("settings"))["bundle"]
        .as_str()
        .unwrap()
        .to_owned();
    for windows in [false, true] {
        let cases = [
            (format!("{}/de/index.json", tag("de", "index")), "Anwendung"),
            (
                format!("{}/en/設定.json", tag("en", "設定")),
                "Unicode page",
            ),
            (
                format!("{bundle}/bundle?locale=de&page=settings"),
                "Einstellungen",
            ),
        ];
        for (path, needle) in cases {
            let url = convert_file_src(&path, windows);
            let r = app.send(tauri::http::Request::get(&url), Some(origin()));
            let body = String::from_utf8(r.body().clone()).unwrap();
            assert_eq!(r.status(), 200, "{url}");
            assert!(body.contains(needle), "{url}: {body}");
        }
    }
    // a hand-written escape of a name is the name
    let path = format!("/{}/%65n/index.json", tag("en", "index"));
    assert_eq!(app.get(&path).0, 200);
}

/// Given two windows, a bound menu label, a bound tray label and a bound window title,
/// when the locale changes from the tray (Rust), then both windows get `changed` with the
/// new version, the labels show the new locale, the chunks keep their URLs (their content
/// did not change), a wrong tag answers 410 with the current one, and an unbound label
/// stays as it was.
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
    // step 3: a switch changes no chunk: the URL from before the switch still serves;
    // a wrong tag → 410 with the current tag and version, readable cross-origin
    let de = app.chunk_path("de", "index");
    assert_eq!(app.get(&de).0, 200);
    let r = app.request(&format!("/0000000000000000{}", &de[17..]), Some(origin()));
    let h = r.headers();
    assert_eq!(r.status(), 410);
    assert_eq!(h["x-i18n-version"], "1");
    assert_eq!(h["x-i18n-tag"], &de[1..17]);
    assert_eq!(
        h["access-control-expose-headers"],
        "etag, x-i18n-tag, x-i18n-version"
    );
    let body: Value = serde_json::from_slice(r.body()).unwrap();
    assert_eq!(
        (&body["tag"], &body["version"]),
        (&json!(&de[1..17]), &json!(1))
    );
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
    // step 4: guest-js reads data over the protocol: `meta` with the languages picks the
    // same way, once, and a later `load` does not pick again
    let app = App::new(catalog());
    let meta = app.ok("meta", json!({ "languages": ["de-AT"] }));
    assert_eq!(
        (&meta["locale"], &meta["tags"]["de/index"].is_string()),
        (&json!("de"), &true)
    );
    assert_eq!(
        app.ok("meta", json!({ "languages": ["ar"] }))["locale"],
        "de"
    );
    assert_eq!(
        app.ok("load", json!({ "languages": ["ar"] }))["locale"],
        "de"
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
    // step 4: switch to en and back to de, then en again; restart → en (the last choice)
    {
        let app = App::with(plugin());
        for l in ["en", "de", "en"] {
            app.ok("set_locale", json!({ "locale": l }));
        }
    }
    {
        let app = App::with(plugin());
        assert_eq!(app.ok("get_locale", json!({})), json!("en"));
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved, json!({ "locale": "en" }));
    }
    // step 5: a saved locale that is now disabled → default, system pick applies again
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
/// crate, here the test calls the same `reload` itself), then both windows get `changed`,
/// bound labels show the new text, the changed chunk moves to a new URL and the old one
/// answers 410 with the new tag.
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
    let old = app.chunk_path("en", "index");
    // step 1: the file changes; watch would call reload, the test does it at once
    std::fs::write(dir.join("en.json"), r#"{"title": "New"}"#).unwrap();
    h.reload().unwrap();
    let event = json!({ "locale": "en", "version": 1 });
    assert_eq!(
        (drain(&main), drain(&second)),
        (vec![event.clone()], vec![event])
    );
    assert_eq!(label.shown(), ["Old", "New"]);
    // step 2: the data under the new tag; the old URL is gone and names the new one
    let new = app.chunk_path("en", "index");
    assert_ne!(new, old);
    assert_eq!(app.get(&new).1, br#"{"data":{"title":"New"}}"#);
    let (status, body) = app.get(&old);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!((status, &body["tag"]), (410, &json!(&new[1..17])));
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
    assert_eq!(
        app.ok("pages", json!({})),
        json!(["settings", "tray", "設定"])
    );
}

/// Given a regional chain, when the front end asks for `meta`, then it gets the version,
/// the locales, the chain of every locale (to premerge, DESIGN 6.2), the formats and the
/// tags of the chain's chunks; unknown names are refused like everywhere.
#[test]
fn meta_lists_chains_and_tags() {
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                {"code": "en"}, {"code": "de"}, {"code": "de-at", "fallbackLocale": "de"}],
                "numberFormats": {"en": {"money": {"style": "currency", "currency": "USD"}}}}"#,
        )
        .messages("en", None, r#"{"a": "A"}"#)
        .messages("de", None, r#"{"a": "A-de"}"#)
        .messages("de", Some("settings"), r#"{"t": "T-de"}"#)
        .build()
        .unwrap();
    let app = App::new(i18n);
    let meta = app.meta(Some("de-at"), Some("settings"));
    assert_eq!(
        (&meta["version"], &meta["locale"]),
        (&json!(0), &json!("de-at"))
    );
    assert_eq!(meta["chains"]["de-at"], json!(["de-at", "de", "en"]));
    assert_eq!(meta["chains"]["en"], json!(["en"]));
    assert_eq!(meta["numberFormats"]["en"]["money"]["currency"], "USD");
    let tags: Vec<&String> = meta["tags"].as_object().unwrap().keys().collect();
    assert_eq!(tags, ["de/index", "de/settings", "en/index"]);
    assert!(meta.get("chunks").is_none());
    assert_eq!(
        kind(&app.call("meta", json!({ "locale": "../x" })).unwrap_err()),
        "UnknownLocale"
    );
    assert_eq!(
        kind(
            &app.call("meta", json!({ "page": "__proto__" }))
                .unwrap_err()
        ),
        "UnknownPage"
    );
}

/// Given lazy pages evicted past a limit, when pages are read in turn, then an eviction
/// changes neither the version nor any URL: no event, and an evicted page's URL serves the
/// same chunk again (read back from its file).
#[test]
fn eviction_changes_no_version_and_no_url() {
    let files = [
        ("en.json", r#"{"root": "R"}"#),
        ("pages/a/en.json", r#"{"t": "a"}"#),
        ("pages/b/en.json", r#"{"t": "b"}"#),
    ];
    let i18n = I18n::builder()
        .lazy_files(files.iter().map(|(p, b)| (p, b)))
        .evict_pages(1)
        .build()
        .unwrap();
    let app = App::new(i18n);
    let events = app.events(&app.w, EVENT_CHANGED);
    let a = app.chunk_path("en", "a");
    let b = app.chunk_path("en", "b"); // loads b: a is evicted
    assert_eq!((app.handle().version(), drain(&events).len()), (0, 0));
    assert_eq!(app.get(&a), (200, br#"{"data":{"t":"a"}}"#.to_vec()));
    assert_eq!(app.get(&b).0, 200);
}

/// Given a bound target whose relabel fails (a destroyed menu item), when the locale
/// changes, then it is unbound after the failure and never called again.
#[test]
fn a_target_that_fails_to_relabel_is_unbound() {
    #[derive(Clone, Default)]
    struct Gone(Arc<Mutex<usize>>);
    impl Relabel for Gone {
        fn relabel(&self, _: &str) -> tauri::Result<()> {
            *self.0.lock().unwrap() += 1;
            Err(tauri::Error::WindowNotFound)
        }
    }
    let app = App::new(catalog());
    let h = app.handle();
    let (at_bind, later) = (Gone::default(), Gone::default());
    // step 1: fails at bind → not bound
    let id = h.bind(&at_bind, h.text("title"));
    assert!(!h.unbind(id));
    // step 2: works at bind, fails at the first switch → unbound after it
    let ok = Label::default();
    let id = h.bind(&ok, h.text("title"));
    assert!(h.unbind(id));
    let id = {
        #[derive(Clone)]
        struct Once(Label, Gone);
        impl Relabel for Once {
            fn relabel(&self, l: &str) -> tauri::Result<()> {
                if self.0.shown().is_empty() {
                    self.0.relabel(l)
                } else {
                    self.1.relabel(l)
                }
            }
        }
        h.bind(&Once(Label::default(), later.clone()), h.text("title"))
    };
    h.set_locale("de").unwrap();
    h.set_locale("en").unwrap();
    assert_eq!(*later.0.lock().unwrap(), 1);
    assert!(!h.unbind(id));
}

/// Given an extra origin (a page served by `tauri-plugin-localhost`), when it reads the
/// protocol, then it is served; other origins are still refused.
#[test]
fn extra_origins_are_served() {
    let app = App::with(plugin(catalog()).origins(["http://localhost:1430"]));
    let path = app.chunk_path("en", "index");
    let h = app.handle();
    assert!(h.origins().iter().any(|o| o == origin()));
    let get = |from: &str| {
        let req = tauri::http::Request::get(format!("i18n-next://localhost{path}"))
            .header("origin", from)
            .body(())
            .unwrap();
        tauri_plugin_i18n_next::protocol::handle(h, h.origins(), &req)
    };
    let r = get("http://localhost:1430");
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["access-control-allow-origin"],
        "http://localhost:1430"
    );
    assert_eq!(get("http://localhost:9999").status(), 403);
}

/// Given a bound label that is slow on the change to `de` (held by a gate on another
/// thread), when the front end switches to `ar` meanwhile, then that call returns at once
/// and everything follows version order: both windows get `(de, 1)` then `(ar, 2)`, the
/// label ends on the `ar` text and the saved locale is `ar`, never `[(ar, 2), (de, 1)]`.
#[test]
fn concurrent_switches_reach_windows_labels_and_file_in_order() {
    use std::sync::mpsc::{Receiver, Sender, channel};
    #[derive(Clone)]
    struct Slow {
        shown: Label,
        entered: Sender<()>,
        gate: Arc<Mutex<Receiver<()>>>,
    }
    impl Relabel for Slow {
        fn relabel(&self, label: &str) -> tauri::Result<()> {
            if label == "Beenden" {
                self.entered.send(()).unwrap();
                self.gate.lock().unwrap().recv().unwrap();
            }
            self.shown.relabel(label)
        }
    }
    let dir = data_dir("order");
    let app = App::with(plugin(catalog()).persist(true).data_dir(&dir));
    let main = app.events(&app.w, EVENT_CHANGED);
    let second = app.events(&app.second, EVENT_CHANGED);
    let (entered_tx, entered) = channel();
    let (gate, gate_rx) = channel();
    let slow = Slow {
        shown: Label::default(),
        entered: entered_tx,
        gate: Arc::new(Mutex::new(gate_rx)),
    };
    let h = app.handle();
    h.bind(&slow, h.text("menu.quit"));
    std::thread::scope(|s| {
        // step 1: the tray switches to de on its own thread; the label holds it there
        let tray = s.spawn(|| h.set_locale("de").unwrap());
        entered.recv().unwrap();
        // step 2: the front end switches to ar: the call returns, nothing delivered yet
        assert_eq!(app.ok("set_locale", json!({ "locale": "ar" })), json!(2));
        assert!(drain(&main).is_empty());
        // step 3: the gate opens; the tray thread delivers 1, then 2
        gate.send(()).unwrap();
        tray.join().unwrap();
    });
    let want = [
        json!({ "locale": "de", "version": 1 }),
        json!({ "locale": "ar", "version": 2 }),
    ];
    assert_eq!(
        (drain(&main), drain(&second)),
        (want.to_vec(), want.to_vec())
    );
    assert_eq!(slow.shown.shown(), ["Quit", "Beenden", "Quit"]);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.join("i18n-next.json")).unwrap()).unwrap();
    assert_eq!(saved, json!({ "locale": "ar" }));
    drop(app);
    std::fs::remove_dir_all(&dir).unwrap();
}
