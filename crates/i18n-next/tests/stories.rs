//! Stories of the crate (DESIGN 7.4): one event stream per test, Given/When/Then in the
//! doc comment, `// step n: action → check` in the body. M0 holds the skeleton story that
//! keeps `tests/support` compiling and honest; real stories arrive with M1.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use support::{Clock, Guard, Locales};

/// Given the support simulator, when a test drives it, then files keep their paths,
/// time moves only on `advance`, and an allowed event does not fail the guard.
#[test]
fn simulator_skeleton() {
    // step 1: a locale tree → files come back in path order
    let locales = Locales::new()
        .file("pages/settings/en.json", r#"{"title":"Settings"}"#)
        .file("en.json", r#"{"hello":"Hello"}"#);
    let paths: Vec<_> = locales.files().map(|(p, _)| p).collect();
    assert_eq!(paths, ["en.json", "pages/settings/en.json"]);

    // step 2: the virtual clock moves only when told
    let clock = Clock::at(1_000);
    clock.advance(250);
    assert_eq!(clock.now_ms(), 1_250);

    // step 3: an expected miss is allowed, the guard stays quiet on drop
    let guard = Guard::new();
    guard.allow("nav.unknown");
    guard.record("nav.unknown");
    drop(guard);

    // step 4: the crate is linked and reports its version
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

/// Given a guard, when an unexpected miss is recorded, then the test fails on drop.
#[test]
#[should_panic(expected = "unexpected events")]
fn guard_fails_on_unexpected_event() {
    let guard = Guard::new();
    guard.record("nav.typo");
}
