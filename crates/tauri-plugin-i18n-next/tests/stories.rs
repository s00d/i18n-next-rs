//! Stories of the plugin (DESIGN 7.4), driven over IPC like guest-js.

mod common;

use common::{App, Value, json};
use tauri_plugin_i18n_next::Error;

/// Stand-in for the M7 commands that reject a name from the frontend (DESIGN 8b.2):
/// fails with the plugin's error type for `kind`.
#[tauri::command]
fn reject(kind: String, name: String) -> tauri_plugin_i18n_next::Result<()> {
    Err(match kind.as_str() {
        "page" => Error::UnknownPage(name),
        _ => Error::UnknownLocale(name),
    })
}

/// Given a command that rejects an unknown page or locale, when the frontend calls it,
/// then guest-js gets `{ kind, message }`: `kind` to branch on, `message` to show.
#[test]
fn rejected_name_reaches_the_frontend_as_kind_and_message() {
    let app = App::with_handler(tauri::generate_handler![reject]);
    let error = |kind: &str, name: &str| -> Value {
        let text = app
            .call("reject", json!({ "kind": kind, "name": name }))
            .expect_err("reject always fails");
        serde_json::from_str(&text).unwrap()
    };
    // step 1: an unknown page
    assert_eq!(
        error("page", "admin"),
        json!({ "kind": "UnknownPage", "message": "unknown page: admin" })
    );
    // step 2: an unknown locale
    assert_eq!(
        error("locale", "xx"),
        json!({ "kind": "UnknownLocale", "message": "unknown locale: xx" })
    );
}

/// Given a mock app with the plugin's commands, when the frontend calls `ping`,
/// then it gets the `i18n-next` version, which is the plugin's release too.
#[test]
fn ping_over_ipc() {
    let app = App::new();
    // step 1: ping → the core version, equal to the plugin version
    let reply = app.ok("ping", json!({}));
    assert_eq!(reply, json!(env!("CARGO_PKG_VERSION")));
    // step 2: an unknown command → an error, not a panic
    assert!(app.call("nope", json!({})).is_err());
}
