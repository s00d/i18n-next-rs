//! Named number formats (DESIGN 5.3, 8b.11): any `numberFormats` text either fails `build`
//! with an error or gives formats that never panic, for any locale tag and value. The input
//! is the JSON options of one format, a NUL, a tag, a NUL, then 8 bytes of an `f64`.
#![no_main]

use i18n_next::I18n;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\0');
    let options = parts.next().unwrap_or("");
    let tag = parts.next().unwrap_or("en");
    let tail = parts.next().unwrap_or("").as_bytes();
    let mut bits = [0u8; 8];
    for (b, t) in bits.iter_mut().zip(tail) {
        *b = *t;
    }
    let value = f64::from_le_bytes(bits);
    let formats = format!(r#"{{"x": {{"f": {options}}}}}"#);
    let Ok(i18n) = I18n::builder().iso("x", tag).locale("x").formats(&formats).build() else {
        return;
    };
    let out = i18n.tn_as(value, "f");
    assert!(!out.is_empty(), "{options} {tag:?} {value}");
    let _ = i18n.tn(value);
    let _ = i18n.text("k").number_as(value, "f").to_string();
});
