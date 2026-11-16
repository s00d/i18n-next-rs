//! Stories of number formats (DESIGN 5.3, 8b.14): named formats from `i18n.json` and the
//! builder, refused options, lookup by truncated codes, the `iso` of a locale, numbers in a
//! `Text` and the macros, autonyms.

use i18n_next::{Arg, Error, I18n, scope, t, tn};

/// Given an `i18n.json` whose named format uses an option outside the subset, when the
/// catalog is built, then `build` fails naming the locale, the format and the option; a
/// value `Intl` throws on and a currency format without a currency fail the same way.
#[test]
fn unsupported_options_fail_the_build() {
    let config = r#"{"locales": [{"code": "de"}],
        "numberFormats": {"de": {"ok": {"style": "percent"}, "signed": {"signDisplay": "always"}}}}"#;
    match I18n::builder().config_json(config).build() {
        Err(Error::UnsupportedFormat {
            locale,
            name,
            option,
        }) => assert_eq!(
            (&*locale, &*name, &*option),
            ("de", "signed", "signDisplay")
        ),
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
    for (options, option) in [
        (r#"{"style": "unit", "unit": "meter"}"#, "unit"),
        (r#"{"notation": "scientific"}"#, "notation"),
        (
            r#"{"notation": "compact", "compactDisplay": "long"}"#,
            "compactDisplay",
        ),
        (r#"{"notation": "compact", "style": "percent"}"#, "notation"),
        (
            r#"{"minimumFractionDigits": 3, "maximumFractionDigits": 1}"#,
            "maximumFractionDigits",
        ),
        (r#"{"maximumFractionDigits": 101}"#, "maximumFractionDigits"),
        (r#"{"style": "currency"}"#, "currency"),
        (r#"{"style": "currency", "currency": "usdx"}"#, "currency"),
        (
            r#"{"maximumSignificantDigits": 2}"#,
            "maximumSignificantDigits",
        ),
        (r#"{"useGrouping": "sometimes"}"#, "useGrouping"),
    ] {
        let err = I18n::builder()
            .formats(&format!(r#"{{"en": {{"x": {options}}}}}"#))
            .build()
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("format \"x\" of \"en\": option \"{option}\" is not supported"),
            "{options}"
        );
    }
    // defaults change nothing, unknown keys are ignored, as by `Intl`
    let ok = I18n::builder()
        .formats(r#"{"en": {"x": {"signDisplay": "auto", "notation": "standard", "localeMatcher": "lookup", "foo": 1}}}"#)
        .build()
        .unwrap();
    assert_eq!(ok.tn_as(1234.5, "x"), "1,234.5");
    // a broken `numberFormats` is a config error with its path
    assert!(matches!(
        I18n::builder().formats(r#"{"en": 1}"#).build(),
        Err(Error::Config { .. })
    ));
}

/// Given named formats under `zh` and `zh-Hant`, when a locale `zh-Hant-TW` asks for them,
/// then each is found by the code made shorter (`FormatService`), formatted by the locale's
/// own tag; a name nowhere is the default format, and a view asks in its own locale.
#[test]
fn named_formats_by_truncated_code() {
    let i18n = I18n::builder()
        .locale("zh-Hant-TW")
        .formats(
            r#"{"zh": {"money": {"style": "currency", "currency": "CNY"}},
                "zh-Hant": {"short": {"notation": "compact"}},
                "en": {"money": {"style": "currency", "currency": "USD"}}}"#,
        )
        .build()
        .unwrap();
    assert_eq!(i18n.tn_as(1234.5, "money"), "CN¥1,234.50");
    assert_eq!(i18n.tn_as(1_234_567, "short"), "123萬");
    assert_eq!(i18n.tn_as(1234.5, "nope"), "1,234.5");
    assert_eq!(i18n.with_locale("en").tn_as(1234.5, "money"), "$1,234.50");
    assert_eq!(
        i18n.with_locale("en").tn_as(1_234_567, "short"),
        "1,234,567"
    );
    // a later source wins over `i18n.json`
    let i18n = I18n::builder()
        .config_json(r#"{"numberFormats": {"en": {"p": {"style": "percent"}}}}"#)
        .formats(r#"{"en": {"p": {"style": "percent", "minimumFractionDigits": 1}}}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.tn_as(0.5, "p"), "50.0%");
}

/// Given option values `Intl` reads by `ToNumber`, `ToString` and `ToBoolean` (Node 22 output
/// in the comments), when the catalog is built, then they format as `Intl` formats them: digit
/// options from `"0x2"`, `""`, `null`, `true`, `[2]`; `useGrouping` off for `""`, `0`, `null`;
/// the last of a repeated key; `compactDisplay` without compact and `numberingSystem` at the
/// locale's default as no-ops. What `Intl` throws on stays an error.
#[test]
fn options_read_like_intl() {
    let cases = [
        // {"minimumFractionDigits":"0x2"}: 1,234.50
        (r#"{"minimumFractionDigits": "0x2"}"#, 1234.5, "1,234.50"),
        (r#"{"minimumFractionDigits": " 0b11 "}"#, 1.5, "1.500"),
        (r#"{"minimumFractionDigits": ""}"#, 1234.5, "1,234.5"),
        (r#"{"maximumFractionDigits": null}"#, 1234.5, "1,235"),
        (r#"{"maximumFractionDigits": true}"#, 1234.56, "1,234.6"),
        (r#"{"maximumFractionDigits": [2]}"#, 1.005, "1.01"),
        (r#"{"maximumFractionDigits": [[1]]}"#, 1.25, "1.3"),
        (r#"{"maximumFractionDigits": ".5"}"#, 1.5, "2"),
        (r#"{"minimumIntegerDigits": "3"}"#, 7.0, "007"),
        (r#"{"useGrouping": ""}"#, 1234567.891, "1234567.891"),
        (r#"{"useGrouping": 0}"#, 1234567.891, "1234567.891"),
        (r#"{"useGrouping": null}"#, 1234567.891, "1234567.891"),
        (r#"{"useGrouping": ["always"]}"#, 1234.0, "1,234"),
        (r#"{"useGrouping": "false"}"#, 1234.0, "1,234"),
        (
            r#"{"signDisplay": "never", "signDisplay": "auto"}"#,
            -1.0,
            "-1",
        ),
        (r#"{"style": "percent", "style": "decimal"}"#, 0.5, "0.5"),
        (r#"{"style": ["percent"]}"#, 0.5, "50%"),
        (
            r#"{"compactDisplay": "long"}"#,
            1234567.891,
            "1,234,567.891",
        ),
        (r#"{"numberingSystem": "latn"}"#, 1234.5, "1,234.5"),
        (r#"{"numberingSystem": "LATN"}"#, 1234.5, "1,234.5"),
        (r#"{"roundingIncrement": "1"}"#, 1.5, "1.5"),
    ];
    for (options, v, want) in cases {
        let i18n = I18n::builder()
            .formats(&format!(r#"{{"en": {{"x": {options}}}}}"#))
            .build()
            .unwrap_or_else(|e| panic!("{options}: {e}"));
        assert_eq!(i18n.tn_as(v, "x"), want, "{options}");
    }
    // the default numbering system of the format's locale, by its `iso`
    let ar = I18n::builder()
        .iso("ar", "ar-EG")
        .formats(r#"{"ar": {"x": {"numberingSystem": "arab"}}}"#)
        .build()
        .unwrap();
    assert_eq!(ar.with_locale("ar").tn_as(1234.5, "x"), "١٬٢٣٤٫٥");
    // a format given again replaces the first, `numberingSystem` and all
    let again = I18n::builder()
        .formats(r#"{"en": {"x": {"numberingSystem": "arab"}}}"#)
        .formats(r#"{"en": {"x": {"style": "percent"}}}"#)
        .build()
        .unwrap();
    assert_eq!(again.tn_as(0.5, "x"), "50%");
    for (locale, options, option) in [
        ("en", r#"{"numberingSystem": "arab"}"#, "numberingSystem"),
        ("ar", r#"{"numberingSystem": "latn"}"#, "numberingSystem"),
        ("en", r#"{"numberingSystem": "ab"}"#, "numberingSystem"),
        ("en", r#"{"compactDisplay": "foo"}"#, "compactDisplay"),
        (
            "en",
            r#"{"minimumFractionDigits": "1_0"}"#,
            "minimumFractionDigits",
        ),
        (
            "en",
            r#"{"minimumFractionDigits": "inf"}"#,
            "minimumFractionDigits",
        ),
        (
            "en",
            r#"{"minimumFractionDigits": "Infinity"}"#,
            "minimumFractionDigits",
        ),
        (
            "en",
            r#"{"minimumFractionDigits": [1, 2]}"#,
            "minimumFractionDigits",
        ),
        (
            "en",
            r#"{"minimumFractionDigits": {}}"#,
            "minimumFractionDigits",
        ),
        (
            "en",
            r#"{"maximumFractionDigits": "0x"}"#,
            "maximumFractionDigits",
        ),
        (
            "en",
            r#"{"maximumFractionDigits": 100.5}"#,
            "maximumFractionDigits",
        ),
        ("en", r#"{"useGrouping": 1}"#, "useGrouping"),
        ("en", r#"{"useGrouping": []}"#, "useGrouping"),
        ("en", r#"{"style": null}"#, "style"),
        (
            "en",
            r#"{"signDisplay": "auto", "signDisplay": "never"}"#,
            "signDisplay",
        ),
    ] {
        let err = I18n::builder()
            .iso("ar", "ar-EG")
            .formats(&format!(r#"{{"{locale}": {{"x": {options}}}}}"#))
            .build()
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("format \"x\" of \"{locale}\": option \"{option}\" is not supported"),
            "{options}"
        );
    }
}

/// Given tags ICU resolves its own way, when they are a locale's `iso`, then they format as
/// `Intl` does: a variant alias that leads to a language without data (`hy-arevmda` → `hyw`)
/// formats as `en`, a region whose script ICU does not alias stays with the language (`az-IR`
/// is `az`, Latin digits), `-u-` extensions are not read.
#[test]
fn tags_resolve_like_icu() {
    let fmt = |tag: &str| {
        I18n::builder()
            .iso("t", tag)
            .locale("t")
            .build()
            .unwrap()
            .tn(-1234.5)
    };
    // Node 22: hy -1234,5; hy-arevmda → en -1,234.5; az-IR -1.234,5; bh → en
    assert_eq!(fmt("hy"), "-1234,5");
    assert_eq!(fmt("hy-arevmda"), "-1,234.5");
    assert_eq!(fmt("hy-AM-arevmda"), "-1,234.5");
    assert_eq!(fmt("hy-arevela"), "-1234,5");
    assert_eq!(fmt("az-IR"), "-1.234,5");
    assert_eq!(fmt("iw"), "\u{200e}-1,234.5");
    // `bh-US` → `bho` (not `modern`): number-locales-all.md, in a build with every locale
    assert_eq!(fmt("bh"), "-1,234.5");
}

/// Given locale codes that are no language tags, when numbers are formatted, then the
/// `iso` decides (`ua` → `uk-UA`, `ar` → `ar-EG` with Arabic digits), a code without one is
/// its own tag, and a tag `Intl` does not know formats as `en`.
#[test]
fn iso_picks_the_number_locale() {
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "ua", "locales": [{"code": "ua", "iso": "uk-UA"}, {"code": "ar", "iso": "ar-EG"},
                {"code": "zh-tw", "iso": "zh-TW"}]}"#,
        )
        .build()
        .unwrap();
    assert_eq!(i18n.tn(1234.5), "1\u{a0}234,5");
    assert_eq!(i18n.with_locale("ar").tn(-1234.5), "\u{61c}-١٬٢٣٤٫٥");
    assert_eq!(i18n.with_locale("zh-tw").tn(1e7), "10,000,000");
    assert_eq!(i18n.with_locale("de").tn(1234.5), "1.234,5");
    assert_eq!(i18n.with_locale("xx").tn(1234.5), "1,234.5");
    assert_eq!(i18n.with_locale("de_CH").tn(1234.5), "1'234.5");
    i18n.set_locale("ar").unwrap();
    assert_eq!(i18n.tn(0.5), "٠٫٥");
    assert_eq!(i18n.language_name("ua"), Some("українська"));
    assert_eq!(i18n.language_name("ar"), Some("العربية (مصر)"));
}

/// Given a message with `{number}`, when a `Text` gets a number, then it is formatted in
/// the text's locale (not escaped under `html`) and wins over a count; an `Arg::Number`
/// formats in the outer text's locale, `t!(.., @number = n)` and `tn!` work on the implicit
/// instance.
#[test]
fn numbers_in_texts_and_macros() {
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"total": "Total: {number} <{x}>", "items": "none | one | {count} items {number}"}"#,
        )
        .messages(
            "de",
            None,
            r#"{"total": "Summe: {number}", "sum": "{a} / {b}"}"#,
        )
        .formats(r#"{"de": {"eur": {"style": "currency", "currency": "EUR"}}}"#)
        .build()
        .unwrap();
    assert_eq!(
        i18n.text("total").number(1234.5).arg("x", "<"),
        "Total: 1,234.5 <<>"
    );
    assert_eq!(
        i18n.text("total").number(1234.5).arg("x", "<").html(),
        "Total: 1,234.5 <&lt;>"
    );
    // the number wins over the plural form: the whole text, `{count}` unfilled
    assert_eq!(
        i18n.text("items").count(3).number(2),
        "none | one | {count} items 2"
    );
    let de = i18n.with_locale("de");
    assert_eq!(
        de.text("total").number_as(1234.5, "eur"),
        "Summe: 1.234,50\u{a0}€"
    );
    let n = Arg::Number {
        value: 0.5.into(),
        format: None,
    };
    assert_eq!(de.text("sum").arg("a", n.clone()).arg("b", 2), "0,5 / 2");
    assert_eq!(
        t!(de => "sum", a = n, b = Arg::Number { value: 3.into(), format: Some("eur".into()) }),
        "0,5 / 3,00\u{a0}€"
    );
    scope().instance(i18n.clone()).locale("de").run(|| {
        assert_eq!(t!("total", @number = 1e6), "Summe: 1.000.000");
        assert_eq!(tn!(1234.5), "1.234,5");
        assert_eq!(tn!(1234.5, "eur"), "1.234,50\u{a0}€");
    });
    assert_eq!(tn!(i18n => 1234.5), "1,234.5");
}
