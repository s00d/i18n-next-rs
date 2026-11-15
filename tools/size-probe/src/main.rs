//! Calls what an app would call, so the linker keeps exactly that part of i18n-next.
//! Grows with the API (M1: build a catalog from embedded files and call `t`).
//!
//! A binary, not a cdylib: exporting from a cdylib needs `#[unsafe(no_mangle)]` (or
//! wasm-bindgen glue, ~26 KB), and the probe stays under `forbid(unsafe_code)`. The std
//! runtime of a wasm binary is a constant ~13 KB; `tools/size-check.sh` subtracts it by
//! also building with `--features empty` (no call into i18n-next).
#![forbid(unsafe_code)]

// The `empty` build links i18n-next without calling it (nothing of it is kept), so the
// dependency counts as used and the build has no warnings.
#[cfg(feature = "empty")]
use i18n_next as _;

fn main() {
    #[cfg(not(feature = "empty"))]
    std::hint::black_box(i18n_next::version().len());
}
