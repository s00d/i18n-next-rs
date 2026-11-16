//! Stories of the crate (DESIGN 7.4): one event stream per test, Given/When/Then in the
//! doc comment, `// step n: action → check` in the body.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{Args, Error, I18n, LocaleInfo, scope, t};
use support::{Clock, Guard, Locales};

fn app_locales() -> Locales {
    Locales::new()
        .file(
            "en.json",
            r#"{"nav": {"home": "Home", "settings": "Settings"}, "greeting": "Hello, {name}!"}"#,
        )
        .file("de.json", "\u{feff}{\"nav\": {\"home\": \"Startseite\"}}")
        .file(
            "pages/settings/en.json",
            r#"{"title": "Settings", "nav": {"home": "Back"}}"#,
        )
        .file("pages/settings/de.json", r#"{"title": "Einstellungen"}"#)
        .file("pages/user/profile/en.json", r#"{"title": "Profile"}"#)
        .file("notes/readme.json", r#"{"ignored": true}"#)
        .file("pages/stray.json", "{ not even json")
}

/// Given a locales directory with root files, page folders (one nested), a BOM and files
/// outside the layout, when the app builds from `dir`, then root and page layers resolve
/// in order and the out-of-layout files are never read.
#[test]
fn dir_with_pages() {
    // step 1: build from a real directory → no error, the broken stray file is ignored
    let dir = app_locales().dir("dir_with_pages");
    let i18n = I18n::builder()
        .dir(&dir)
        .locale("de")
        .fallback("en")
        .build()
        .unwrap();

    // step 2: pages come from folders, nested folders join with '-'
    assert_eq!(i18n.pages(), ["settings", "user-profile"]);

    // step 3: page → root → fallback page → fallback root
    let settings = i18n.page("settings");
    assert_eq!(settings.t("title"), "Einstellungen");
    assert_eq!(settings.t("nav.home"), "Startseite"); // de root beats the en page
    assert_eq!(settings.t("nav.settings"), "Settings");
    assert_eq!(i18n.page("user-profile").t("title"), "Profile");

    // step 4: the locales the files define
    let codes: Vec<_> = i18n.locales().into_iter().map(|l| l.code).collect();
    assert_eq!(codes, ["de", "en"]);
}

/// Given embedded files (the Android and browser path) with nested pages and Windows
/// separators, when the app builds from `files()`, then it reads exactly like `dir`.
#[test]
fn files_like_android_assets() {
    // step 1: files with mixed separators
    let i18n = I18n::builder()
        .files([
            ("en.json", br#"{"a": "root"}"#.as_slice()),
            ("pages\\user\\profile\\en.json", br#"{"a": "nested page"}"#),
            ("pages/user-profile/en.json", br#"{"b": "dash page"}"#),
        ])
        .build()
        .unwrap();

    // step 2: two folders, one page, merged in path order
    assert_eq!(i18n.pages(), ["user-profile"]);
    let page = i18n.page("user-profile");
    assert_eq!(page.t("a"), "nested page");
    assert_eq!(page.t("b"), "dash page");
    assert_eq!(i18n.t("a"), "root");
}

/// Given a broken locale file, when the app builds, then it fails with the file's path
/// (JS would log and skip; Rust refuses half a catalog). The same for `dir`.
#[test]
fn broken_file_is_an_error_with_its_path() {
    // step 1: files() → Json error naming the file
    let err = I18n::builder()
        .files([
            ("en.json", br#"{"ok": "x"}"#.as_slice()),
            ("pages/about/de.json", br#"{"a": "b",}"#),
        ])
        .build()
        .unwrap_err();
    match &err {
        Error::Json { path, offset, .. } => {
            assert_eq!(path, "pages/about/de.json");
            assert_eq!(*offset, 10);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        err.to_string()
            .starts_with("pages/about/de.json: invalid JSON at byte 10")
    );

    // step 2: dir → the full path
    let dir = Locales::new().file("de.json", "{").dir("broken_file");
    match I18n::builder().dir(&dir).build().unwrap_err() {
        Error::Json { path, .. } => assert!(path.ends_with("de.json"), "{path}"),
        other => panic!("{other:?}"),
    }

    // step 3: a missing directory → Io with its path
    let err = I18n::builder().dir(dir.join("nope")).build().unwrap_err();
    assert!(
        matches!(&err, Error::Io { path, .. } if path.ends_with("nope")),
        "{err:?}"
    );
    assert!(std::error::Error::source(&err).is_some());

    // step 4: bad `messages` text names its chunk
    let err = I18n::builder()
        .messages("en", Some("cart"), "[")
        .build()
        .unwrap_err();
    assert!(
        matches!(&err, Error::Json { path, .. } if path == "messages:en/cart"),
        "{err:?}"
    );
}

/// Given valid JSON that is not an object (`null`, an array), when the app builds, then
/// it fails with `NotObject` and the path.
#[test]
fn non_object_file_is_an_error() {
    // step 1: a page file holding `null`
    let err = I18n::builder()
        .files([("pages/about/fr.json", b"null")])
        .build()
        .unwrap_err();
    assert!(matches!(&err, Error::NotObject { path } if path == "pages/about/fr.json"));
    assert_eq!(
        err.to_string(),
        "pages/about/fr.json: expected a JSON object"
    );

    // step 2: add_messages with an array → nothing changes
    let i18n = I18n::builder()
        .messages("en", None, r#"{"a": "A"}"#)
        .build()
        .unwrap();
    let err = i18n.add_messages("en", None, "[1]").unwrap_err();
    assert!(matches!(&err, Error::NotObject { path } if path == "messages:en/index"));
    assert_eq!(i18n.version(), 0);
    assert_eq!(i18n.t("a"), "A");
}

/// Given a catalog with `en` and `de`, when the front end asks for names it does not
/// know (`../../etc/passwd`, `CON`, empty, unicode, `__proto__`), then `set_locale`
/// refuses, views read nothing of their own, and nothing is ever opened as a path.
#[test]
fn unknown_and_bad_names() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages("en", None, r#"{"a": "A"}"#)
        .messages("de", None, r#"{"a": "Ä"}"#)
        .missing(guard.missing())
        .build()
        .unwrap();
    for name in ["../../etc/passwd", "CON", "", "ünïcödé", "__proto__", "xx"] {
        // step 1: set_locale → UnknownLocale, the active locale stays
        let err = i18n.set_locale(name).unwrap_err();
        assert!(
            matches!(&err, Error::UnknownLocale(c) if c == name),
            "{err:?}"
        );
        assert_eq!(&*i18n.locale(), "de");
        // step 2: with_locale → the global fallback answers, `has` is false
        let view = i18n.with_locale(name);
        assert_eq!(view.t("a"), "A");
        assert!(!view.has("a"));
        assert_eq!(&*view.locale(), name);
        // step 3: a page of that name → just the root
        assert_eq!(i18n.page(name).t("a"), "Ä");
    }
    // step 4: a miss in an unknown locale reports that locale
    guard.allow("xx|index|nope");
    assert_eq!(i18n.with_locale("xx").t("nope"), "nope");
    // step 5: chunk names that can never be valid
    assert!(matches!(
        i18n.add_messages("", None, "{}"),
        Err(Error::UnknownLocale(_))
    ));
    assert!(matches!(
        i18n.add_messages("en", Some("__proto__"), "{}"),
        Err(Error::UnknownPage(_))
    ));
    assert_eq!(i18n.version(), 0);
}

/// Given `i18n.json` with a disabled locale, when the app lists locales and switches,
/// then the disabled one is hidden, refused, and cannot be the start locale; a view of it
/// still reads its own messages.
#[test]
fn disabled_locale() {
    let config = r#"{"locales": [{"code": "en"}, {"code": "es", "disabled": true}]}"#;
    let files = [
        ("en.json", br#"{"a": "A"}"#.as_slice()),
        ("es.json", br#"{"a": "A-es"}"#),
    ];

    // step 1: hidden from locales()
    let i18n = I18n::builder()
        .config_json(config)
        .locale("en")
        .files(files)
        .build()
        .unwrap();
    let codes: Vec<_> = i18n.locales().into_iter().map(|l| l.code).collect();
    assert_eq!(codes, ["en"]);

    // step 2: set_locale refuses it; with_locale still reads its chain, as Nuxt's
    // `buildFallbackLocaleChain` does for a configured locale (`es` → `[es, en]`)
    assert!(matches!(i18n.set_locale("es"), Err(Error::DisabledLocale(c)) if c == "es"));
    assert_eq!(i18n.with_locale("es").try_t("a").as_deref(), Some("A-es"));
    assert!(i18n.with_locale("es").has("a"));
    assert_eq!(i18n.version(), 0);

    // step 3: it cannot be the start locale either
    let err = I18n::builder()
        .config_json(config)
        .locale("es")
        .files(files)
        .build()
        .unwrap_err();
    assert_eq!(err.to_string(), "locale \"es\" is disabled");
}

/// Given `pages(false)` (`disablePageLocales`), when page files and a root file define
/// the same key, then everything lands in the root and the page file wins in any order.
#[test]
fn pages_false_folds_into_root() {
    // step 1: `zz.json` sorts after `pages/…`, `aa.json` before: the page file wins both times
    let i18n = I18n::builder()
        .pages(false)
        .locale("zz")
        .files([
            ("aa.json", br#"{"k": "root"}"#.as_slice()),
            ("pages/x/aa.json", br#"{"k": "page", "only": "page"}"#),
            ("pages/x/zz.json", br#"{"k": "page"}"#),
            ("zz.json", br#"{"k": "root", "r": "root"}"#),
        ])
        .build()
        .unwrap();
    assert_eq!(i18n.pages(), Vec::<String>::new());
    assert_eq!(i18n.t("k"), "page");
    assert_eq!(i18n.t("r"), "root");
    assert_eq!(i18n.with_locale("aa").t("k"), "page");
    assert_eq!(i18n.with_locale("aa").t("only"), "page");
}

/// Given `page_alias("about-us", "about")` (`routesLocaleLinks`), when the app reads the
/// alias, then it reads the target's chunk; an alias to nothing fails the build.
#[test]
fn page_alias() {
    let i18n = I18n::builder()
        .messages("en", Some("about"), r#"{"title": "About"}"#)
        .page_alias("about-us", "about")
        .build()
        .unwrap();
    // step 1: the alias reads the target page, and is a known page itself
    assert_eq!(i18n.page("about-us").t("title"), "About");
    assert_eq!(i18n.pages(), ["about", "about-us"]);
    // step 2: writing through the alias writes the target
    i18n.add_messages("en", Some("about-us"), r#"{"sub": "Us"}"#)
        .unwrap();
    assert_eq!(i18n.page("about").t("sub"), "Us");
    // step 3: an alias to a page nothing defines
    let err = I18n::builder()
        .page_alias("a", "nowhere")
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::UnknownPage(p) if p == "nowhere"));
}

/// Given `i18n.json` with a default locale, a global fallback, per-locale fallbacks, iso
/// tags, `dir` and display names, when the app builds from it, then those drive the
/// chain and `locales()`, and builder calls override the file.
#[test]
fn config_drives_locales_and_fallbacks() {
    let dir = Locales::new()
        .file(
            "i18n.json",
            r#"{
              "defaultLocale": "de-at", "fallbackLocale": "en",
              "locales": [
                {"code": "en", "iso": "en-US", "displayName": "English"},
                {"code": "de-at", "iso": "de-AT", "fallbackLocale": "de", "unknownField": [1, {"x": 2}]},
                {"code": "ar", "dir": "rtl"}
              ],
              "numberFormats": {"en": {"currency": {"style": "currency", "currency": "USD"}}},
              "nuxtOnly": {"strategy": "prefix"}
            }"#,
        )
        .file("locales/en.json", r#"{"a": "en", "b": "en", "c": "en"}"#)
        .file("locales/de.json", r#"{"a": "de", "b": "de"}"#)
        .file("locales/de-at.json", r#"{"a": "at"}"#)
        .dir("config_drives");

    // step 1: the start locale and its chain de-at → de → en come from the file
    let i18n = I18n::builder()
        .config(dir.join("i18n.json"))
        .dir(dir.join("locales"))
        .iso("ar", "ar-EG")
        .build()
        .unwrap();
    assert_eq!(&*i18n.locale(), "de-at");
    assert_eq!([i18n.t("a"), i18n.t("b"), i18n.t("c")], ["at", "de", "en"]);

    // step 2: locales() in file order, then the rest; builder iso wins
    let infos = i18n.locales();
    let LocaleInfo {
        code,
        iso,
        dir: text_dir,
        display_name,
        ..
    } = infos[0].clone();
    assert_eq!(
        (
            code.as_str(),
            iso.as_deref(),
            text_dir,
            display_name.as_deref()
        ),
        ("en", Some("en-US"), None, Some("English"))
    );
    assert_eq!(infos[1].iso.as_deref(), Some("de-AT"));
    assert_eq!(
        (
            infos[2].code.as_str(),
            infos[2].dir.as_deref(),
            infos[2].iso.as_deref()
        ),
        ("ar", Some("rtl"), Some("ar-EG"))
    );
    assert_eq!(infos[3].code, "de");
    assert_eq!(infos.len(), 4);

    // step 3: builder fallback calls override the file
    let i18n = I18n::builder()
        .config(dir.join("i18n.json"))
        .dir(dir.join("locales"))
        .fallback_for("de-at", "en")
        .locale("de-at")
        .build()
        .unwrap();
    assert_eq!(i18n.t("b"), "en");

    // step 4: a malformed config → Config error with the field; a missing one → Io
    let err = I18n::builder()
        .config_json(r#"{"locales": [{"iso": "x"}]}"#)
        .build()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "i18n.json: locales[].code is missing or empty"
    );
    let err = I18n::builder()
        .config_json(r#"{"defaultLocale": 1}"#)
        .build()
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Config {
            reason: "defaultLocale must be a string",
            ..
        }
    ));
    let err = I18n::builder()
        .config_json(r#"{"locales": [{"code": "x"},]}"#)
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Json { .. }), "{err:?}");
    let err = I18n::builder().config_json("[]").build().unwrap_err();
    assert!(matches!(err, Error::NotObject { .. }));
    assert!(matches!(
        I18n::builder().config(dir.join("none.json")).build(),
        Err(Error::Io { .. })
    ));
}

/// Given a `missing` hook recording into a guard, when the app looks up hits, subtrees,
/// misses, defaults and the empty key through every entry point, then the hook fires
/// exactly for misses of `t` / `t_with`, with (locale, key, page).
#[test]
fn missing_handler_fires_exactly_on_misses() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"nav": {"home": "Home"}, "n": 5, "greet": "Hi {name}"}"#,
        )
        .messages("de", Some("cart"), r#"{"title": "Korb"}"#)
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 1: hits, a subtree, a number, the empty key → no call
    assert_eq!(i18n.t("nav.home"), "Home");
    assert_eq!(i18n.t("nav"), "nav");
    assert_eq!(i18n.t("n"), "5");
    assert_eq!(i18n.t_with("n", &Args::new().str("name", "x")), "5");
    assert_eq!(i18n.t(""), "");
    assert_eq!(
        i18n.t_with("greet", &Args::new().str("name", "Ann")),
        "Hi Ann"
    );
    assert_eq!(guard.seen(), Vec::<String>::new());

    // step 2: try_t / get / has never report
    assert_eq!(i18n.try_t("nope"), None);
    assert_eq!(i18n.get("nope"), None);
    assert!(!i18n.has("nope"));
    assert_eq!(guard.seen(), Vec::<String>::new());

    // step 3: misses report locale, key and page; defaults come back interpolated
    guard.allow("de|index|nope");
    guard.allow("de|cart|nope.cart");
    guard.allow("en|index|gone");
    assert_eq!(i18n.t("nope"), "nope");
    assert_eq!(
        i18n.page("cart")
            .t_with("nope.cart", &Args::new().int("n", 2).or("{n} items")),
        "2 items"
    );
    assert_eq!(
        i18n.with_locale("en").t_with("gone", &Args::new().or("")),
        "gone"
    );
    assert_eq!(
        guard.seen(),
        ["de|index|nope", "de|cart|nope.cart", "en|index|gone"]
    );
}

/// Given a running catalog, when the app adds messages and switches locales, then every
/// change is a new version, merges deep, and a value handed out earlier stays as it was.
#[test]
fn add_messages_and_versions() {
    let i18n = I18n::builder()
        .locale("en")
        .messages(
            "en",
            None,
            r#"{"nav": {"home": "Home", "about": "About"}, "list": ["a", "b"]}"#,
        )
        .build()
        .unwrap();
    let old = i18n.get("nav.home").unwrap();

    // step 1: objects merge, arrays and scalars replace
    i18n.add_messages("en", None, r#"{"nav": {"home": "Start"}, "list": ["z"]}"#)
        .unwrap();
    assert_eq!(i18n.version(), 1);
    assert_eq!(
        [i18n.t("nav.home"), i18n.t("nav.about"), i18n.t("list.0")],
        ["Start", "About", "z"]
    );
    assert_eq!(i18n.try_t("list.1"), None);
    assert_eq!(&*old, "Home");

    // step 2: a new locale becomes known and switchable
    i18n.add_messages("fr", Some("x"), r#"{"t": "Titre"}"#)
        .unwrap();
    i18n.set_locale("fr").unwrap();
    assert_eq!(i18n.version(), 3);
    assert_eq!(i18n.page("x").t("t"), "Titre");

    // step 3: the same locale again is no change
    i18n.set_locale("fr").unwrap();
    assert_eq!(i18n.version(), 3);
    assert_eq!(
        format!("{i18n:?}"),
        "I18n { locale: \"fr\", version: 3, loads: 0, .. }"
    );
}

/// Given the support simulator, when a test drives it, then files keep their paths,
/// time moves only on `advance`, and an allowed event does not fail the guard.
#[test]
fn simulator_skeleton() {
    // step 1: a locale tree → files come back in path order
    let locales = Locales::new()
        .file("pages/settings/en.json", r#"{"title":"Settings"}"#)
        .file("en.json", r#"{"hello":"Hello"}"#);
    let paths: Vec<_> = locales.files().map(|(p, _)| p).collect();
    assert_eq!(paths, ["en.json", "pages/settings/en.json"]);

    // step 2: the virtual clock moves only when told
    let clock = Clock::at(1_000);
    clock.advance(250);
    assert_eq!(clock.now_ms(), 1_250);

    // step 3: an expected miss is allowed, the guard stays quiet on drop
    let guard = Guard::new();
    guard.allow("nav.unknown");
    guard.record("nav.unknown");
    drop(guard);

    // step 4: the crate is linked and reports its version
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

/// Given a guard, when an unexpected miss is recorded, then the test fails on drop.
#[test]
#[should_panic(expected = "unexpected events")]
fn guard_fails_on_unexpected_event() {
    let guard = Guard::new();
    guard.record("nav.typo");
}

fn codes(i18n: &I18n) -> Vec<String> {
    i18n.locales().into_iter().map(|l| l.code).collect()
}

/// Given locales that only builder calls name (both ends of a `fallback_for` link, the
/// global fallback, an `iso` code, the implicit default `en`), when the app views them,
/// switches to them and lists locales, then each is a known locale reading its own chain.
#[test]
fn locales_named_only_by_the_builder_are_known() {
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .fallback_for("de-ch", "de")
        .fallback_for("x-src", "x-dst")
        .iso("ua", "uk-UA")
        .messages("en", None, r#"{"a": "en", "b": "en"}"#)
        .messages("de", None, r#"{"a": "de"}"#)
        .build()
        .unwrap();

    // step 1: a link key with no file → a view reads de, then the global fallback
    let ch = i18n.with_locale("de-ch");
    assert_eq!([ch.t("a"), ch.t("b")], ["de", "en"]);
    assert!(!ch.has("a")); // nothing of its own

    // step 2: set_locale accepts it; the active chain is de-ch → de → en
    i18n.set_locale("de-ch").unwrap();
    assert_eq!(&*i18n.locale(), "de-ch");
    assert_eq!([i18n.t("a"), i18n.t("b")], ["de", "en"]);
    assert!(!i18n.has("a"));

    // step 3: a link target, an iso code → known, reading the global fallback
    for code in ["x-dst", "x-src", "ua"] {
        i18n.set_locale(code).unwrap();
        assert_eq!(i18n.t("a"), "en");
    }

    // step 4: locales() lists all of them, by code; the iso tag stays
    assert_eq!(codes(&i18n), ["de", "de-ch", "en", "ua", "x-dst", "x-src"]);
    let ua = i18n.locales().into_iter().find(|l| l.code == "ua").unwrap();
    assert_eq!(ua.iso.as_deref(), Some("uk-UA"));

    // step 5: the global fallback with no file of its own is known too
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages("de", None, r#"{"a": "de"}"#)
        .build()
        .unwrap();
    assert_eq!(codes(&i18n), ["de", "en"]);
    i18n.set_locale("en").unwrap();
    assert_eq!(i18n.try_t("a"), None);

    // step 6: the implicit default `en`, active with no file → known and switchable back
    let i18n = I18n::builder()
        .messages("de", None, r#"{"a": "de"}"#)
        .build()
        .unwrap();
    assert_eq!(&*i18n.locale(), "en");
    assert_eq!(codes(&i18n), ["de", "en"]);
    i18n.set_locale("de").unwrap();
    i18n.set_locale("en").unwrap();
    assert_eq!(i18n.version(), 2);

    // step 7: an `i18n.json` fallback target no entry lists → known with its chain
    let i18n = I18n::builder()
        .config_json(r#"{"locales": [{"code": "de-ch", "fallbackLocale": "de"}]}"#)
        .locale("de-ch")
        .messages("de", None, r#"{"a": "de"}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.t("a"), "de");
    assert_eq!(codes(&i18n), ["de-ch", "de"]);
}

/// Given `pages(false)` and a `page_alias`, when the app builds and reads the alias, then
/// the build succeeds and the alias reads the root, like every page name when pages fold.
#[test]
fn page_alias_with_pages_folded() {
    // step 1: the alias target has no page chunk (pages fold) → no UnknownPage
    let i18n = I18n::builder()
        .pages(false)
        .page_alias("about-us", "about")
        .files([
            ("en.json", br#"{"k": "root"}"#.as_slice()),
            ("pages/about/en.json", br#"{"t": "About"}"#),
        ])
        .build()
        .unwrap();
    // step 2: the alias and its target read the root, which holds the folded page
    assert_eq!(i18n.page("about-us").t("t"), "About");
    assert_eq!(i18n.page("about").t("k"), "root");
}

/// Given `pages(false)` and a root file that sorts after two page files of its locale
/// (`ru.json` after `pages/a/ru.json`, `pages/b/ru.json`), when the app builds, then the
/// files merge in JS path order and the root goes under what the pages merged, exactly
/// as `@i18n-micro/utils` does (values from the JS loader).
#[test]
fn pages_false_merges_in_js_order() {
    // step 1: build the auditor's `nopages_ru_triple` tree
    let i18n = I18n::builder()
        .pages(false)
        .locale("ru")
        .files([
            (
                "ru.json",
                br#"{"k":{"y":"R"},"m":{"y":"R"},"same":"root"}"#.as_slice(),
            ),
            ("pages/a/ru.json", br#"{"k":"s","m":null,"same":"pa"}"#),
            ("pages/b/ru.json", br#"{"k":{"x":"B"},"m":{"x":"B"}}"#),
        ])
        .build()
        .unwrap();
    // step 2: page b over page a, then the root under both
    assert_eq!(i18n.try_t("k"), None);
    assert!(i18n.has("k"));
    assert_eq!(
        [i18n.t("k.x"), i18n.t("k.y"), i18n.t("m.x"), i18n.t("m.y")],
        ["B", "R", "B", "R"]
    );
    assert_eq!(i18n.t("same"), "pa");
}

/// Given fallback links that form a cycle, through the builder and through `i18n.json`,
/// when the app looks up a key nothing has, then the lookup ends with a miss report and
/// keys on the cycle still resolve.
#[test]
fn fallback_cycles_end() {
    let guard = Guard::new();
    // step 1: a → b → a with b as the global fallback, plus a self-link
    let i18n = I18n::builder()
        .locale("a")
        .fallback("b")
        .fallback_for("a", "b")
        .fallback_for("b", "a")
        .fallback_for("c", "c")
        .messages("b", None, r#"{"x": "B"}"#)
        .missing(guard.missing())
        .build()
        .unwrap();
    assert_eq!(i18n.t("x"), "B");
    guard.allow("a|index|nope");
    guard.allow("c|index|nope");
    assert_eq!(i18n.t("nope"), "nope");
    assert_eq!(i18n.with_locale("c").t("nope"), "nope");
    assert_eq!(i18n.with_locale("c").t("x"), "B");

    // step 2: the same cycle from `i18n.json`
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "a", "fallbackLocale": "a",
                "locales": [{"code": "a", "fallbackLocale": "b"},
                            {"code": "b", "fallbackLocale": "a"}]}"#,
        )
        .messages("a", None, r#"{"y": "A"}"#)
        .missing(guard.missing())
        .build()
        .unwrap();
    guard.allow("b|index|nope");
    assert_eq!(i18n.with_locale("b").t("y"), "A");
    assert_eq!(i18n.with_locale("b").t("nope"), "nope");
    assert_eq!(
        guard.seen(),
        ["a|index|nope", "c|index|nope", "b|index|nope"]
    );
}

/// Given a global fallback that has a `fallback_for` link of its own, when the app reads a
/// key only that link's target has, then it misses: the global fallback is the weakest
/// step, taken alone (`utils` `buildFallbackLocaleChain`); the fallback itself still
/// follows its link when it is the active locale.
#[test]
fn global_fallback_is_the_last_step_alone() {
    let i18n = I18n::builder()
        .locale("de-at")
        .fallback("en")
        .fallback_for("de-at", "de")
        .fallback_for("en", "fr")
        .messages("fr", None, r#"{"only_fr": "fr"}"#)
        .messages("en", None, r#"{"a": "en"}"#)
        .messages("de", None, r#"{"b": "de"}"#)
        .build()
        .unwrap();
    // step 1: de-at → de → en, never en → fr
    assert_eq!([i18n.t("a"), i18n.t("b")], ["en", "de"]);
    assert_eq!(i18n.try_t("only_fr"), None);
    // step 2: en active → en → fr
    i18n.set_locale("en").unwrap();
    assert_eq!(i18n.t("only_fr"), "fr");
}

/// Given an `i18n.json` whose global fallback `en` links to `fr`, a disabled `de` with
/// messages, `pt → zz` where `zz` is in no `locales[]` entry, when the app reads through
/// views of an unknown, a disabled and a linked locale, then each reads exactly the chain
/// `buildFallbackLocaleChain` gives (`qq` → `[en]`, `de` → `[de, en]`, `pt` → `[pt, en]`).
#[test]
fn unknown_and_disabled_locales_read_the_js_chain() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
                {"code": "en", "fallbackLocale": "fr"}, {"code": "fr"},
                {"code": "de", "disabled": true}, {"code": "pt", "fallbackLocale": "zz"}]}"#,
        )
        .messages("en", None, r#"{"e": "E"}"#)
        .messages("fr", None, r#"{"z": "fr-z"}"#)
        .messages("de", None, r#"{"d": "D"}"#)
        .missing(guard.missing())
        .build()
        .unwrap();
    // step 1: the active en follows its own link → en, fr
    assert_eq!([i18n.t("e"), i18n.t("z")], ["E", "fr-z"]);
    // step 2: unknown qq → the global fallback alone, never en → fr
    guard.allow("qq|index|z");
    assert_eq!(i18n.with_locale("qq").t("e"), "E");
    assert_eq!(i18n.with_locale("qq").t("z"), "z");
    assert!(!i18n.with_locale("qq").has("e"));
    // step 3: disabled de → its own messages, then en; set_locale still refuses it
    guard.allow("de|index|z");
    assert_eq!(i18n.with_locale("de").t("d"), "D");
    assert_eq!(i18n.with_locale("de").t("e"), "E");
    assert_eq!(i18n.with_locale("de").t("z"), "z");
    assert!(i18n.with_locale("de").has("d"));
    assert!(matches!(
        i18n.set_locale("de"),
        Err(Error::DisabledLocale(_))
    ));
    // step 4: fr, a known locale, ends with en alone → fr, en (not en → fr again)
    assert_eq!(i18n.with_locale("fr").t("e"), "E");
    // step 5: pt → zz with no zz messages: zz adds no layer, as JS stops there → pt, en
    assert_eq!(i18n.with_locale("pt").t("e"), "E");
    i18n.set_locale("zz").unwrap();
    guard.allow("zz|index|z");
    assert_eq!([i18n.t("e"), i18n.t("z")], ["E", "z"]);
    assert_eq!(guard.seen(), ["qq|index|z", "de|index|z", "zz|index|z"]);
}

/// Given documents whose dotted paths would multiply their size in memory (a deep prefix
/// of long keys over many leaves), when the app loads them, then the build fails at
/// once with a `Json` error, before storing anything large; a path at the limit loads.
#[test]
fn oversized_key_paths_are_refused() {
    let json_kind = |json: &str| match I18n::builder().messages("en", None, json).build() {
        Err(Error::Json { kind, .. }) => kind,
        other => panic!("{other:?}"),
    };
    // step 1: the audit's 137 KB document (it took 2.2 GB) → refused at the first long path
    let key = "k".repeat(1000);
    let mut deep = String::new();
    for _ in 0..100 {
        deep.push_str(&format!("{{\"{key}\":"));
    }
    deep.push('[');
    deep.push_str(&vec!["0"; 20_000].join(","));
    deep.push(']');
    deep.push_str(&"}".repeat(100));
    assert_eq!(json_kind(&deep), "key path longer than 4096 bytes");

    // step 2: every path under the limit, but together far more than the document
    let prefix = format!("{{\"{}\":{{\"{}\":", "p".repeat(2000), "q".repeat(2000));
    let leaves = vec!["0"; 1_000].join(",");
    let wide = format!("{prefix}[{leaves}]}}}}");
    assert_eq!(json_kind(&wide), "key paths too long for the document size");

    // step 3: a key of exactly 4096 bytes loads; one byte more does not
    let at_limit = "a".repeat(4096);
    let i18n = I18n::builder()
        .messages("en", None, &format!("{{\"{at_limit}\": \"ok\"}}"))
        .build()
        .unwrap();
    assert_eq!(i18n.t(&at_limit), "ok");
    let over = format!("{{\"{at_limit}b\": \"no\"}}");
    assert_eq!(json_kind(&over), "key path longer than 4096 bytes");
    // `add_messages` checks the same, and changes nothing
    assert!(matches!(
        i18n.add_messages("en", None, &over),
        Err(Error::Json { path, .. }) if path == "messages:en/index"
    ));
    assert_eq!(i18n.version(), 0);
}

/// Given duplicate keys and patches that replace containers (empty and not), when the
/// app loads and patches, then a replaced subtree is gone and a merged object keeps the
/// children of both; thousands of duplicate empty objects load like any document.
#[test]
fn replaced_containers_drop_their_subtree() {
    // step 1: duplicates in one document → the last value wins, its subtree only
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"a": {"b": "x"}, "a": {}, "c": {}, "c": {"d": "y"}, "e": [1], "e": []}"#,
        )
        .build()
        .unwrap();
    assert_eq!(i18n.try_t("a.b"), None);
    assert!(i18n.has("a"));
    assert_eq!(i18n.t("c.d"), "y");
    assert_eq!(i18n.try_t("e.0"), None);

    // step 2: an empty object merged over a full one keeps the children…
    i18n.add_messages("en", None, r#"{"c": {}}"#).unwrap();
    assert_eq!(i18n.t("c.d"), "y");
    // …so a later scalar over it still drops them
    i18n.add_messages("en", None, r#"{"c": "flat"}"#).unwrap();
    assert_eq!(i18n.t("c"), "flat");
    assert_eq!(i18n.try_t("c.d"), None);
    // an empty object, then children merged into it, then replaced
    i18n.add_messages("en", None, r#"{"a": {"z": "1"}}"#)
        .unwrap();
    assert_eq!(i18n.t("a.z"), "1");
    i18n.add_messages("en", None, r#"{"a": []}"#).unwrap();
    assert_eq!(i18n.try_t("a.z"), None);

    // step 3: 5k keys and 20k duplicate empty objects (the audit's quadratic case)
    let mut doc: String = (0..5_000).map(|i| format!("\"k{i}\":\"v\",")).collect();
    doc.push_str(&"\"x\":{},".repeat(20_000));
    let i18n = I18n::builder()
        .messages("en", None, &format!("{{{doc}\"end\":\"e\"}}"))
        .build()
        .unwrap();
    assert_eq!([i18n.t("k4999"), i18n.t("end")], ["v", "e"]);
    assert!(i18n.has("x"));
}

/// Given a default text whose placeholders hold JS whitespace (space, NBSP, BOM) and a
/// zero-width space, and arguments named exactly so, when a lookup misses, then only the
/// U+200B one is a placeholder: JS `\s` covers the rest, so `{a b}` stays as written
/// (`@i18n-micro/core` `interpolate` gives `[{a b}|{a\u{a0}b}|{a\u{feff}b}|Z]`), the same
/// as for a found message.
#[test]
fn default_text_placeholders_follow_js_whitespace() {
    let guard = Guard::new();
    let tpl = "[{a b}|{a\u{a0}b}|{a\u{feff}b}|{a\u{200b}b}]";
    let i18n = I18n::builder()
        .messages("en", None, &format!(r#"{{"k": "{tpl}"}}"#))
        .missing(guard.missing())
        .build()
        .unwrap();
    let args = Args::new()
        .str("a b", "S")
        .str("a\u{a0}b", "N")
        .str("a\u{feff}b", "F")
        .str("a\u{200b}b", "Z");
    let want = "[{a b}|{a\u{a0}b}|{a\u{feff}b}|Z]";
    // step 1: a miss with the template as its default → interpolated once, JS spaces kept
    guard.allow("en|index|nope");
    assert_eq!(i18n.t_with("nope", &args.clone().or(tpl)), want);
    // step 2: the same text found in the catalog → the same answer
    assert_eq!(i18n.t_with("k", &args), want);
    assert_eq!(guard.seen(), ["en|index|nope"]);
}

/// Given an `i18n.json` where the code `ua` has `iso: uk-UA` and `en` has a malformed iso,
/// when the app counts apples in each locale, then `ua` follows the Ukrainian categories,
/// `en` (no rules for `en_US`) the index rule, and a view in another locale its own rules.
#[test]
fn plurals_follow_the_iso_tag() {
    // step 1: build from the config → both locales known, `ua` active
    let i18n = I18n::builder()
        .config_json(
            r#"{"defaultLocale": "ua", "locales": [
                {"code": "ua", "iso": "uk-UA"}, {"code": "en", "iso": "en_US"}]}"#,
        )
        .messages(
            "ua",
            None,
            r#"{"apples": "немає яблук | {count} яблуко | {count} яблука | {count} яблук"}"#,
        )
        .messages(
            "en",
            None,
            r#"{"apples": "no apples | {count} apple | {count} apples | {count} lots"}"#,
        )
        .build()
        .unwrap();

    // step 2: Ukrainian categories, the zero form for 0
    let ua: Vec<String> = [0, 1, 2, 5, 11, 21, 22, 25, 101]
        .iter()
        .map(|&n| i18n.tc("apples", n))
        .collect();
    assert_eq!(
        ua,
        [
            "немає яблук",
            "1 яблуко",
            "2 яблука",
            "5 яблук",
            "11 яблук",
            "21 яблуко",
            "22 яблука",
            "25 яблук",
            "101 яблуко"
        ]
    );

    // step 3: a view in `en`: `en_US` is no BCP 47 tag → index rule, the last form past the end
    let en = i18n.with_locale("en");
    assert_eq!(en.tc("apples", 1), "1 apple");
    assert_eq!(en.tc("apples", 3), "3 lots");
    assert_eq!(en.tc("apples", 21), "21 lots");

    // step 4: switch the active locale → the same rules as the view
    i18n.set_locale("en").unwrap();
    assert_eq!(i18n.tc("apples", 21), "21 lots");

    // step 5: a negative or fractional count → the last form, `{count}` as JS prints it
    i18n.set_locale("ua").unwrap();
    assert_eq!(i18n.tc("apples", -1), "-1 яблук");
    #[cfg(feature = "format")]
    assert_eq!(i18n.tc("apples", 1.5), "1.5 яблук");
}

/// Given messages of every kind, when the app calls `tc` on each, then plurals pick a form,
/// a string without `|` is its own form, a number comes back as is, and a boolean, a
/// subtree or a miss fall back to the default or the key; only the miss is reported.
#[test]
fn tc_on_every_kind_of_value() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"car": "car | cars", "plain": "  {count} things  ", "n": 7, "flag": true,
                "tree": {"a": "b"}, "marked": "=0 none | one | {count} many"}"#,
        )
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 1: plural forms; `t` keeps the whole string
    assert_eq!(i18n.tc("car", 0), "car");
    assert_eq!(i18n.tc("car", 7), "cars");
    assert_eq!(i18n.t("car"), "car | cars");
    assert_eq!(i18n.tc("marked", 0), "none");
    assert_eq!(i18n.tc("marked", 2), "2 many");

    // step 2: one form, trimmed and interpolated; a number as is
    assert_eq!(i18n.tc("plain", 3), "3 things");
    assert_eq!(i18n.tc("n", 3), "7");

    // step 3: a boolean and a subtree have no plural → default or key, no report
    let d = Args::new().or("{count} default");
    assert_eq!(i18n.tc("flag", 2), "flag");
    assert_eq!(i18n.tc_with("tree", 2, &d), "2 default");
    assert_eq!(guard.seen(), Vec::<String>::new());

    // step 4: a miss is reported; an empty default stays empty (unlike `t_with`)
    guard.allow("en|index|nope");
    guard.allow("en|settings|gone");
    assert_eq!(i18n.tc_with("nope", 1, &Args::new().or("")), "");
    assert_eq!(i18n.page("settings").tc("gone", 1), "gone");
    assert_eq!(guard.seen(), ["en|index|nope", "en|settings|gone"]);
}

/// Given `en` active and `ru` known, when the app reads Russian plurals through a view, then
/// switches to `ru`, then views a code that is not known but has rules (`fr`, reading the
/// fallback's text), then each answer goes by the rules of the locale read, not the active one.
#[test]
fn views_count_by_their_own_rules() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("en")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"apples": "no apples | {count} apple | {count} apples | {count} lots",
                "cart": "=0 empty | {count} one | {count} many | {count} other"}"#,
        )
        .messages(
            "ru",
            None,
            r#"{"apples": "нет яблок | {count} яблоко | {count} яблока | {count} яблок"}"#,
        )
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 1: a view in the known `ru` while `en` is active → Russian categories (21 → one)
    let ru = i18n.with_locale("ru");
    assert_eq!(ru.tc("apples", 21), "21 яблоко");
    assert_eq!(ru.tc("apples", 22), "22 яблока");
    // the active `en` goes by English rules: two named categories, so the index rule
    assert_eq!(i18n.tc("apples", 21), "21 lots");

    // step 2: switch to `ru` → the same Russian answers, for the instance and the view
    i18n.set_locale("ru").unwrap();
    assert_eq!(i18n.tc("apples", 21), "21 яблоко");
    assert_eq!(i18n.with_locale("ru").tc("apples", 21), "21 яблоко");
    // and a view back in `en` keeps English rules
    assert_eq!(i18n.with_locale("en").tc("apples", 21), "21 lots");

    // step 3: `fr` is not known: the fallback's text, French categories (`one, many,
    // other`: 2 → other, 10^6 → many); English or no rules would give `2 many`
    let fr = i18n.with_locale("fr");
    assert_eq!(fr.tc("cart", 2), "2 other");
    assert_eq!(fr.tc("cart", 1_000_000), "1000000 many");
    assert_eq!(i18n.with_locale("en").tc("cart", 2), "2 many");
    assert_eq!(guard.seen(), Vec::<String>::new());
}

/// Given plural messages, when the app passes `tc_with` an argument named `count`, a list,
/// and counts past 2^53, then the count wins (JS `{ ...params, count }`), a list gives its
/// items but no `{length}` (a spread array has none), and a huge integer is the `f64` JS
/// holds, for the form and for `{count}`.
#[test]
fn counts_win_spread_and_round_like_js() {
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("fr")
        .messages(
            "fr",
            None,
            r#"{"n": "{count}", "pair": "{0}+{1} of {length} | {0}+{1} of {length}, {count}",
                "cart": "=0 zéro | {count} un | {count} beaucoup | {count} autre"}"#,
        )
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 1: an argument named `count` loses to the count, in the form and in a default
    guard.allow("fr|index|nope");
    let args = Args::new().int("count", 9);
    assert_eq!(i18n.tc_with("n", 3, &args), "3");
    assert_eq!(i18n.tc_with("nope", 3, &args.clone().or("{count}")), "3");

    // step 2: a list → `{0}`, `{1}`, but `{length}` stays (t_with still reads it)
    guard.allow("fr|index|nope");
    guard.allow("fr|index|nope");
    let list = Args::list(["A", "B"]);
    assert_eq!(i18n.tc_with("pair", 1, &list), "A+B of {length}, 1");
    assert_eq!(
        i18n.tc_with("nope", 1, &list.clone().or("{0} {length}")),
        "A {length}"
    );
    assert_eq!(i18n.t_with("nope", &list.clone().or("{0} {length}")), "A 2");

    // step 3: past 2^53 → the nearest f64, printed as JS `String(n)`
    assert_eq!(i18n.tc("n", (1u64 << 53) + 1), "9007199254740992");
    assert_eq!(i18n.tc("n", u64::MAX), "18446744073709552000");
    assert_eq!(i18n.tc("n", i64::MIN), "-9223372036854776000");
    assert_eq!(i18n.tc("n", (1u64 << 62) + 1), "4611686018427388000");
    assert_eq!(i18n.tc("n", 1u64 << 53), "9007199254740992");

    // step 4: the category of such a count is ICU's (`Intl.PluralRules("fr")`): 10^18 and
    // 2·10^18 have `i % 10^18 = 0` → one, 3·10^17 → many; a negative one takes the last form
    assert_eq!(i18n.tc("cart", 10u64.pow(18)), "1000000000000000000 un");
    assert_eq!(i18n.tc("cart", 2 * 10u64.pow(18)), "2000000000000000000 un");
    assert_eq!(
        i18n.tc("cart", 3 * 10u64.pow(17)),
        "300000000000000000 beaucoup"
    );
    assert_eq!(i18n.tc("cart", i64::MIN), "-9223372036854776000 autre");
    assert_eq!(guard.seen().len(), 3);
}

/// An app error with translated messages, the `thiserror` way (`#[error("{}", key!(..))]`,
/// `#[error("{}", t!("..", path = .0))]`), its `Display` written by hand: no dependency.
#[derive(Debug)]
enum AppError {
    NotFound,
    Io(std::path::PathBuf),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound => write!(f, "{}", i18n_next::key!("errors.not_found")),
            AppError::Io(p) => write!(
                f,
                "{}",
                i18n_next::t!("errors.io", path = p.display().to_string())
            ),
        }
    }
}

impl std::error::Error for AppError {}

/// Given an error enum whose `Display` uses `key!` and `t!`, when it is shown in a scope,
/// after `set_locale`, and in a scope of another locale, then each time it shows the locale
/// that is current then; boxed as `dyn Error` it reads the same.
#[test]
fn error_messages_follow_the_current_locale() {
    let i18n = I18n::builder()
        .locale("en")
        .messages(
            "en",
            None,
            r#"{"errors": {"not_found": "Not found", "io": "Cannot read {path}"}}"#,
        )
        .messages(
            "de",
            None,
            r#"{"errors": {"not_found": "Nicht gefunden", "io": "{path} ist nicht lesbar"}}"#,
        )
        .build()
        .unwrap();
    let io = AppError::Io("/etc/app.toml".into());
    scope().instance(i18n.clone()).run(|| {
        // step 1: the active locale → English
        assert_eq!(AppError::NotFound.to_string(), "Not found");
        assert_eq!(io.to_string(), "Cannot read /etc/app.toml");
        // step 2: set_locale → the same values speak German
        i18n.set_locale("de").unwrap();
        let boxed: Box<dyn std::error::Error> = Box::new(AppError::NotFound);
        assert_eq!(boxed.to_string(), "Nicht gefunden");
        assert_eq!(io.to_string(), "/etc/app.toml ist nicht lesbar");
        // step 3: a nested scope of another locale → that one
        scope()
            .locale("en")
            .run(|| assert_eq!(format!("{io}"), "Cannot read /etc/app.toml"));
        // step 4: a `Text` as a `t_with` argument → resolved inside, in the active locale
        let nested = Args::new().arg("path", i18n.text("errors.not_found"));
        assert_eq!(
            i18n.t_with("errors.io", &nested),
            "Nicht gefunden ist nicht lesbar"
        );
    });
}

/// Given an axum-like server with one global catalog and a middleware that negotiates
/// `Accept-Language` and wraps the handler in a scope, when two requests in different
/// languages are polled interleaved on one thread (one waits on I/O), then each handler,
/// across its awaits, answers in its own language; between polls no scope is left over.
#[test]
fn two_requests_in_their_own_languages() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let i18n = I18n::builder()
        .locale("en")
        .messages("en", None, r#"{"hello": "Hello, {name}", "done": "Done"}"#)
        .messages(
            "de",
            None,
            r#"{"hello": "Hallo, {name}", "done": "Fertig"}"#,
        )
        .messages(
            "pt-br",
            None,
            r#"{"hello": "Olá, {name}", "done": "Pronto"}"#,
        )
        .build()
        .unwrap();
    let available: Vec<String> = i18n.locales().into_iter().map(|l| l.code).collect();
    let out: Rc<RefCell<Vec<String>>> = Rc::default();
    // the middleware: negotiate, then the handler in a scope
    let request = |accept: &str, name: &'static str| {
        let lang = i18n_next::negotiate(accept, &available).unwrap_or_else(|| "en".into());
        let (out, i18n) = (out.clone(), i18n.clone());
        scope().instance(i18n).locale(lang).wrap(async move {
            out.borrow_mut().push(t!("hello", name = name).to_string());
            Pending::once().await; // the handler awaits a database
            out.borrow_mut().push(t!("done").to_string());
        })
    };
    let mut ex = support::Executor::new();
    // step 1: two requests arrive → German and Brazilian Portuguese
    ex.spawn(request("de-AT,de;q=0.9,en;q=0.5", "Ann"));
    ex.spawn(request("fr, pt;q=0.8", "Bia"));
    // step 2: poll A, then B, then A, then B → each line in its request's language
    for i in [0, 1, 0, 1] {
        ex.poll(i);
        // between polls nothing is in scope
        assert_eq!(t!("done"), "done");
    }
    assert_eq!(ex.unfinished(), 0);
    assert_eq!(
        *out.borrow(),
        ["Hallo, Ann", "Olá, Bia", "Fertig", "Pronto"]
    );
}

/// Pending on the first poll (waking itself), ready on the next.
struct Pending(bool);

impl Pending {
    fn once() -> Self {
        Pending(false)
    }
}

impl std::future::Future for Pending {
    type Output = ();
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.0 {
            std::task::Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

/// Given `Accept-Language` headers with refused tags (`q=0`), scripts and malformed
/// weights, when they are negotiated, then a refused tag is never chosen (not by `*`, not
/// by shortening, not as the same language), a tag's script is preferred, and an entry with
/// a malformed weight is dropped (RFC 9110).
#[test]
fn negotiate_refusals_scripts_and_bad_weights() {
    use i18n_next::negotiate;
    // step 1: `*` with a refused tag → the first available one that is not refused
    assert_eq!(negotiate("*, en;q=0", ["en", "de"]).as_deref(), Some("de"));
    assert_eq!(negotiate("en;q=0, *", ["en"]), None);
    // step 2: a refused language is not reached by shortening or the same-language step
    assert_eq!(negotiate("de;q=0, de-AT", ["de", "en"]), None);
    assert_eq!(
        negotiate("de;q=0, de-AT, en;q=0.5", ["de", "en"]).as_deref(),
        Some("en")
    );
    assert_eq!(negotiate("de-at;q=0, de", ["de-AT"]), None);
    assert_eq!(negotiate("de-AT;q=0, de", ["de"]).as_deref(), Some("de"));
    // step 3: same language → the same script first, then any
    let zh = ["zh-hans", "zh-hant-hk"];
    assert_eq!(negotiate("zh-Hant-TW", zh).as_deref(), Some("zh-hant-hk"));
    assert_eq!(negotiate("zh-Hans-SG", zh).as_deref(), Some("zh-hans"));
    assert_eq!(negotiate("zh-TW", zh).as_deref(), Some("zh-hans"));
    assert_eq!(negotiate("zh-Latn", zh).as_deref(), Some("zh-hans"));
    // step 4: spaces around `=`, a weight out of range, another parameter → dropped
    let both = ["de", "en"];
    assert_eq!(
        negotiate("de;q = 0.5, en;q=0.4", both).as_deref(),
        Some("en")
    );
    assert_eq!(
        negotiate("de;q= 0.5, en;q=0.4", both).as_deref(),
        Some("en")
    );
    assert_eq!(negotiate("de;q=2, en;q=0.4", both).as_deref(), Some("en"));
    assert_eq!(
        negotiate("de;level=1, en;q=0.4", both).as_deref(),
        Some("en")
    );
    assert_eq!(
        negotiate("de ; q=0.5 , en;q=0.4", both).as_deref(),
        Some("de")
    );
    assert_eq!(negotiate("de;Q=0.5, en;q=0.4", both).as_deref(), Some("de"));
}

/// Given a hostile `Accept-Language` (thousands of entries, megabytes of bytes), when it is
/// negotiated, then only the first 32 entries within 4 KiB count: the work is bounded by
/// those caps, not by the header, and the result is the one those entries give.
#[test]
fn negotiate_bounds_a_huge_header() {
    use i18n_next::negotiate;
    let filler = |n: usize| vec!["xx;q=0"; n].join(",");
    // step 1: the 32nd entry counts, the 33rd and everything after it do not
    assert_eq!(
        negotiate(&format!("{},de", filler(31)), ["de"]).as_deref(),
        Some("de")
    );
    assert_eq!(negotiate(&format!("{},de", filler(32)), ["de"]), None);
    // step 2: 100 000 refusals behind a match: the match wins, the tail is never read
    let huge = format!("fr;q=0.1,{}", filler(100_000));
    assert!(huge.len() > 500_000);
    assert_eq!(negotiate(&huge, ["en", "fr"]).as_deref(), Some("fr"));
    // step 3: past 4 KiB the header is cut at the last `,`, never inside a tag
    let long = format!("{},de-AT", "a-".repeat(2050));
    assert!(long.len() > 4096);
    assert_eq!(negotiate(&long, ["de"]), None);
    let one_tag = format!("{}en", "a-".repeat(5_000));
    assert_eq!(negotiate(&one_tag, ["en", "a"]), None);
}

fn nesting_catalog() -> I18n {
    I18n::builder()
        .locale("en")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"outer": "Outer {x}", "inner": "inner-en", "bold": "<b>{y}</b>", "leaf": "L", "wrap": "({x})"}"#,
        )
        .messages("de", None, r#"{"outer": "Außen {x}", "inner": "inner-de"}"#)
        .messages("en", Some("cart"), r#"{"inner": "inner-cart"}"#)
        .build()
        .unwrap()
}

/// Given an outer `Text` with its own locale or page (or a scope's) and a nested `Text`
/// without, when the outer is shown, then the nested one resolves in the outer's locale,
/// page and instance; its own options still win.
#[test]
fn nested_text_inherits_the_outer_locale_page_and_instance() {
    let i18n = nesting_catalog();
    let inner = || i18n.text("inner");
    // step 1: the outer's own locale / page → the nested text follows
    assert_eq!(
        i18n.text("outer").locale("de").arg("x", inner()),
        "Außen inner-de"
    );
    assert_eq!(
        i18n.text("outer").page("cart").arg("x", inner()),
        "Outer inner-cart"
    );
    let shown = i18n
        .text("outer")
        .locale("de")
        .arg("x", inner())
        .to_string();
    assert_eq!(shown, "Außen inner-de");
    // step 2: a scope's locale reaches both, an inner own locale wins
    let out = scope().locale("de").run(|| {
        i18n.text("outer")
            .arg("x", inner().locale("en"))
            .to_string()
    });
    assert_eq!(out, "Außen inner-en");
    // step 3: an implicit nested text (`t!`) takes the outer's instance
    assert_eq!(
        i18n.text("outer").locale("de").arg("x", t!("inner")),
        "Außen inner-de"
    );
    // step 4: deeper levels inherit through each level
    let two = i18n
        .text("outer")
        .locale("de")
        .arg("x", i18n.text("outer").arg("x", inner()));
    assert_eq!(two, "Außen Außen inner-de");
}

/// Given an `html` outer text, when a nested `Text` is shown in it, then a plain nested
/// text is escaped once and an `html` one (markup already, its own args escaped) is not.
#[test]
fn nested_html_text_is_escaped_once() {
    let i18n = nesting_catalog();
    let bold = || i18n.text("bold").arg("y", "<i>");
    // step 1: html in html → as is
    assert_eq!(
        i18n.text("outer").html().arg("x", bold().html()),
        "Outer <b>&lt;i&gt;</b>"
    );
    // step 2: plain in html → escaped
    assert_eq!(
        i18n.text("outer").html().arg("x", bold()),
        "Outer &lt;b&gt;&lt;i&gt;&lt;/b&gt;"
    );
    // step 3: html in plain → as the inner shows it
    assert_eq!(
        i18n.text("outer").arg("x", bold().html()),
        "Outer <b>&lt;i&gt;</b>"
    );
}

/// Given a `Text` and a `Key`, when they are formatted with width, fill and precision,
/// then the text is padded and cut like a `str`; without them nothing changes.
#[test]
fn display_honours_width_fill_and_precision() {
    let i18n = nesting_catalog();
    let t = i18n.text("inner");
    // step 1: width, alignment, precision
    assert_eq!(
        format!("[{:>10}] [{:<10}] [{:.2}] [{:*^12}]", t, t, t, t),
        "[  inner-en] [inner-en  ] [in] [**inner-en**]"
    );
    assert_eq!(format!("{t}"), "inner-en");
    // step 2: a key, through a scope
    const INNER: i18n_next::Key = i18n_next::key!("inner");
    let out = scope()
        .instance(i18n.clone())
        .run(|| format!("[{INNER:>9}]"));
    assert_eq!(out, "[ inner-en]");
}

/// Given values of more kinds, when they are arguments, then each shows as JS `String`
/// shows it: integers past 2^53 rounded to the nearest `f64`, a `char`, any `Display`
/// through `Arg::display`; a raw identifier names its argument without `r#`.
#[test]
fn more_argument_kinds_and_raw_names() {
    use i18n_next::Arg;
    let i18n = I18n::builder()
        .messages("en", None, r#"{"n": "[{n}]", "kw": "{type}/{match}"}"#)
        .build()
        .unwrap();
    let n = |a: Arg<'static>| i18n.text("n").arg("n", a).to_string();
    // step 1: u64 → exact up to 2^53, the nearest f64 past it
    assert_eq!(n(Arg::from(7u64)), "[7]");
    assert_eq!(n(Arg::from((1u64 << 53) + 1)), "[9007199254740992]");
    assert_eq!(n(Arg::from(u64::MAX)), "[18446744073709552000]");
    // step 2: u128 / i128 → the same rule, an exponent from 1e21 (feature `format`)
    #[cfg(feature = "format")]
    {
        assert_eq!(n(Arg::from(42u128)), "[42]");
        assert_eq!(n(Arg::from(10u128.pow(21))), "[1e+21]");
        assert_eq!(n(Arg::from(-(1i128 << 60))), "[-1152921504606847000]");
        assert_eq!(n(Arg::from(-5i128)), "[-5]");
        assert_eq!(n(Arg::from(u128::MAX)), "[3.402823669209385e+38]");
    }
    // step 3: char, Display
    assert_eq!(n(Arg::from('é')), "[é]");
    let path = std::path::Path::new("/tmp/a.json");
    assert_eq!(n(Arg::display(path.display())), "[/tmp/a.json]");
    assert_eq!(
        i18n.t_with("n", &i18n_next::Args::new().arg("n", Arg::display(1.5f32))),
        "[1.5]"
    );
    // step 4: raw identifiers in `t!`
    assert_eq!(t!(i18n => "kw", r#type = "a", r#match = 'b'), "a/b");
}

/// Given a `Text` nested 10 000 levels deep (each level wraps the previous one), when it
/// is shown, turned into a string, cloned and dropped on a normal test thread, then nothing
/// overflows the stack: nesting stops at 64 levels and a deeper argument's placeholder
/// shows as written.
#[test]
fn deep_nesting_is_capped() {
    let i18n = nesting_catalog();
    let mut t = i18n.text("leaf");
    for _ in 0..10_000 {
        t = i18n.text("wrap").arg("x", t);
    }
    // step 1: shown → at most 65 levels, the innermost kept one is a raw placeholder
    let s = t.to_string();
    let open = s.bytes().take_while(|&b| b == b'(').count();
    assert!((1..=65).contains(&open), "{open} levels");
    assert_eq!(s, format!("{}{{x}}{}", "(".repeat(open), ")".repeat(open)));
    assert_eq!(format!("{t}"), s);
    assert!(t == s.as_str());
    // step 2: cloned and dropped → no recursion past the cap
    let c = t.clone();
    assert_eq!(c.to_string(), s);
    drop(c);
    drop(t);
    // step 3: exactly 64 levels are kept, a 65th is dropped
    let mut t = i18n.text("leaf");
    for _ in 0..64 {
        t = i18n.text("wrap").arg("x", t);
    }
    assert_eq!(
        t.to_string(),
        format!("{}L{}", "(".repeat(64), ")".repeat(64))
    );
    let list = i18n.text("wrap").args([t.clone()]).arg("x", t.clone());
    assert_eq!(list.to_string(), "({x})");
    // step 4: the deepest kept text shows on a 1 MiB stack (a wasm default)
    let shown = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || t.to_string())
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(shown.len(), 129);
}

/// Given a `missing` handler, when texts with misses are shown (a nested one under
/// `hide_if_empty`, hidden or not, into a writer that fails, compared), then each miss is
/// reported exactly once per display, never by a comparison.
#[test]
fn missing_is_reported_once_per_display() {
    use std::fmt::Write as _;
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let i18n = I18n::builder()
        .locale("en")
        .messages("en", None, r#"{"after": "{x}y", "only": "{x}"}"#)
        .missing(move |_, k, _| log.lock().unwrap().push(k.to_owned()))
        .build()
        .unwrap();
    let take = || std::mem::take(&mut *seen.lock().unwrap());
    let blank_miss = || i18n.text("nope").or("  ");
    // step 1: hide_if_empty, the nested miss is blank and the outer is not → once
    let t = i18n.text("after").hide_if_empty().arg("x", blank_miss());
    assert_eq!(t.to_string(), "  y");
    assert_eq!(take(), ["nope"]);
    assert_eq!(format!("{t}"), "  y");
    assert_eq!(take(), ["nope"]);
    // step 2: hidden → the nested miss still once
    let hidden = i18n.text("only").hide_if_empty().arg("x", blank_miss());
    assert_eq!(hidden.to_string(), "");
    assert_eq!(take(), ["nope"]);
    assert_eq!(hidden.resolve(), None);
    assert_eq!(take(), ["nope"]);
    assert!(hidden.parts().next().is_none());
    assert_eq!(take(), ["nope"]);
    // step 3: a writer that fails on any text → the miss is still reported
    struct Fail;
    impl std::fmt::Write for Fail {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            if s.is_empty() {
                Ok(())
            } else {
                Err(std::fmt::Error)
            }
        }
    }
    assert!(write!(Fail, "{}", i18n.text("gone")).is_err());
    assert_eq!(take(), ["gone"]);
    assert!(write!(Fail, "{}", i18n.text("only").arg("x", i18n.text("gone"))).is_err());
    assert_eq!(take(), ["gone"]);
    // step 4: a comparison reports nothing
    assert!(i18n.text("gone") == "gone");
    assert!(t == "  y");
    assert!(take().is_empty());
}

/// Given an app that reads texts through `t_ref`, when the closure reads again, switches
/// the locale and reports a miss from inside the call, then each read sees the catalog as
/// it is at that moment and nothing locks up.
#[test]
fn t_ref_reads_in_place() {
    let misses = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = misses.clone();
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages("de", None, r#"{"hi": "Hallo"}"#)
        .messages("en", None, r#"{"hi": "Hello", "bye": "Bye"}"#)
        .messages("de", Some("settings"), r#"{"title": "Einstellungen"}"#)
        .missing(move |locale, key, _| seen.lock().unwrap().push(format!("{locale}:{key}")))
        .build()
        .unwrap();

    // step 1: hit, fallback, page, miss → the same texts as `t`
    assert_eq!(i18n.t_ref("hi", str::to_owned), "Hallo");
    assert_eq!(i18n.t_ref("bye", str::to_owned), "Bye");
    assert_eq!(i18n.page("settings").t_ref("title", str::to_owned), "Einstellungen");
    assert_eq!(i18n.t_ref("nope", str::to_owned), "nope");
    assert_eq!(i18n.t_ref("", str::len), 0);
    assert_eq!(*misses.lock().unwrap(), ["de:nope"]);

    // step 2: nested reads and a locale switch inside the closure → the outer text stays,
    // the inner read after the switch sees `en`
    let (outer, inner) = i18n.t_ref("hi", |outer| {
        let before = i18n.t_ref("hi", str::to_owned);
        i18n.set_locale("en").unwrap();
        (format!("{outer}/{before}"), i18n.t_ref("hi", str::to_owned))
    });
    assert_eq!(outer, "Hallo/Hallo");
    assert_eq!(inner, "Hello");
    assert_eq!(i18n.t_ref("hi", str::to_owned), "Hello");
}
