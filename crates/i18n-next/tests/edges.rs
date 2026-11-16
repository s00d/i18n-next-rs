//! Edge cases `cargo mutants` found unchecked (M10, DESIGN 7.5): each test pins one decision
//! no story reached. Grouped by the module that makes the decision.
#![cfg(all(feature = "format", not(target_arch = "wasm32")))]

use i18n_next::{Arg, Args, I18n, Timestamp, scope};
use std::sync::Arc;

/// `Arg` equality: strings by content whatever their ownership, other kinds by kind and value.
#[test]
fn arg_equality_by_kind_and_value() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"a": "A", "b": "B"}"#)
        .build()
        .unwrap();
    let same = [
        (Arg::Str("x"), Arg::String(Arc::from("x"))),
        (Arg::Int(3), Arg::Int(3)),
        (Arg::Float(1.5), Arg::Float(1.5)),
        (Arg::Bool(true), Arg::Bool(true)),
        (Arg::Null, Arg::Null),
        (Arg::from(i18n.text("a")), Arg::from(i18n.text("a"))),
        (
            Arg::Number {
                value: 2.into(),
                format: Some("f".into()),
            },
            Arg::Number {
                value: 2.into(),
                format: Some("f".into()),
            },
        ),
        (Arg::date(5i64), Arg::date(5i64)),
    ];
    for (a, b) in &same {
        assert!(a == b, "{a:?} == {b:?}");
    }
    let differ = [
        (Arg::Str("x"), Arg::Str("y")),
        (Arg::Str("3"), Arg::Int(3)),
        (Arg::Int(3), Arg::Str("3")),
        (Arg::Int(3), Arg::Int(4)),
        (Arg::Int(1), Arg::Bool(true)),
        (Arg::Float(1.5), Arg::Float(2.5)),
        (Arg::Bool(true), Arg::Bool(false)),
        (Arg::Null, Arg::Bool(false)),
        (Arg::from(i18n.text("a")), Arg::from(i18n.text("b"))),
        (
            Arg::Number {
                value: 2.into(),
                format: None,
            },
            Arg::Number {
                value: 3.into(),
                format: None,
            },
        ),
        (
            Arg::Number {
                value: 2.into(),
                format: None,
            },
            Arg::Number {
                value: 2.into(),
                format: Some("f".into()),
            },
        ),
        (Arg::date(5i64), Arg::date(6i64)),
    ];
    for (a, b) in &differ {
        assert!(a != b, "{a:?} != {b:?}");
    }
}

/// `Arg::Text` equality reads the parameters of the inner `Text`, in order.
#[test]
fn text_arg_equality_reads_its_parameters() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"a": "A {n}"}"#)
        .build()
        .unwrap();
    let a = |n: i64| Arg::from(i18n.text("a").arg("n", n));
    assert!(a(1) == a(1));
    assert!(a(1) != a(2));
    assert!(Arg::from(i18n.text("a").args([1])) != Arg::from(i18n.text("a").args([1, 2])));
}

/// The typed `Args` builders keep their value and kind.
#[test]
fn args_builders_keep_value_and_kind() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"k": "{i} {f} {b} {s} {z}"}"#)
        .build()
        .unwrap();
    let args = Args::new()
        .int("i", -3)
        .float("f", 1.5)
        .bool("b", false)
        .str("s", "x")
        .null("z");
    assert_eq!(i18n.t_with("k", &args), "-3 1.5 false x null");
}

/// A JSON number in a catalog shows as JS `String(n)`, also where its digits as written
/// would not: below the smallest double, above the largest, at the edges of the exponent.
#[test]
fn catalog_numbers_show_as_js_strings() {
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"a": 4.9e-324, "b": 2e308, "c": 1e307, "d": 1.5e308, "e": 1e-306}"#,
        )
        .build()
        .unwrap();
    let shown: Vec<String> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|k| i18n.t(k).into())
        .collect();
    assert_eq!(
        shown,
        ["5e-324", "Infinity", "1e+307", "1.5e+308", "1e-306"]
    );
}

