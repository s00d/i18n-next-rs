//! Exports what an app would call, so the linker keeps exactly that part of i18n-next.
//! Grows with the API (M1: build a catalog from embedded files and call `t`).

/// Keeps `version()` alive in the .wasm.
#[unsafe(no_mangle)]
pub extern "C" fn probe_version_len() -> usize {
    i18n_next::version().len()
}
