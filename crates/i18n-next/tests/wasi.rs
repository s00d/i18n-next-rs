//! wasm32-wasip1 tests (DESIGN 7.6): plain `#[test]` run by wasmtime
//! (`CARGO_TARGET_WASM32_WASIP1_RUNNER`). The `dir` / `lazy_dir` stories join it from
//! M1 on. Other targets compile nothing here.
#![cfg(target_os = "wasi")]

#[test]
fn version_in_wasi() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}
