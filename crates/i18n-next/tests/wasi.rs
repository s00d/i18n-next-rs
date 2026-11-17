//! Platform stories on native and `wasm32-wasip1` (DESIGN 7.6): plain `#[test]`, run by
//! wasmtime on wasip1 (`CARGO_TARGET_WASM32_WASIP1_RUNNER = "wasmtime run --dir=."`). The
//! bodies live in `support/platform.rs`, shared with `tests/wasm.rs`; the `dir` story runs
//! here only (there is no file system in `wasm32-unknown-unknown`). Native runs them too,
//! so a local `cargo test` checks what CI checks under wasmtime.
#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

#[path = "support/platform.rs"]
mod platform;
mod support;

use support::Guard;

#[test]
fn version_on_the_platform() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn files_and_switching() {
    platform::files_and_switching();
}

#[test]
fn files_and_messages() {
    platform::files_and_messages();
}

#[test]
fn chains_aliases_and_folding() {
    platform::chains_aliases_and_folding();
}

#[test]
fn many_holes_beside_a_huge_argument() {
    platform::many_holes_beside_a_huge_argument();
}

#[test]
fn dir_story() {
    platform::dir_story();
}

/// Given a guard that allowed `x` once, when `x` is recorded twice, then the second one
/// is unexpected: an allowance pays for one occurrence.
#[test]
fn guard_allowance_is_per_occurrence() {
    let guard = Guard::new();
    guard.allow("x");
    guard.record("x");
    assert_eq!(guard.unexpected(), Vec::<String>::new());
    guard.record("x");
    assert_eq!(guard.unexpected(), ["x"]);
    // the test owns the failure it provoked: allow it, then the check passes
    guard.allow("x");
    guard.finish();
}

/// Given a guard held by a catalog's `missing` hook, when the guard is dropped before the
/// catalog, then `finish` has already checked: an allowed miss passes and nothing is
/// checked twice.
#[test]
fn guard_finish_checks_in_any_order() {
    let guard = Guard::new();
    let i18n = i18n_next::I18n::builder()
        .missing(guard.missing())
        .build()
        .unwrap();
    guard.allow("en|index|nope");
    assert_eq!(i18n.t("nope"), "nope");
    guard.finish();
    drop(guard);
    drop(i18n);
}
