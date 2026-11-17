//! Stories that run on every platform (DESIGN 7.6): the same bodies are called from
//! `tests/wasm.rs` (wasm-bindgen-test, Node and Chrome) and `tests/wasi.rs` (native and
//! wasmtime), so the browser, WASI and native paths check one scenario list. Only
//! [`dir_story`] reads the file system, and it is left out of `wasm32-unknown-unknown`.
#![allow(dead_code)]

use i18n_next::{Args, Error, I18n};

/// Given a message with 600 placeholders beside an unused 4 MiB argument, when `t_with`,
/// `tc_with` and a default fill it, then the result is sized by the values used. On wasm32
/// (a 32-bit `usize`) the old bound, holes × longest value = 2.4 GB, overflowed and
/// panicked with "capacity overflow".
pub fn many_holes_beside_a_huge_argument() {
    let tpl = "{a}".repeat(600);
    let json = format!(r#"{{"k": "{tpl}", "p": "x | {tpl}", "one": " {tpl} "}}"#);
    let i18n = I18n::builder()
        .locale("en")
        .messages("en", None, &json)
        .missing(|_, _, _| {})
        .build()
        .unwrap();
    let big = "y".repeat(4 << 20);
    let args = Args::new().str("b", &big).str("a", "1");
    let want = "1".repeat(600);
    // step 1: a template, a plural form, a single form → 600 ones
    assert_eq!(i18n.t_with("k", &args), want);
    assert_eq!(i18n.tc_with("p", 5, &args), want);
    assert_eq!(i18n.tc_with("one", 5, &args), want);
    // step 2: a default tokenized on the fly → the same
    assert_eq!(i18n.t_with("nope", &args.clone().or(&tpl)), want);
    assert_eq!(i18n.tc_with("nope", 5, &args.clone().or(&tpl)), want);
}

/// Given embedded files, when the app switches locales and adds messages, then the
/// snapshot path (thread-local cache, `RwLock`) works, also in a single-threaded runtime.
pub fn files_and_switching() {
    let i18n = I18n::builder()
        .locale("en")
        .files([
            ("en.json", br#"{"hi": "Hi, {name}!"}"#.as_slice()),
            ("de.json", br#"{"hi": "Hallo, {name}!"}"#),
            ("pages/user/profile/de.json", br#"{"title": "Profil"}"#),
        ])
        .build()
        .unwrap();
    let args = Args::new().str("name", "Ann");
    // step 1: the start locale answers
    assert_eq!(i18n.t_with("hi", &args), "Hi, Ann!");
    // step 2: a switch is seen by the next read, nested page folders join with `-`
    i18n.set_locale("de").unwrap();
    assert_eq!(i18n.t_with("hi", &args), "Hallo, Ann!");
    assert_eq!(
        i18n.page("user-profile").t("title", str::to_owned),
        "Profil"
    );
    // step 3: an added number prints as JS prints it; two changes, two versions
    i18n.add_messages("de", None, r#"{"n": 1.5e-7}"#).unwrap();
    assert_eq!(i18n.t("n", str::to_owned), "1.5e-7");
    assert_eq!(i18n.version(), 2);
}

/// Given embedded files and `messages`, when the app builds and reads, then pages,
/// fallbacks, interpolation and the broken-file error work as on native.
pub fn files_and_messages() {
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .files([
            ("en.json", br#"{"a": "A", "hi": "Hi {name}"}"#.as_slice()),
            ("pages/settings/de.json", br#"{"title": "Einstellungen"}"#),
        ])
        .messages("de", None, r#"{"hi": "Hallo {name}"}"#)
        .build()
        .unwrap();
    // step 1: fallback, interpolation, page
    assert_eq!(i18n.t("a", str::to_owned), "A");
    assert_eq!(
        i18n.t_with("hi", &Args::new().str("name", "Ann")),
        "Hallo Ann"
    );
    assert_eq!(
        i18n.page("settings").t("title", str::to_owned),
        "Einstellungen"
    );
    // step 2: a broken file is an error with its path
    let err = I18n::builder()
        .files([("de.json", b"{")])
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Json { path, .. } if path == "de.json"));
}

/// Given a fallback chain with a cycle (`ru → de-at → de → ru`, global `en`), two page
/// aliases and `pages(false)` on another catalog, when the app reads through them, then
/// the chain stops at the cycle in order, an alias reads its target, and folded page files
/// win over the root file.
pub fn chains_aliases_and_folding() {
    let i18n = I18n::builder()
        .locale("ru")
        .fallback("en")
        .fallback_for("ru", "de-at")
        .fallback_for("de-at", "de")
        .fallback_for("de", "ru")
        .messages(
            "en",
            None,
            r#"{"a": "en", "b": "en", "c": "en", "d": "en"}"#,
        )
        .messages("de", None, r#"{"a": "de", "b": "de", "c": "de"}"#)
        .messages("de-at", None, r#"{"a": "at", "b": "at"}"#)
        .messages("ru", None, r#"{"a": "ru"}"#)
        .messages("ru", Some("about"), r#"{"title": "О нас"}"#)
        .page_alias("about-us", "about")
        .page_alias("company", "about")
        .build()
        .unwrap();
    // step 1: ru → de-at → de → (ru again: stop) → en
    assert_eq!(
        ["a", "b", "c", "d"].map(|k| i18n.t(k, str::to_owned)),
        ["ru", "at", "de", "en"]
    );
    // step 2: the cycle seen from de: de → ru → de-at → en
    let de = i18n.with_locale("de");
    assert_eq!(
        ["a", "b", "c", "d"].map(|k| de.t(k, str::to_owned)),
        ["de", "de", "de", "en"]
    );
    assert_eq!(de.t_with("nope", &Args::new().or("{x}").str("x", "X")), "X");
    // step 3: both aliases read the target page, and are pages themselves
    assert_eq!(i18n.page("about-us").t("title", str::to_owned), "О нас");
    assert_eq!(i18n.page("company").t("title", str::to_owned), "О нас");
    assert_eq!(i18n.pages(), ["about", "about-us", "company"]);

    // step 4: pages(false) folds the page files of `en` into its root; the page file wins
    let folded = I18n::builder()
        .pages(false)
        .locale("en")
        .files([
            ("en.json", br#"{"k": "root", "r": "root"}"#.as_slice()),
            ("pages/x/en.json", br#"{"k": "page", "p": "page"}"#),
        ])
        .build()
        .unwrap();
    assert_eq!(folded.pages(), Vec::<String>::new());
    assert_eq!(
        ["k", "r", "p"].map(|k| folded.t(k, str::to_owned)),
        ["page", "root", "page"]
    );
}

/// Given a real locales directory (root files, a BOM, page folders one nested, a file
/// outside the layout), when the app builds from `dir`, then it reads exactly like
/// `files()`. On wasip1 the directory is reached through wasmtime's `--dir=.` preopen,
/// relative to the package root (CI: `wasmtime run --dir=.`).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn dir_story() {
    let root = if cfg!(target_os = "wasi") {
        std::path::PathBuf::from("tests/support/dir")
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/dir")
    };
    let i18n = I18n::builder()
        .dir(&root)
        .locale("de")
        .fallback("en")
        .build()
        .unwrap();
    // step 1: root of de (with a BOM), fallback to en
    assert_eq!(i18n.t("nav.home", str::to_owned), "Startseite");
    assert_eq!(i18n.t("nav.settings", str::to_owned), "Settings");
    // step 2: page layer, then root; a nested folder is `user-profile`
    assert_eq!(
        i18n.page("settings").t("title", str::to_owned),
        "Einstellungen"
    );
    assert_eq!(
        i18n.page("settings").t("nav.home", str::to_owned),
        "Startseite"
    );
    assert_eq!(
        i18n.page("user-profile").t("title", str::to_owned),
        "Profile"
    );
    // step 3: the file outside the layout is never read
    assert_eq!(i18n.get("ignored", |t| t.map(str::to_owned)), None);
    assert_eq!(i18n.pages(), ["settings", "user-profile"]);
    let codes: Vec<String> = i18n.locales().into_iter().map(|l| l.code).collect();
    assert_eq!(codes, ["de", "en"]);
}
