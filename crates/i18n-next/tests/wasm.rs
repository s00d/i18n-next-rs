//! wasm-bindgen-test (DESIGN 7.6): runs in Node and headless Chrome via
//! `wasm-bindgen-test-runner` (`cargo test --target wasm32-unknown-unknown`). The compat
//! goldens run here on compiled-in fixtures through `files()`, as an app in the browser
//! would load them, and so do the platform stories shared with `tests/wasi.rs`
//! (`support/platform.rs`, every one but the `dir` story). Native builds compile nothing.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

#[path = "support/compat.rs"]
mod compat;
#[path = "support/platform.rs"]
mod platform;

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

#[wasm_bindgen_test]
fn files_and_switching() {
    platform::files_and_switching();
}

#[wasm_bindgen_test]
fn files_and_messages() {
    platform::files_and_messages();
}

#[wasm_bindgen_test]
fn chains_aliases_and_folding() {
    platform::chains_aliases_and_folding();
}
