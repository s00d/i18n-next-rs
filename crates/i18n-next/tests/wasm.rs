//! wasm-bindgen-test (DESIGN 7.6): runs in Node and headless Chrome via
//! `wasm-bindgen-test-runner` (`cargo test --target wasm32-unknown-unknown`). The compat
//! goldens run here on compiled-in fixtures through `files()`, as an app in the browser
//! would load them. Native builds compile nothing here.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

#[path = "support/compat.rs"]
mod compat;

use i18n_next::{Args, I18n};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn version_in_wasm() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

/// Given the compat fixtures embedded in the binary, when the goldens are replayed in
/// wasm, then every line matches, as on native.
#[wasm_bindgen_test]
fn compat_goldens_in_wasm() {
    if let Err(diff) = compat::run() {
        panic!("compat goldens differ:\n{diff}");
    }
}

/// Given embedded files, when the app switches locales and adds messages in wasm, then
/// the snapshot path (thread-local cache, RwLock) works in the single-threaded runtime.
#[wasm_bindgen_test]
fn files_and_switching_in_wasm() {
    let i18n = I18n::builder()
        .locale("en")
        .files([
            ("en.json", br#"{"hi": "Hi, {name}!"}"#.as_slice()),
            ("de.json", br#"{"hi": "Hallo, {name}!"}"#),
            ("pages/user/profile/de.json", br#"{"title": "Profil"}"#),
        ])
        .build()
        .unwrap();
    let args = Args::new().str("name", "Ann");
    assert_eq!(i18n.t_with("hi", &args), "Hi, Ann!");
    i18n.set_locale("de").unwrap();
    assert_eq!(i18n.t_with("hi", &args), "Hallo, Ann!");
    assert_eq!(i18n.page("user-profile").t("title"), "Profil");
    i18n.add_messages("de", None, r#"{"n": 1.5e-7}"#).unwrap();
    assert_eq!(i18n.t("n"), "1.5e-7");
    assert_eq!(i18n.version(), 2);
}