/// A float argument at a tie between two shortest digit strings shows the even one, as JS
/// does (`{:e}` takes the larger); an odd last digit that is no tie stays (`2^-24`). The
/// sums are exact: the doubles there are 1/8 or 1/4 apart.
#[test]
fn float_ties_take_the_even_digit() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"k": "{n}"}"#)
        .build()
        .unwrap();
    for (v, js) in [
        (562_949_953_421_313.0 + 0.25, "562949953421313.2"),
        (562_949_953_421_313.0 + 0.75, "562949953421313.8"),
        (1_000_000_000_000_001.0 + 0.25, "1000000000000001.2"),
        (700_000_000_000_001.0 + 0.25, "700000000000001.2"),
        (2f64.powi(-24), "5.960464477539063e-8"),
        (5.0 * 2f64.powi(-24), "2.980232238769531e-7"),
        (5.0 * 2f64.powi(-27), "3.725290298461914e-8"),
    ] {
        assert_eq!(i18n.t_with("k", &Args::new().float("n", v)), js, "{v:e}");
    }
}

/// `format` (expected values from Node's `Intl`, ICU 78): a date at the latest instant JS
/// allows, a week year (`Y`, `gd`) across New Year and before year 0, flexible day periods
/// (`zh-Hant` time styles) and wide ones (`ko`), and the hour 12 as PM. (`sc` and `ksh`
/// also use `Y`, `haw` Roman months: the default locale set leaves them out.)
#[test]
fn date_fields_match_intl() {
    let fmt = |locale: &str, options: &str, at: i64| {
        I18n::builder()
            .iso("x", locale)
            .locale("x")
            .time_zone("UTC")
            .datetime_formats(&format!(r#"{{"x": {{"f": {options}}}}}"#))
            .build()
            .unwrap()
            .td_as(at, "f")
    };
    // 2024-03-05 and 2027-01-01 (a Friday: its week year is 2026), in milliseconds
    let (mar5, jan1) = (1_709_596_800_000, 1_798_761_600_000);
    let (min, max) = (-8_640_000_000_000_000, 8_640_000_000_000_000);
    let at = |h: i64, m: i64, s: i64| mar5 + ((h * 60 + m) * 60 + s) * 1000;
    let hm = r#"{"hour": "numeric", "minute": "numeric"}"#;
    let ym = r#"{"year": "numeric", "month": "short"}"#;
    let ym2 = r#"{"year": "2-digit", "month": "short"}"#;
    let short = r#"{"timeStyle": "short"}"#;
    let medium = r#"{"timeStyle": "medium"}"#;
    let h = r#"{"hour": "numeric"}"#;
    let cases: &[(&str, &str, i64, &str)] = &[
        ("en", "{}", max, "9/13/275760"),
        ("en", hm, at(12, 30, 0), "12:30 PM"),
        ("en", hm, at(0, 0, 0), "12:00 AM"),
        ("gd", ym, jan1, "Faoi 2026"),
        ("gd", ym2, jan1, "Faoi 26"),
        ("gd", ym, mar5 + 90 * 86_400_000, "Ògmh 2024"),
        ("gd", ym, min, "Gibl -271821"),
        ("gd", ym2, min, "Gibl -21"),
        ("zh-Hant", short, at(12, 0, 0), "中午12:00"),
        ("zh-Hant", short, at(12, 30, 0), "中午12:30"),
        ("zh-Hant", medium, at(12, 0, 5), "中午12:00:05"),
        ("zh-Hant", short, at(0, 0, 0), "凌晨12:00"),
        ("zh-Hant", short, at(6, 0, 0), "清晨6:00"),
        ("zh-Hant", short, at(15, 0, 0), "下午3:00"),
        ("zh-Hant", short, at(20, 0, 0), "晚上8:00"),
        ("ko", h, at(12, 0, 0), "오후 12시"),
        ("ko", h, at(6, 0, 0), "오전 6시"),
    ];
    for &(locale, options, ms, want) in cases {
        assert_eq!(fmt(locale, options, ms), want, "{locale} {options} {ms}");
    }
}

/// A `Timestamp` is valid inside JS's range only; one from before 1970 keeps its sign.
#[test]
fn timestamps_validity_and_sign() {
    assert!(Timestamp::from(0i64).is_valid());
    assert!(!Timestamp::from(f64::NAN).is_valid());
    assert!(!Timestamp::from(8_640_000_000_000_001i64).is_valid());
    let before = std::time::UNIX_EPOCH - std::time::Duration::from_millis(1500);
    assert_eq!(Timestamp::from(before).millis(), Some(-1500));
}

/// With no instance anywhere, `td` / `td_as` / `tdr` format in `en`, UTC, with the system
/// clock; inside a scope they use its instance and its named formats.
#[test]
fn dates_without_an_instance() {
    let now = Timestamp::from(std::time::SystemTime::now())
        .millis()
        .unwrap();
    assert_eq!(i18n_next::td(1_704_428_640_000i64), "1/5/2024");
    // an hour of margin either way: the clock read above and the one `tdr` reads differ
    // by far less
    assert_eq!(i18n_next::tdr(now - 3 * 3_600_000), "3 hours ago");
    assert_eq!(i18n_next::tdr(f64::NAN), "Invalid Date");
    let i18n = I18n::builder()
        .time_zone("UTC")
        .datetime_formats(r#"{"en": {"y": {"year": "numeric"}}}"#)
        .build()
        .unwrap();
    scope().instance(i18n).run(|| {
        assert_eq!(i18n_next::td_as(1_704_428_640_000i64, "y"), "2024");
    });
}

/// The system zone is taken once, only when known and when no zone was given.
#[cfg(feature = "tz")]
#[test]
fn system_time_zone_is_taken_once() {
    let i18n = I18n::builder().build().unwrap();
    assert!(!i18n.set_system_time_zone("Mars/Olympus"));
    assert!(i18n.set_system_time_zone("Asia/Tokyo"));
    assert_eq!(i18n.td(1_704_481_200_000i64), "1/6/2024");
    assert!(!i18n.set_system_time_zone("UTC"));
}

/// Relative dates by their own words where the locale has one (`numeric: auto`), in both
/// directions.
#[test]
fn relative_words_both_ways() {
    let now = 1_704_428_640_000i64;
    let i18n = I18n::builder().clock(move || now).build().unwrap();
    let auto = i18n_next::RelativeOptions::default().auto();
    assert_eq!(i18n.tdr_with(now + 86_400_000, auto), "tomorrow");
    assert_eq!(i18n.tdr_with(now - 86_400_000, auto), "yesterday");
    assert_eq!(i18n.tdr_with(now + 2 * 86_400_000, auto), "in 2 days");
}

/// Currency spacing (ICU `currencySpacing`): a space between digits and a symbol that ends
/// in a letter, ASCII or not, none next to a symbol character (Node's `Intl`).
#[test]
fn currency_spacing_matches_intl() {
    for (locale, options, want) in [
        (
            "en",
            r#"{"style": "currency", "currency": "USD"}"#,
            "$12.00",
        ),
        (
            "en",
            r#"{"style": "currency", "currency": "USD", "currencyDisplay": "code"}"#,
            "USD\u{a0}12.00",
        ),
        (
            "fa",
            r#"{"style": "currency", "currency": "IRR"}"#,
            "\u{200e}ریال\u{a0}۱۲",
        ),
        (
            "am",
            r#"{"style": "currency", "currency": "ETB"}"#,
            "ብር\u{a0}12.00",
        ),
        (
            "bn",
            r#"{"style": "currency", "currency": "BDT"}"#,
            "১২.০০৳",
        ),
    ] {
        let i18n = I18n::builder()
            .iso("x", locale)
            .locale("x")
            .formats(&format!(r#"{{"x": {{"f": {options}}}}}"#))
            .build()
            .unwrap();
        assert_eq!(i18n.tn_as(12, "f"), want, "{locale} {options}");
    }
}

/// Number format options as `Intl` reads them: unsupported ones and out-of-range values
/// fail the build naming the option; strings and arrays convert as JS `ToNumber` /
/// `ToString` do; nested objects and arrays inside an option are skipped whole; a format
/// named `__proto__` is ignored, as on a JS object.
#[test]
fn number_options_as_intl_reads_them() {
    let build = |options: &str| {
        I18n::builder()
            .formats(&format!(r#"{{"en": {{"f": {options}}}}}"#))
            .build()
    };
    let err = |options: &str| match build(options) {
        Err(i18n_next::Error::UnsupportedFormat { option, .. }) => option,
        other => panic!("{options}: {other:?}"),
    };
    for (options, option) in [
        (r#"{"roundingIncrement": 2}"#, "roundingIncrement"),
        (r#"{"unitDisplay": "short"}"#, "unitDisplay"),
        (
            r#"{"minimumSignificantDigits": 2}"#,
            "minimumSignificantDigits",
        ),
        (
            r#"{"maximumFractionDigits": [true]}"#,
            "maximumFractionDigits",
        ),
    ] {
        assert_eq!(err(options), option, "{options}");
    }
    let shown = |options: &str| build(options).unwrap().tn_as(1.23456, "f");
    assert_eq!(shown(r#"{"roundingIncrement": "0o1"}"#), "1.235");
    assert_eq!(shown(r#"{"roundingIncrement": "0O1"}"#), "1.235");
    assert_eq!(
        shown(r#"{"x": [{"a": {"b": [1]}}, [2]], "y": {"c": [{}]}, "maximumFractionDigits": 1}"#),
        "1.2"
    );
    let i18n = I18n::builder()
        .formats(r#"{"en": {"__proto__": {"style": "percent"}}}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.tn_as(0.5, "__proto__"), "0.5");
}

/// Date options `Intl` validates and then ignores: `localeMatcher` and `formatMatcher`
/// take their two values each, anything else fails the build.
#[test]
fn date_matchers_take_their_values() {
    let build = |options: &str| {
        I18n::builder()
            .datetime_formats(&format!(r#"{{"en": {{"f": {options}}}}}"#))
            .build()
    };
    for ok in [
        r#"{"localeMatcher": "lookup"}"#,
        r#"{"localeMatcher": "best fit"}"#,
        r#"{"formatMatcher": "basic"}"#,
        r#"{"formatMatcher": "best fit"}"#,
    ] {
        assert!(build(ok).is_ok(), "{ok}");
    }
    for (options, option) in [
        (r#"{"localeMatcher": "basic"}"#, "localeMatcher"),
        (r#"{"formatMatcher": "lookup"}"#, "formatMatcher"),
    ] {
        match build(options) {
            Err(i18n_next::Error::UnsupportedFormat { option: o, .. }) => assert_eq!(o, option),
            other => panic!("{options}: {other:?}"),
        }
    }
}

/// Of several formats whose `numberingSystem` the locale does not have, the error names
/// the first by locale and name.
#[test]
fn numbering_errors_name_the_first_format() {
    let config = r#"{"locales": [{"code": "ar", "iso": "ar-EG"}],
        "numberFormats": {"ar": {"b": {"numberingSystem": "latn"}, "a": {"numberingSystem": "latn"}}}}"#;
    match I18n::builder().config_json(config).build() {
        Err(i18n_next::Error::UnsupportedFormat { name, option, .. }) => {
            assert_eq!((name.as_str(), option.as_str()), ("a", "numberingSystem"));
        }
        other => panic!("{other:?}"),
    }
}

/// Plural forms of formatted numbers (currency names) read the operands of
/// the number as shown: visible fraction digits (`is`: `t`), the double past 10^18 (`ro`:
/// `n % 100`), as Node's `Intl` does.
#[cfg(feature = "currency-names")]
#[test]
fn plural_operands_of_shown_numbers_match_intl() {
    let usd = r#"{"style": "currency", "currency": "USD", "currencyDisplay": "name"}"#;
    let usd0 = r#"{"style": "currency", "currency": "USD", "currencyDisplay": "name", "maximumFractionDigits": 0}"#;
    let isk = r#"{"style": "currency", "currency": "ISK", "currencyDisplay": "name", "maximumFractionDigits": 2}"#;
    let eur = r#"{"style": "currency", "currency": "EUR", "currencyDisplay": "name"}"#;
    let big = 1e18 + 512.0;
    let cases: &[(&str, &str, f64, &str)] = &[
        ("ro", usd0, 1.0, "1 dolar american"),
        ("ro", usd0, 19.0, "19 dolari americani"),
        ("ro", usd0, 20.0, "20 de dolari americani"),
        ("ro", usd0, 101.0, "101 dolari americani"),
        (
            "ro",
            usd0,
            big,
            "1.000.000.000.000.000.500 dolari americani",
        ),
        (
            "ro",
            usd0,
            1e18 + 2048.0,
            "1.000.000.000.000.002.000 de dolari americani",
        ),
        (
            "ro",
            usd0,
            1.5e20,
            "150.000.000.000.000.000.000 de dolari americani",
        ),
        (
            "ro",
            usd0,
            1e21,
            "1.000.000.000.000.000.000.000 de dolari americani",
        ),
        ("ro", usd, 1.0, "1,00 dolari americani"),
        ("is", usd, 1.0, "1,00 Bandaríkjadalur"),
        ("is", usd, 1.21, "1,21 Bandaríkjadalur"),
        ("is", usd, 0.31, "0,31 Bandaríkjadalur"),
        ("is", usd, 2.0, "2,00 Bandaríkjadalir"),
        ("is", usd, 11.0, "11,00 Bandaríkjadalir"),
        ("is", usd, 0.5, "0,50 Bandaríkjadalir"),
        ("is", usd, 0.01, "0,01 Bandaríkjadalur"),
        (
            "is",
            usd,
            big,
            "1.000.000.000.000.000.500,00 Bandaríkjadalir",
        ),
        ("is", isk, 1.1, "1,1 íslensk króna"),
        ("is", isk, 1.21, "1,21 íslensk króna"),
        ("is", isk, 2.0, "2 íslenskar krónur"),
        ("is", isk, 21.0, "21 íslensk króna"),
        ("is", isk, 1.5, "1,5 íslenskar krónur"),
        ("is", isk, 101.0, "101 íslensk króna"),
        ("lv", eur, 1.21, "1,21 eiro"),
        (
            "lv",
            eur,
            big,
            "1\u{a0}000\u{a0}000\u{a0}000\u{a0}000\u{a0}000\u{a0}500,00 eiro",
        ),
    ];
    for &(locale, options, v, want) in cases {
        let i18n = I18n::builder()
            .iso("x", locale)
            .locale("x")
            .formats(&format!(r#"{{"x": {{"f": {options}}}}}"#))
            .build()
            .unwrap();
        assert_eq!(i18n.tn_as(v, "f"), want, "{locale} {options} {v}");
    }
}

/// The payload JSON of a chunk is cached; adding messages to that chunk drops the cache, so
/// the next payload carries them.
#[cfg(feature = "payload")]
#[test]
fn payload_follows_added_messages() {
    let i18n = I18n::builder()
        .messages("en", None, r#"{"a": "A"}"#)
        .build()
        .unwrap();
    let text = |i18n: &I18n| {
        String::from_utf8(i18n_next::payload::encode(i18n, None, None, false).1).unwrap()
    };
    assert!(!text(&i18n).contains("\"b\""));
    i18n.add_messages("en", None, r#"{"b": "B"}"#).unwrap();
    let after = text(&i18n);
    assert!(
        after.contains("\"b\":\"B\"") && after.contains("\"a\":\"A\""),
        "{after}"
    );
}

/// Without `tz` only UTC exists: a named zone is refused, dates show UTC to the second.
#[cfg(not(feature = "tz"))]
#[test]
fn without_tz_only_utc() {
    assert!(I18n::builder().time_zone("Asia/Tokyo").build().is_err());
    let i18n = I18n::builder()
        .time_zone("UTC")
        .datetime_formats(r#"{"en": {"t": {"hour": "numeric", "minute": "2-digit", "second": "2-digit", "hourCycle": "h23"}}}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.td_as(1_704_428_640_000i64, "t"), "04:24:00");
}
