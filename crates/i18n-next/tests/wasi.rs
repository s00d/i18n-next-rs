//! wasm32-wasip1 tests (DESIGN 7.6): plain `#[test]` run by wasmtime
//! (`CARGO_TARGET_WASM32_WASIP1_RUNNER`). Other targets compile nothing here.
#![cfg(target_os = "wasi")]

use i18n_next::{Args, Error, I18n};

#[test]
fn version_in_wasi() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

/// Given embedded files and `messages`, when the app builds and reads in WASI, then pages,
/// fallbacks, interpolation and the broken-file error work as on native.
#[test]
fn files_and_messages_in_wasi() {
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .files([
            ("en.json", br#"{"a": "A", "hi": "Hi {name}"}"#.as_slice()),
            ("pages/settings/de.json", br#"{"title": "Einstellungen"}"#),
        ])
        .messages("de", None, r#"{"hi": "Hallo {name}"}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.t("a"), "A");
    assert_eq!(
        i18n.t_with("hi", &Args::new().str("name", "Ann")),
        "Hallo Ann"
    );
    assert_eq!(i18n.page("settings").t("title"), "Einstellungen");

    let err = I18n::builder()
        .files([("de.json", b"{")])
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Json { path, .. } if path == "de.json"));
}
