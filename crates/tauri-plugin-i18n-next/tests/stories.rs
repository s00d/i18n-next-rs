//! Stories of the plugin (DESIGN 7.4), driven over IPC like guest-js.

mod common;

use common::{App, json};

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
