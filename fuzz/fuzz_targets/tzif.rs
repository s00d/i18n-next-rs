//! Bytes into the own time-zone readers (DESIGN 5.4, 8b.11): as a TZif file, an Android
//! tzdata container, a bundled tzdb and (as text) a POSIX `TZ` rule. Each gives `None` or a
//! value, never a panic or an overflow, and every value answers `offset_at` at the extremes.
//! The module is crate-private: included with `#[path]`.
#![no_main]
// `src/tz` tests `feature = "tzdb"`, a feature of i18n-next, not of this crate
#![allow(unexpected_cfgs)]

#[path = "../../crates/i18n-next/src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use libfuzzer_sys::fuzz_target;

const AT: [i64; 9] = [
    i64::MIN,
    -(1 << 40),
    -2_208_988_800,
    -1,
    0,
    1_700_000_000,
    4_102_444_800,
    1 << 40,
    i64::MAX,
];

fuzz_target!(|data: &[u8]| {
    let tables = [
        tz::tzif::parse(data),
        tz::tzif::android_find(data, "Test/V2").and_then(tz::tzif::parse),
        tz::tzdb::find(data, "Europe/Berlin"),
    ];
    for t in tables.iter().flatten() {
        for at in AT {
            let _ = t.offset_at(at);
        }
    }
    if let Some(rule) = std::str::from_utf8(data)
        .ok()
        .and_then(tz::posix::Posix::parse)
    {
        for at in AT {
            let _ = rule.offset_at(at);
        }
    }
    let _ = tz::civil_from_unix(i64::from_le_bytes(
        data.get(..8)
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0; 8]),
    ));
});
