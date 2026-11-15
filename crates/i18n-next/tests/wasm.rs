//! wasm-bindgen-test scaffold (DESIGN 7.6): runs in Node and headless Chrome via
//! `wasm-bindgen-test-runner` (`cargo test --target wasm32-unknown-unknown`).
//! Compat goldens and stories join it from M1 on. Native builds compile nothing here.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn version_in_wasm() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}
