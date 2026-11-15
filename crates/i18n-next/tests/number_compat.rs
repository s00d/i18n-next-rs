//! Number goldens (DESIGN 7.2, 5.3): `Intl.NumberFormat` of Node (ICU 78.3, CLDR 48) through
//! `@i18n-micro/core`, replayed natively; the same runs in wasm (`tests/wasm.rs`).
//! `number-props.md` is the property run: 20 000 seeded random values × modern locales ×
//! supported options, recorded once in Node, so no Node is needed at test time.

#[path = "support/number.rs"]
mod number;

/// Given the formats Node recorded for 22 locales × 36 values × 50 option sets and the
/// named formats of an `i18n.json`, when Rust formats the same, then every line matches.
#[test]
fn number_goldens_match_intl() {
    // `num_name` lines (5.1k) run only with `currency-names` (CI: its own nextest step)
    let floor = if cfg!(feature = "currency-names") {
        45_000
    } else {
        39_900
    };
    match number::run(number::NUMBER) {
        Ok(n) => assert!(n > floor, "only {n} lines ran"),
        Err(diff) => panic!("number.md differs:\n{diff}"),
    }
}

/// Given random values, modern locales and supported options, when Rust formats them, then
/// each matches what `Intl.NumberFormat` gave for it.
#[test]
fn random_numbers_match_intl() {
    match number::run(number::PROPS) {
        Ok(n) => assert!(n > 19_000, "only {n} lines ran"),
        Err(diff) => panic!("number-props.md differs:\n{diff}"),
    }
}

/// Given every modern locale and the tags derived from likely subtags and aliases, when a
/// locale has that tag as its `iso`, then it formats as `Intl` does for the tag, and each
/// modern locale's `language_name` is its CLDR autonym.
#[test]
fn locales_resolve_like_intl() {
    match number::run(number::LOCALES) {
        Ok(n) => assert!(n > 2_000, "only {n} lines ran"),
        Err(diff) => panic!("number-locales.md differs:\n{diff}"),
    }
}

/// Given a build with every locale (`I18N_NEXT_LOCALES=all`, its own CI step), when each tag
/// derived from CLDR (and each alias, `ku-IQ` to the root) is a locale's `iso`, then it formats
/// as `Intl` does, and every locale's `language_name` is its autonym or `None` (`ff-Latn`
/// resets its parent's): the data and resolution below the `modern` set.
#[test]
fn all_locales_resolve_like_intl() {
    if option_env!("I18N_NEXT_LOCALES") != Some("all") {
        return;
    }
    match number::run(number::LOCALES_ALL) {
        Ok(n) => assert!(n > 2_800, "only {n} lines ran"),
        Err(diff) => panic!("number-locales-all.md differs:\n{diff}"),
    }
}
