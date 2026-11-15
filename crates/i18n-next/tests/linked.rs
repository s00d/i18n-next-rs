//! Stories of linked messages (DESIGN 1.1, 7.4, M3): one event stream per test,
//! Given/When/Then in the doc comment, `// step n: action → check` in the body. The exact
//! JS semantics are pinned by `linked_compat.rs`; these follow an app through them.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{Args, CheckItem, I18n, LinkError};
use support::{Executor, FakeFetch, Guard, Locales, loads};

fn shop() -> Locales {
    Locales::new()
        .file(
            "en.json",
            r#"{"brand": "acme", "title": "Welcome to @:brand", "shout": "@.upper:brand",
                "intro": "@.capitalize:{'nav.home'}.", "nav": {"home": "home"},
                "greet": "Hi @:withName", "withName": "{name}!",
                "cart": "no items | one item in @:brand | {count} items in @:brand"}"#,
        )
        .file(
            "de.json",
            r#"{"brand": "Akme", "nav": {"home": "Startseite"}}"#,
        )
        .file(
            "pages/checkout/en.json",
            r#"{"brand": "acme pay", "pay": "Pay with @:brand", "back": "Back to @:{'nav.home'}"}"#,
        )
}

/// Given an English catalog with links, a German one that only renames the brand, and a
/// checkout page that overrides the brand, when the app builds with
/// `linked_messages(true)`, then every view reads its own resolution: German reads the
/// English messages with its own brand, the page reads root messages with the page's brand,
/// placeholders of a linked text fill at read, plurals keep working, and `has` still sees
/// only the locale's own keys.
#[test]
fn every_view_reads_its_own_resolution() {
    // step 1: build with links on → nothing reported
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("en")
        .fallback("en")
        .files(shop().files())
        .linked_messages(true)
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 2: English root → plain links, modifiers, a literal key before a dot
    assert_eq!(i18n.t("title"), "Welcome to acme");
    assert_eq!(i18n.t("shout"), "ACME");
    assert_eq!(i18n.t("intro"), "Home.");
    assert_eq!(i18n.get("title").as_deref(), Some("Welcome to acme"));

    // step 3: the page view → root messages see the page's brand, page links reach root keys
    let checkout = i18n.page("checkout");
    assert_eq!(checkout.t("title"), "Welcome to acme pay");
    assert_eq!(checkout.t("pay"), "Pay with acme pay");
    assert_eq!(checkout.t("back"), "Back to home");

    // step 4: German → the fallback's messages with the German brand and home
    let de = i18n.with_locale("de");
    assert_eq!(de.t("title"), "Welcome to Akme");
    assert_eq!(de.t("shout"), "AKME");
    assert_eq!(de.t("intro"), "Startseite.");
    assert_eq!(de.page("checkout").t("back"), "Back to Startseite");
    // the German root beats the English page: the German brand
    assert_eq!(de.page("checkout").t("title"), "Welcome to Akme");

    // step 5: `has` → the locale's own keys only, as without links
    assert!(!de.has("title"));
    assert!(de.has("brand"));

    // step 6: placeholders of the linked text → filled at read
    let ann = Args::new().str("name", "Ann");
    assert_eq!(i18n.t_with("greet", &ann), "Hi Ann!");
    assert_eq!(i18n.t("greet"), "Hi {name}!");

    // step 7: a plural with links → resolved, then forms by count
    assert_eq!(i18n.tc("cart", 1), "one item in acme");
    assert_eq!(de.tc("cart", 3), "3 items in Akme");

    // step 8: the catalog check → German lacks keys and the page, no link is broken
    let report = i18n.check();
    assert!(
        report
            .items
            .iter()
            .all(|i| !matches!(i, CheckItem::BrokenLink { .. })),
        "{report}"
    );
    assert!(report.items.contains(&CheckItem::MissingPage {
        locale: "de".into(),
        page: "checkout".into()
    }));
    guard.finish();
}

