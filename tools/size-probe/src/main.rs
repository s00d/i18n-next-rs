//! Calls what an app would call, so the linker keeps exactly that part of i18n-next:
//! a catalog built from embedded files (root + page), `t`, `t_with`, a page view, a
//! plural (`tc`), a locale switch, and the M4 surface a real app uses: a `Text` with an
//! argument, `t!` / `tc!` on the instance of a `scope().run`. The full build also formats
//! numbers (`tn`, named currency and compact formats, a `Text` with `{number}`), so its
//! bytes include the CLDR data of the locales `tools/size-check.sh` selects.
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
    use i18n_next::{Args, I18n, scope, t, tc};
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
    let text = i18n.text(black_box("greeting")).arg("name", black_box("Ann"));
    black_box(text.to_string());
    scope().instance(i18n.clone()).locale(black_box("ru")).run(|| {
        black_box(t!("nav.home").to_string());
        black_box(tc!("apples", black_box(5)).to_string());
        black_box(t!(i18n => "greeting", name = black_box("Bob")).to_string());
    });
    // M5: number formats with CLDR data (the full build only).
    #[cfg(feature = "full")]
    {
        let Ok(n) = I18n::builder()
            .formats(black_box(r#"{"ru": {"rub": {"style": "currency", "currency": "RUB"}, "short": {"notation": "compact"}}}"#))
            .locale("ru")
            .build()
        else {
            return;
        };
        black_box(n.tn(black_box(1234.5)));
        black_box(n.tn_as(black_box(1234.5), black_box("rub")));
        black_box(n.tn_as(black_box(1_234_567), black_box("short")));
        black_box(n.text(black_box("total")).number(black_box(12.5)).to_string());
    }
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
