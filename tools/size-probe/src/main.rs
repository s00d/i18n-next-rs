//! Calls what an app would call, so the linker keeps exactly that part of i18n-next.
//! Grows with the API (M1: build a catalog from embedded files and call `t`).
//!
//! A binary, not a cdylib: exporting from a cdylib needs `#[unsafe(no_mangle)]` (or
//! wasm-bindgen glue, ~26 KB), and the probe stays under `forbid(unsafe_code)`. The std
//! runtime of a wasm binary is a constant ~13 KB; `tools/size-check.sh` subtracts it by
//! also building with `--features empty` (no call into i18n-next).
#![forbid(unsafe_code)]

use std::hint::black_box;

fn main() {
    #[cfg(not(feature = "empty"))]
    black_box(i18n_next::version().len());
}
