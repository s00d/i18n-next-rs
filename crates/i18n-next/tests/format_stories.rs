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
            format!("number format \"x\" of \"en\": option \"{option}\" is not supported"),
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
