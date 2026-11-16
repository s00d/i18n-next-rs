//! The browser backend of `tz` (`src/tz/browser.rs`) under wasm-bindgen-test, in Node and
//! headless Chrome (DESIGN 5.4 / 7.6): named zones through `Intl.DateTimeFormat` give the
//! offsets of the bundled IANA data (`tzdb.bin`, the same release as the native checks) on
//! every transition since 1970 ± 1 s and random instants, for zones whose rules have not
//! changed for years (so an engine's older ICU tz data agrees); the system zone is the one
//! `Intl` resolves; unknown names are `None`. Native builds compile nothing.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

#[path = "../src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use js_sys::wasm_bindgen::JsValue;
use js_sys::{Date, Intl, Object, Reflect};
use tz::Zone;
use wasm_bindgen_test::wasm_bindgen_test;

static DATA: &[u8] = include_bytes!("../src/tz/tzdb.bin");

/// Stable since before 2020, with DST, half and quarter hours, southern rules.
const ZONES: &[&str] = &[
    "America/New_York",
    "Europe/Berlin",
    "Europe/London",
    "Asia/Tokyo",
    "Asia/Kolkata",
    "Asia/Kathmandu",
    "Australia/Sydney",
    "Australia/Lord_Howe",
    "America/St_Johns",
    "Pacific/Chatham",
    "UTC",
];

/// 2037-01-01: transitions after this are the POSIX rule on both sides anyway.
const TO: i64 = 2_114_380_800;

#[wasm_bindgen_test]
fn named_zones_match_the_bundled_tzdb() {
    let mut seed = 0x1970u64;
    for name in ZONES {
        let zone = Zone::named(name).unwrap_or_else(|| panic!("Intl does not know {name}"));
        assert_eq!(zone.name(), *name);
        let table = tz::tzdb::find(DATA, name).unwrap();
        let mut ts: Vec<i64> = table
            .times
            .iter()
            .filter(|&&t| t < TO)
            .flat_map(|&t| [t - 1, t, t + 1])
            .collect();
        for _ in 0..300 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ts.push(((seed >> 11) % TO as u64) as i64);
        }
        for t in ts {
            assert_eq!(zone.offset_at(t), table.offset_at(t), "{name} at {t}");
        }
    }
}

/// Given the hour cache, when the same hour is asked again and an hour with a transition at
/// :30 UTC (Kolkata-style offsets make transitions off the hour) is asked second by second,
/// then the answers stay exact.
#[wasm_bindgen_test]
fn hour_cache_stays_exact() {
    // Lord Howe: DST +10:30 → +11:00 ends 2026-04-05 02:00 local = 2026-04-04T15:00Z, and
    // starts 2026-10-04 02:00 local (+10:30) = 2026-10-03T15:30Z: inside an hour
    let zone = Zone::named("Australia/Lord_Howe").unwrap();
    let start = 1_791_041_400;
    for t in start - 1800..start + 1800 {
        let want = if t < start { 37_800 } else { 39_600 };
        assert_eq!(zone.offset_at(t), want, "at {t}");
        assert_eq!(zone.offset_at(t), want, "cached, at {t}");
    }
}

#[wasm_bindgen_test]
fn unknown_names_are_none() {
    for bad in ["Nowhere/Zone", "", "Europe/Berlin/X"] {
        assert!(Zone::named(bad).is_none(), "{bad:?}");
    }
}

/// Given the engine's own zone, then `Zone::system` is named as `Intl` resolves it and its
/// offsets are those of `Date.prototype.getTimezoneOffset`.
#[wasm_bindgen_test]
fn system_zone_is_intl() {
    let fmt = Intl::DateTimeFormat::new(&js_sys::Array::new(), &Object::new());
    let want = Reflect::get(&fmt.resolved_options(), &JsValue::from_str("timeZone"))
        .unwrap()
        .as_string()
        .unwrap();
    let zone = Zone::system();
    assert_eq!(zone.name(), want);
    for t in [0i64, 1_000_000_000, 1_782_864_000, 1_767_225_600] {
        let local = -Date::new(&JsValue::from_f64((t * 1000) as f64)).get_timezone_offset() * 60.0;
        assert_eq!(f64::from(zone.offset_at(t)), local, "at {t}");
    }
}
