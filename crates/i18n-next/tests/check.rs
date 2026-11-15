//! Stories of `I18n::check()` (DESIGN 8b.8, 7.4, M3): Given/When/Then in the doc comment,
//! `// step n: action → check` in the body.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{CheckItem, I18n, LinkError};
use support::Locales;

fn s(v: &str) -> String {
    v.to_owned()
}

/// Given a catalog where every locale has every page and key with the same placeholders
/// and plurals shaped for their locale, when the test suite checks it, then it is clean.
#[test]
fn a_complete_catalog_is_clean() {
    // step 1: en (one, other) and ru (one, few, many), root and one page each
    let i18n = I18n::builder()
        .locale("en")
        .files(
            Locales::new()
                .file(
                    "en.json",
                    r#"{"hi": "Hi {name}", "apples": "no apples | one apple | {count} apples",
                        "menu": "=0 none | one item | {count} items", "n": 1, "on": true, "tree": {}}"#,
                )
                .file(
                    "ru.json",
                    r#"{"hi": "Привет, {name}", "apples": "нет | {count} яблоко | {count} яблока | {count} яблок",
                        "menu": "=0 нет | {count} пункт | {count} пункта | {count} пунктов", "n": 2, "on": false, "tree": {"x": "y"}}"#,
                )
                .file("pages/about/en.json", r#"{"title": "About"}"#)
                .file("pages/about/ru.json", r#"{"title": "О нас"}"#)
                .files(),
        )
        .build()
        .unwrap();

    // step 2: check → no findings, Display says so
    let report = i18n.check();
    assert!(report.is_clean(), "{report}");
    assert_eq!(report.to_string(), "catalog check: clean");
}

/// Given a catalog with each kind of mistake once (a page only English has, a key German
/// forgot, a renamed placeholder, plurals the locale reads by index, a broken link), when
/// the test suite checks it, then the report lists exactly those, sorted, and prints one
/// line each.
#[test]
fn every_kind_of_mistake_is_listed() {
    // step 1: en is the default locale of i18n.json; ru has rules with three categories
    let i18n = I18n::builder()
        .config_json(r#"{"defaultLocale": "en", "locales": [{"code": "en"}, {"code": "de"}, {"code": "ru"}]}"#)
        .linked_messages(true)
        .files(
            Locales::new()
                .file(
                    "en.json",
                    r#"{"hi": "Hi {name}", "bye": "Bye", "cars": "car | cars", "see": "See @:nope",
                        "apples": "no apples | one apple | {count} apples"}"#,
                )
                .file(
                    "de.json",
                    r#"{"hi": "Hallo {nam}", "cars": "kein Auto | ein Auto | {count} Autos", "see": "x",
                        "apples": "keine | ein Apfel | {count} Äpfel"}"#,
                )
                .file(
                    "ru.json",
                    r#"{"hi": "Привет {name}", "bye": "Пока", "cars": "машина | машины", "see": "y",
                        "apples": "{count} нет | {count} яблоко | {count} яблока | {count} яблок"}"#,
                )
                .file("pages/about/en.json", r#"{"title": "About"}"#)
                .file("pages/about/ru.json", r#"{"title": "О нас"}"#)
                .files(),
        )
        .build()
        .unwrap();

    // step 2: check → one item per mistake, in the report's order
    let report = i18n.check();
    let en_categories = vec![s("one"), s("other")];
    let ru_categories = vec![s("one"), s("few"), s("many")];
    assert_eq!(
        report.items,
        vec![
            CheckItem::MissingPage {
                locale: s("de"),
                page: s("about")
            },
            CheckItem::MissingKey {
                locale: s("de"),
                page: s("index"),
                key: s("bye")
            },
            CheckItem::Placeholders {
                locale: s("de"),
                page: s("index"),
                key: s("hi"),
                expected: vec![s("name")],
                found: vec![s("nam")],
            },
            CheckItem::PluralForms {
                locale: s("en"),
                page: s("index"),
                key: s("cars"),
                forms: 2,
                expected: vec![3],
                categories: en_categories,
            },
            CheckItem::PluralForms {
                locale: s("ru"),
                page: s("index"),
                key: s("cars"),
                forms: 2,
                expected: vec![4],
                categories: ru_categories.clone(),
            },
            CheckItem::PluralShape {
                locale: s("ru"),
                page: s("index"),
                key: s("apples"),
                categories: ru_categories,
            },
            CheckItem::BrokenLink {
                locale: s("en"),
                page: s("index"),
                key: s("see"),
                link: s("@:nope"),
                reason: LinkError::Missing,
            },
        ],
        "{report}"
    );

    // step 3: Display → a header and one line per item
    let text = report.to_string();
    assert!(
        text.starts_with("catalog check: 7 problem(s)\n  - de: no file for page \"about\""),
        "{text}"
    );
    assert!(
        text.contains("de/index: \"hi\" has {nam}, the default locale {name}"),
        "{text}"
    );
    assert!(
        text.contains("en/index: \"cars\" has 2 plural forms, expected 3 (zero | one | other)"),
        "{text}"
    );
    assert!(
        text.contains("en/index: \"see\" left as written: @:nope names no message"),
        "{text}"
    );
}

/// Given a disabled locale with gaps and a locale whose `iso` has no CLDR rules, when the
/// catalog is checked, then the disabled locale is skipped and the index-rule locale takes
/// any count of forms.
#[test]
fn disabled_locales_and_the_index_rule() {
    // step 1: es is disabled, xx has an iso Intl rejects
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "locales": [{"code": "en"}, {"code": "es", "disabled": true},
                {"code": "xx", "iso": "en_US"}]}"#,
        )
        .messages(
            "en",
            None,
            r#"{"a": "A", "p": "none | one | {count} many"}"#,
        )
        .messages("es", None, r#"{"p": "x | y"}"#)
        .messages("xx", None, r#"{"a": "A", "p": "x | y"}"#)
        .build()
        .unwrap();

    // step 2: check → clean: es is not looked at, xx reads `p` by index
    assert!(i18n.check().is_clean(), "{}", i18n.check());
}

/// Given a library page whose message links a key only the app's lazy root has, when the
/// catalog is checked, then the broken link says its target may be in a lazy file (links of
/// eager chunks resolve at `build`, when lazy files are not read), not "names no message".
#[test]
fn a_link_into_a_lazy_file_says_so() {
    // step 1: an eager library page, a lazy app root
    let i18n = I18n::builder()
        .linked_messages(true)
        .extend([("pages/lib/en.json", br#"{"x": "Lib @:app.t"}"#.as_slice())])
        .lazy_files([("en.json", br#"{"app": {"t": "T"}}"#.as_slice())])
        .build()
        .unwrap();
    // step 2: check → the reason names the lazy files
    let report = i18n.check();
    assert_eq!(
        report.items,
        vec![CheckItem::BrokenLink {
            locale: s("en"),
            page: s("lib"),
            key: s("x"),
            link: s("@:app.t"),
            reason: LinkError::Lazy,
        }],
        "{report}"
    );
    assert!(
        report
            .to_string()
            .contains("@:app.t names no message read at build (its view has lazy files)"),
        "{report}"
    );
}