/// Given the same catalog, when the app builds without `linked_messages`, then `@:` is
/// plain text everywhere (off by default, as in the Nuxt module).
#[test]
fn links_are_off_by_default() {
    // step 1: build without the flag
    let i18n = I18n::builder().files(shop().files()).build().unwrap();

    // step 2: every link reads as written
    assert_eq!(i18n.t("title"), "Welcome to @:brand");
    assert_eq!(i18n.page("checkout").t("pay"), "Pay with @:brand");
    assert_eq!(i18n.tc("cart", 1), "one item in @:brand");

    // step 3: the check does not look at links when they are off
    let report = i18n.check();
    assert!(
        report
            .items
            .iter()
            .all(|i| !matches!(i, CheckItem::BrokenLink { .. })),
        "{report}"
    );
}

/// Given messages whose links name a missing key, a subtree, a boolean, a cycle, a message
/// that is itself broken and an unknown modifier, when the app builds with links on, then
/// each message stays as written, `missing` hears about each once (at build, with the
/// message's key), `t` on it later reports nothing, and `check` names every failing link.
#[test]
fn broken_links_stay_as_written_and_are_reported_once() {
    // step 1: build → one `missing` call per broken message
    let guard = Guard::new();
    for key in [
        "miss", "obj", "flag", "a", "b", "chain", "shout", "half", "fine",
    ] {
        guard.allow(&format!("en|index|{key}"));
    }
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"miss": "See @:nope", "obj": "Go @:nav", "nav": {"x": "x"},
                "flag": "@:yes", "yes": true, "a": "A @:b", "b": "B @:a", "chain": "C @:a",
                "shout": "@.shout:brand", "brand": "acme", "half": "@:brand and @:nope",
                "fine": "@:brand."}"#,
        )
        .linked_messages(true)
        .missing(guard.missing())
        .build()
        .unwrap();
    guard.finish();

    // step 2: lookups → the messages as written, no further `missing`
    assert_eq!(i18n.t("miss"), "See @:nope");
    assert_eq!(i18n.t("obj"), "Go @:nav");
    assert_eq!(i18n.t("a"), "A @:b");
    assert_eq!(i18n.t("half"), "@:brand and @:nope");
    // `brand.` is the key a bare link names: a missing key, but `fine` is checked below
    assert_eq!(i18n.t("fine"), "@:brand.");
    assert!(guard.unexpected().is_empty());

    // step 3: check → every failing link with its reason, sorted by key
    let broken: Vec<(String, String, LinkError)> = i18n
        .check()
        .items
        .into_iter()
        .filter_map(|item| match item {
            CheckItem::BrokenLink {
                key, link, reason, ..
            } => Some((key, link, reason)),
            _ => None,
        })
        .collect();
    let want = [
        ("a", "@:b", LinkError::Broken),
        ("b", "@:a", LinkError::Cycle),
        ("chain", "@:a", LinkError::Broken),
        ("fine", "@:brand.", LinkError::Missing),
        ("flag", "@:yes", LinkError::Missing),
        ("half", "@:nope", LinkError::Missing),
        ("miss", "@:nope", LinkError::Missing),
        ("obj", "@:nav", LinkError::Missing),
        ("shout", "@.shout:brand", LinkError::UnknownModifier),
    ];
    let want: Vec<_> = want
        .iter()
        .map(|(k, l, r)| ((*k).to_owned(), (*l).to_owned(), *r))
        .collect();
    assert_eq!(broken, want);
    assert!(
        i18n.check()
            .to_string()
            .contains("\"b\" left as written: @:a is a cycle")
    );
}

