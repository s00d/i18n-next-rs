//! Story of the dev missing-key report (DESIGN 8b.7). Its own test binary: the report is
//! one per process, like the catalog's `missing` handler that feeds it.

mod common;

use common::{App, Value, data_dir, drain, json, plugin};
use i18n_next::I18n;
use tauri_plugin_i18n_next::EVENT_MISSING;

/// Given a catalog whose `missing` handler is the plugin's, when Rust and the front end
/// miss keys (one before the plugin starts), then each key is reported once with a
/// count, every window gets `missing` for a new key, and `i18n-missing.json` holds the
/// whole report.
#[test]
fn rust_and_front_end_misses_land_in_one_report() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"ok": "OK"}"#)
        .missing(tauri_plugin_i18n_next::missing)
        .build()
        .unwrap();
    // step 1: a miss before the plugin exists is kept
    assert_eq!(i18n.t("early"), "early");
    let dir = data_dir("missing");
    let app = App::with(plugin(i18n).data_dir(&dir));
    let file = dir.join("i18n-missing.json");
    let report = || -> Value { serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap() };
    assert_eq!(
        report(),
        json!([{ "locale": "en", "page": "index", "key": "early", "count": 1 }])
    );
    let (main, second) = (
        app.events(&app.w, EVENT_MISSING),
        app.events(&app.second, EVENT_MISSING),
    );
    // step 2: a Rust miss → one event per window
    let h = app.handle();
    assert_eq!(h.page("settings").t("nope"), "nope");
    let event = json!({ "locale": "en", "page": "settings", "key": "nope", "count": 1 });
    assert_eq!(
        (drain(&main), drain(&second)),
        (vec![event.clone()], vec![event])
    );
    // step 3: the front end reports the same key and a new one → count, one event
    app.ok(
        "report_missing",
        json!({ "keys": [
            { "locale": "en", "page": "settings", "key": "nope" },
            { "locale": "en", "page": "index", "key": "js.only" }
        ] }),
    );
    assert_eq!(
        drain(&main),
        [json!({ "locale": "en", "page": "index", "key": "js.only", "count": 1 })]
    );
    // step 4: a found key reports nothing; the file has everything, sorted
    assert_eq!(h.t("ok"), "OK");
    assert!(drain(&main).is_empty());
    assert_eq!(
        report(),
        json!([
            { "locale": "en", "page": "index", "key": "early", "count": 1 },
            { "locale": "en", "page": "index", "key": "js.only", "count": 1 },
            { "locale": "en", "page": "settings", "key": "nope", "count": 2 }
        ])
    );
    drop(app);
    std::fs::remove_dir_all(&dir).unwrap();
}
