//! Date goldens (DESIGN 7.2, 5.3): `Intl.DateTimeFormat` and `Intl.RelativeTimeFormat` of Node
//! (ICU 78.3, CLDR 48) through `@i18n-micro/core`, replayed natively; the same runs in wasm
//! (`tests/wasm.rs`). `date-props.md` is the property run: 20 000 seeded random instants ×
//! modern locales × supported options and zones, recorded once in Node.

// zones and CLDR data: features `format` and `tz` (defaults)
#![cfg(all(feature = "format", feature = "tz"))]

#[path = "support/date.rs"]
mod date;

/// Given 16 locales × 26 instants (DST edges, leap day, year 9999, 2 BC, the ends of `Date`,
/// invalid ones) × 40 option sets in UTC and Berlin (5 in New York, Kolkata and Lord Howe)
/// and the named formats of an `i18n.json`, when Rust formats the same, then every line
/// matches.
#[test]
fn date_goldens_match_intl() {
    if !date::zones_pinned("date_goldens_match_intl") {
        return;
    }
    match date::run(date::DATE) {
        Ok(n) => assert!(n > 39_000, "only {n} lines ran"),
        Err(diff) => panic!("date.md differs:\n{diff}"),
    }
}

/// Given offsets around every unit's edge in both directions, every style and `numeric`, when
/// Rust says them relative to the same `now`, then each matches `formatRelativeTime`.
#[test]
fn relative_goldens_match_intl() {
    if !date::zones_pinned("relative_goldens_match_intl") {
        return;
    }
    match date::run(date::RELATIVE) {
        Ok(n) => assert!(n > 9_800, "only {n} lines ran"),
        Err(diff) => panic!("relative.md differs:\n{diff}"),
    }
}

/// Given random instants, modern locales, supported options and zones, when Rust formats
/// them, then each matches what `Intl.DateTimeFormat` gave for it.
#[test]
fn random_dates_match_intl() {
    if !date::zones_pinned("random_dates_match_intl") {
        return;
    }
    match date::run(date::PROPS) {
        Ok(n) => assert!(n > 19_900, "only {n} lines ran"),
        Err(diff) => panic!("date-props.md differs:\n{diff}"),
    }
}