/// Given Turkish, Greek and Lithuanian locales (by `iso` and by code), when their messages
/// shout or capitalize a linked word, then the case follows the locale as
/// `toLocaleUpperCase` does in V8: dotted İ, Greek without accents, Lithuanian dots.
#[test]
fn case_modifiers_follow_the_locale() {
    // step 1: one word per locale, the same links in each
    let links = r#""up": "@.upper:w", "low": "@.lower:w", "cap": "@.capitalize:w""#;
    let i18n = I18n::builder()
        .locale("en")
        .iso("turkish", "tr-TR")
        .linked_messages(true)
        .messages("en", None, &format!(r#"{{"w": "istanbul", {links}}}"#))
        .messages(
            "turkish",
            None,
            &format!(r#"{{"w": "istanbul IŞIK", {links}}}"#),
        )
        .messages("el", None, &format!(r#"{{"w": "άυλος ή", {links}}}"#))
        .messages("lt", None, &format!(r#"{{"w": "Ìr", {links}}}"#))
        .build()
        .unwrap();

    // step 2: the default mapping
    assert_eq!(i18n.t("up"), "ISTANBUL");
    assert_eq!(i18n.t("cap"), "Istanbul");

    // step 3: Turkish by `iso` → dotted capital İ, dotless small ı
    let tr = i18n.with_locale("turkish");
    assert_eq!(tr.t("up"), "İSTANBUL IŞIK");
    assert_eq!(tr.t("low"), "istanbul ışık");
    assert_eq!(tr.t("cap"), "İstanbul IŞIK");

    // step 4: Greek by code → accents dropped, a dialytika kept where the accent was
    let el = i18n.with_locale("el");
    assert_eq!(el.t("up"), "ΑΫΛΟΣ Ή");
    // capitalize upper-cases the first letter alone: `ά` → `Α`
    assert_eq!(el.t("cap"), "Αυλος ή");

    // step 5: Lithuanian → a dot above where an accent sits on i
    assert_eq!(i18n.with_locale("lt").t("low"), "i\u{307}\u{300}r");
}

/// Given a built catalog with links, when the app adds messages at run time, then the new
/// text is not resolved (as `$mergeTranslations` in Nuxt) and earlier results stay.
#[test]
fn messages_added_later_are_not_resolved() {
    // step 1: build with links
    let i18n = I18n::builder()
        .messages(
            "en",
            None,
            r#"{"brand": "acme", "title": "Welcome to @:brand"}"#,
        )
        .linked_messages(true)
        .build()
        .unwrap();
    assert_eq!(i18n.t("title"), "Welcome to acme");

    // step 2: add a link and rename the brand → the link stays, the old result too
    i18n.add_messages("en", None, r#"{"brand": "Acme 2", "later": "See @:brand"}"#)
        .unwrap();
    assert_eq!(i18n.t("later"), "See @:brand");
    assert_eq!(i18n.t("title"), "Welcome to acme");
}

/// Given locales read on first use (`lazy_files`: a lazy root and a lazy page, beside an
/// eager German root) and a page the app fetches itself (`remote`), when the app builds with
/// links on, then each pair resolves its own links when it loads, over the layers of its
/// view: a page links its root (read first if needed) and the fallback, a remote page
/// resolves when its file arrives, and an evicted page resolves again when read again.
#[test]
fn lazy_and_remote_pairs_resolve_when_they_load() {
    // step 1: build → nothing read yet, nothing reported
    let guard = Guard::new();
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .linked_messages(true)
        .files([("de.json", br#"{"brand": "Akme"}"#.as_slice())])
        .lazy_files([
            (
                "en.json",
                br#"{"brand": "acme", "title": "Welcome to @:brand"}"#.as_slice(),
            ),
            (
                "pages/big/en.json",
                br#"{"cta": "Buy @.upper:{'brand'}: @:title"}"#,
            ),
            (
                "pages/big/de.json",
                br#"{"hint": "@.capitalize:{'brand'} @:nope"}"#,
            ),
        ])
        .remote(["pages/shop/de.json"])
        .evict_pages(1)
        .missing(guard.missing())
        .build()
        .unwrap();

    // step 2: the lazy English page in English → its root read too, links resolved
    let en = i18n.with_locale("en");
    assert_eq!(en.page("big").t("cta"), "Buy ACME: Welcome to acme");

    // step 3: the lazy German page → its own links over the German root and the fallback;
    // a broken link stays as written (the lazy path has no build to report to)
    assert_eq!(i18n.page("big").t("hint"), "@.capitalize:{'brand'} @:nope");
    assert_eq!(i18n.t("brand"), "Akme");

    // step 4: a remote page → as written until its file arrives, then resolved
    i18n.add_file("pages/shop/de.json", br#"{"buy": "Kaufen bei @:brand"}"#)
        .unwrap();
    assert_eq!(i18n.page("shop").t("buy"), "Kaufen bei Akme");

    // step 5: evicted and read again → resolved again
    let _ = i18n.page("big").t("hint");
    assert_eq!(en.page("big").t("cta"), "Buy ACME: Welcome to acme");
    guard.finish();
}

/// Lazy German and English files where German texts link the brand only the English root
/// has (`de` falls back to `en`): the files of the M3 audit's order probe.
fn brand_files() -> Locales {
    Locales::new()
        .file("en.json", r#"{"brand": "Acme"}"#)
        .file("de.json", r#"{"q": "@:brand"}"#)
        .file("pages/about/en.json", r#"{"o": "Other"}"#)
        .file(
            "pages/about/de.json",
            r#"{"p": "About @:brand", "r": "See @:o"}"#,
        )
        .file("pages/x/en.json", r#"{"x": "X"}"#)
}

/// Given lazy files where German texts link keys of the English root and of the English
/// page (the fallback), when the app reads them in different orders, with pages evicted
/// and read again in between, then every order answers what Nuxt answers for the whole
/// view: a lazy pair reads the other layers of its view before resolving its links, found
/// in the current snapshot (never a cell an eviction replaced).
#[test]
fn a_lazy_link_does_not_depend_on_what_was_read_first() {
    let files = brand_files();
    let build = || {
        I18n::builder()
            .locale("de")
            .fallback("en")
            .linked_messages(true)
            .lazy_files(files.files())
            .evict_pages(1)
            .build()
            .unwrap()
    };
    let want = |i: &I18n| {
        let about = i.page("about");
        assert_eq!(i.t("q"), "Acme");
        assert_eq!(about.t("p"), "About Acme");
        assert_eq!(about.t("r"), "See Other");
    };

    // step 1: German first, nothing English read yet → resolved over the English layers
    let first = build();
    assert_eq!(loads(&first), 0);
    want(&first);

    // step 2: English first → the same answers
    let english = build();
    let en = english.with_locale("en");
    assert_eq!(
        (en.t("brand"), en.page("about").t("o")),
        ("Acme".into(), "Other".into())
    );
    want(&english);

    // step 3: the English page loaded, evicted (by `x`), loaded again in a new cell, and
    // only then the German page → it reads the current English page, not the evicted one
    let evicted = build();
    let en = evicted.with_locale("en");
    assert_eq!(en.page("about").t("o"), "Other");
    assert_eq!(en.page("x").t("x"), "X");
    assert_eq!(en.page("about").t("o"), "Other");
    want(&evicted);

    // step 4: the German page evicted and read again → resolved again, the same
    assert_eq!(en.page("x").t("x"), "X");
    want(&evicted);
}

/// Given two locales that fall back to each other and lazy pages that link across, when
/// each page is read first on a fresh catalog, then loading never recurses into the pair
/// being loaded (no deadlock, no panic) and each page resolves the links into the other
/// locale's root.
#[test]
fn a_fallback_cycle_of_lazy_pairs_loads() {
    let files = Locales::new()
        .file("en.json", r#"{"brand": "Acme"}"#)
        .file("de.json", r#"{"marke": "Akme"}"#)
        .file("pages/p/en.json", r#"{"a": "A @:marke", "b": "B"}"#)
        .file("pages/p/de.json", r#"{"c": "C @:brand", "d": "@:b"}"#);
    let build = || {
        I18n::builder()
            .locale("de")
            .fallback_for("de", "en")
            .fallback_for("en", "de")
            .linked_messages(true)
            .lazy_files(files.files())
            .build()
            .unwrap()
    };
    // step 1: German page first
    let i = build();
    assert_eq!(i.page("p").t("c"), "C Acme");
    assert_eq!(i.page("p").t("d"), "B");
    assert_eq!(i.with_locale("en").page("p").t("a"), "A Akme");
    // step 2: English page first
    let i = build();
    assert_eq!(i.with_locale("en").page("p").t("a"), "A Akme");
    assert_eq!(i.page("p").t("c"), "C Acme");
}

/// The remote twin of [`brand_files`].
fn brand_remote() -> I18n {
    I18n::builder()
        .locale("de")
        .fallback("en")
        .linked_messages(true)
        .remote(["en.json", "de.json", "pages/about/de.json"])
        .build()
        .unwrap()
}

/// Given remote German and English files where German texts link the English brand, when
/// one `ensure_async` fetches them and the network answers in every order, then the answers
/// are the same as Nuxt's ("Acme", "About Acme"): the call shows its files together, the
/// weakest layer first, so a German pair never resolves before the English root is in.
#[test]
fn remote_links_resolve_the_same_in_any_arrival_order() {
    let files = brand_files();
    let body = |path: &str| {
        files
            .files()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| b.to_vec())
            .unwrap()
    };
    let paths = ["en.json", "de.json", "pages/about/de.json"];
    let orders = [[1, 2, 0], [0, 1, 2], [2, 0, 1], [1, 0, 2]];
    for order in orders {
        let i18n = brand_remote();
        let net = FakeFetch::new();
        let mut ex = Executor::new();
        // step 1: one call for the German about page → three requests
        let done = ex.spawn(i18n.ensure_async("de", Some("about"), net.fetch()));
        ex.run_until_stalled();
        assert_eq!(net.pending().len(), 3);
        // step 2: answers in this order → nothing shown before the last one
        for (n, &at) in order.iter().enumerate() {
            net.answer(paths[at], Ok(&body(paths[at])));
            ex.run_until_stalled();
            assert_eq!(i18n.version(), u64::from(n == 2), "order {order:?}");
        }
        done.borrow_mut().take().unwrap().unwrap();
        // step 3: the answers of the whole view
        assert_eq!(i18n.t("q"), "Acme", "order {order:?}");
        assert_eq!(i18n.page("about").t("p"), "About Acme", "order {order:?}");
    }
}

/// Given a remote root with a library layer (`extend`) whose text links a builder message,
/// when the app reads it before the file arrives, then the link is resolved there too
/// (base and messages are a view of their own until the file comes); messages added at run
/// time over the waiting pair are not resolved when the file arrives, as in Nuxt.
#[test]
fn a_remote_pair_resolves_its_partial_view_but_not_runtime_messages() {
    let i18n = I18n::builder()
        .linked_messages(true)
        .extend([("en.json", br#"{"lib": "@:own"}"#.as_slice())])
        .messages("en", None, r#"{"own": "O"}"#)
        .remote(["en.json"])
        .build()
        .unwrap();
    // step 1: before the file → the library link resolves over the messages
    assert_eq!((i18n.t("lib"), i18n.t("own")), ("O".into(), "O".into()));
    // step 2: a runtime message with a link waits over the pair
    i18n.add_messages("en", None, r#"{"x": "@:t"}"#).unwrap();
    assert_eq!(i18n.t("x"), "@:t");
    // step 3: the file arrives → its own links resolve, the runtime message stays as written
    i18n.add_file("en.json", br#"{"t": "T", "y": "@:t"}"#)
        .unwrap();
    assert_eq!(
        [i18n.t("t"), i18n.t("y"), i18n.t("x"), i18n.t("lib")],
        ["T", "T", "@:t", "O"]
    );
}
