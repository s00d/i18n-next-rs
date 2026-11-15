//! Stories of the crate (DESIGN 7.4): one event stream per test, Given/When/Then in the
//! doc comment, `// step n: action → check` in the body.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{Args, Error, I18n, LocaleInfo};
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
        "I18n { locale: \"fr\", version: 3, .. }"
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
