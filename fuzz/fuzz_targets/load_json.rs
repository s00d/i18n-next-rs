//! Bytes as locale files into `Builder::files` + `build`: `Ok` or `Err`, never a panic, and
//! memory bounded by the input (libFuzzer's `-rss_limit_mb` fails the run otherwise). The
//! first byte picks the layout (root / page / both, links on or off), the rest is the file;
//! a built catalog then answers lookups and `check()`.
#![no_main]

use i18n_next::{Args, I18n};
use libfuzzer_sys::fuzz_target;

const PATHS: [&str; 4] = ["en.json", "pages/a/en.json", "de.json", "pages/a/b/de.json"];

fuzz_target!(|data: &[u8]| {
    let Some((&mode, body)) = data.split_first() else {
        return;
    };
    let path = PATHS[usize::from(mode & 3)];
    // a second copy under another path: merging and layers on the same bytes
    let mut files = vec![(path, body)];
    if mode & 4 != 0 {
        files.push((PATHS[usize::from((mode >> 4) & 3)], body));
    }
    let built = I18n::builder()
        .locale("de")
        .fallback("en")
        .linked_messages(mode & 8 != 0)
        .files(files)
        .build();
    let Ok(i18n) = built else {
        return;
    };
    for key in ["a", "a.b", "0", "", "__proto__", "x.0.y"] {
        let _ = i18n.t(key, str::to_owned);
        let _ = i18n.page("a").t_with(key, &Args::new().str("name", "Ann").or("{name}"));
        let _ = i18n.tc(key, 3);
        let _ = i18n.has(key);
    }
    let _ = i18n.check().to_string();
});
