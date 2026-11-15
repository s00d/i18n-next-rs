//! Calls what an app would call, so the linker keeps exactly that part of i18n-next:
//! a catalog built from embedded files (root + page), `t`, `t_with`, a page view, a
//! plural (`tc`) and a locale switch.
//!
//! A binary, not a cdylib: exporting from a cdylib needs `#[unsafe(no_mangle)]` (or
//! wasm-bindgen glue, ~26 KB), and the probe stays under `forbid(unsafe_code)`. The std
//! runtime of a wasm binary (with its allocator) is a constant ~14 KB; `tools/size-check.sh`
//! subtracts it by also building with `--features empty` (no call into i18n-next).
#![forbid(unsafe_code)]

// The `empty` build links i18n-next without calling it (nothing of it is kept), so the
// dependency counts as used and the build has no warnings.
#[cfg(feature = "empty")]
use i18n_next as _;

#[cfg(not(feature = "empty"))]
fn main() {
    use i18n_next::{Args, I18n};
    use std::hint::black_box;

    let files: [(&str, &[u8]); 4] = [
        ("en.json", include_bytes!("../../../examples/locales/en.json")),
        ("ru.json", include_bytes!("../../../examples/locales/ru.json")),
        ("pages/settings/en.json", include_bytes!("../../../examples/locales/pages/settings/en.json")),
        ("pages/settings/ru.json", include_bytes!("../../../examples/locales/pages/settings/ru.json")),
    ];
    let Ok(i18n) = I18n::builder().locale("ru").fallback("en").files(black_box(files)).build() else {
        return;
    };
    black_box(i18n.t(black_box("nav.home")));
    black_box(i18n.t_with(black_box("greeting"), &Args::new().str("name", black_box("Ann"))));
    black_box(i18n.page(black_box("settings")).t(black_box("title")));
    black_box(i18n.tc(black_box("apples"), black_box(21)));
    let _ = black_box(i18n.set_locale(black_box("en")));
}

/// Allocates, grows and frees once, so the allocator (dlmalloc on wasm) is part of the
/// runtime that is subtracted, not charged to i18n-next: any app that allocates has it.
#[cfg(feature = "empty")]
fn main() {
    use std::hint::black_box;
    let mut v: Vec<u8> = Vec::with_capacity(black_box(1));
    v.extend_from_slice(black_box(b"grow past the first capacity"));
    black_box(v);
}
