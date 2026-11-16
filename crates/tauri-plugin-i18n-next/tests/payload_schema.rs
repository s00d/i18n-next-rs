//! Characterization golden of everything the front end reads (DESIGN 7.2): the `load`,
//! `load_page` and `meta` payloads, protocol chunks (linked texts included), the bundle,
//! error bodies, the CORS preflight and the `changed` event.
//! `UPDATE_GOLDEN=1` rewrites `tests/__golden__/payload.md`; read every diff whole.

mod common;

use common::{App, drain, json};
use i18n_next::I18n;
use tauri_plugin_i18n_next::EVENT_CHANGED;

#[test]
fn payload_schema_matches_the_golden() {
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en",
                "locales": [{"code": "en", "iso": "en-US", "displayName": "English"},
                            {"code": "ar", "iso": "ar-EG", "dir": "rtl"},
                            {"code": "es", "disabled": true}],
                "numberFormats": {"en": {"money": {"style": "currency", "currency": "USD"}}},
                "datetimeFormats": {"en": {"short": {"dateStyle": "short"}}}}"#,
        )
        .formats(r#"{"en": {"money": {"style": "currency", "currency": "EUR"}}, "ar": {"n": {"maximumFractionDigits": 1}}}"#)
        .messages(
            "en",
            None,
            r#"{"nav": {"home": "Home", "list": ["a", {"b": "B"}]}, "a.b": "own dotted key",
                "esc": "quote \" slash \\ tab \t nl \n u2028  ", "n": 1.50, "t": true,
                "x": null, "__proto__": {"p": 1}, "cart": "=0 empty | {count} item | {count} items",
                "inf": 1e400, "brand": "Acme", "hello": "Hi @:brand"}"#,
        )
        .messages("ar", None, r#"{"nav": {"home": "الرئيسية"}, "brand": "أكمي"}"#)
        .messages("en", Some("settings"), r#"{"title": "Settings"}"#)
        .messages("ar", Some("settings"), r#"{"title": "الإعدادات"}"#)
        .linked_messages(true)
        .build()
        .unwrap();
    let app = App::new(i18n);
    let events = app.events(&app.w, EVENT_CHANGED);
    let text = |b: Vec<u8>| String::from_utf8(b).unwrap();
    let mut out = String::from("# Payload schema (characterization golden)\n\n");
    let mut line = |call: &str, result: String| {
        out.push_str(&format!("{call}\n→ {result}\n\n"));
    };
    line("load {}", text(app.call_raw("load", json!({})).unwrap()));
    line(
        "load {page: settings}",
        text(app.call_raw("load", json!({ "page": "settings" })).unwrap()),
    );
    line(
        "load_page {page: settings}",
        text(
            app.call_raw("load_page", json!({ "page": "settings" }))
                .unwrap(),
        ),
    );
    line(
        "meta {locale: ar, page: settings}",
        text(
            app.call_raw("meta", json!({ "locale": "ar", "page": "settings" }))
                .unwrap(),
        ),
    );
    line(
        "load_page {page: nope}",
        app.call_raw("load_page", json!({ "page": "nope" }))
            .unwrap_err(),
    );
    let paths = [
        app.chunk_path("en", "index"),
        app.chunk_path("ar", "index"),
        app.chunk_path("ar", "settings"),
        app.bundle_path("ar", Some("settings")),
        format!("{}/xx/index.json", &app.chunk_path("en", "index")[..17]),
        "/0000000000000000/en/index.json".to_owned(),
    ];
    let preflight = tauri::http::Request::builder()
        .method("OPTIONS")
        .uri("i18n-next://localhost/x")
        .header("access-control-request-headers", "x-trace");
    for (call, r) in paths
        .iter()
        .map(|path| {
            (
                format!("GET {path}"),
                app.request(path, Some(common::origin())),
            )
        })
        .chain([(
            "OPTIONS (x-trace)".to_owned(),
            app.send(preflight, Some(common::origin())),
        )])
    {
        let mut headers: Vec<String> = r
            .headers()
            .iter()
            .filter(|(k, _)| *k != "access-control-allow-origin")
            .map(|(k, v)| format!("{k}: {}", v.to_str().unwrap()))
            .collect();
        headers.sort();
        line(
            &call,
            format!(
                "{} [{}] {}",
                r.status().as_u16(),
                headers.join("; "),
                text(r.into_body())
            ),
        );
    }
    app.ok("set_locale", json!({ "locale": "ar" }));
    line(
        "event after set_locale ar",
        json!(drain(&events)).to_string(),
    );
    line(
        "available_locales",
        app.ok("available_locales", json!({})).to_string(),
    );
    line("pages", app.ok("pages", json!({})).to_string());

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/__golden__/payload.md");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &out).unwrap();
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        golden == out,
        "payload golden differs (UPDATE_GOLDEN=1 rewrites it):\n{out}"
    );
}
