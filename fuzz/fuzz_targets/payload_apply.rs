//! Bytes as a payload into `payload::apply` (DESIGN 8b.11): `Ok` or `Err`, never a panic;
//! a failed apply changes nothing. After a successful one the guest answers lookups, and
//! encoding it again and applying that to a fresh guest gives the same answers (the payload
//! round-trips through its own writer), except for numbers JSON cannot hold (`1e400`
//! reads as `Infinity`), which the writer sends as `null`, as `JSON.stringify` does.
#![no_main]

use i18n_next::{I18n, payload};
use libfuzzer_sys::fuzz_target;

const KEYS: [&str; 5] = ["a", "a.b", "0", "", "x.0.y"];

fuzz_target!(|data: &[u8]| {
    let guest = I18n::builder().fallback("en").build().unwrap();
    let before = guest.version();
    let Ok(_) = payload::apply(&guest, data) else {
        assert_eq!(guest.version(), before, "a failed apply changed the catalog");
        return;
    };
    let pages = guest.pages();
    let (_, again) = payload::encode(&guest, None, pages.first().map(String::as_str), false);
    let copy = I18n::builder().fallback("en").build().unwrap();
    payload::apply(&copy, &again).expect("an encoded payload applies");
    assert_eq!(guest.locale(), copy.locale());
    let finite = |s: &str| !(s.ends_with("Infinity") || s == "NaN");
    for key in KEYS {
        let t = guest.t(key);
        if finite(&t) {
            assert_eq!(t, copy.t(key), "{key:?}");
        }
        if let Some(p) = pages.first() {
            let t = guest.page(p).t(key);
            if finite(&t) {
                assert_eq!(t, copy.page(p).t(key), "{p}/{key:?}");
            }
        }
    }
});
