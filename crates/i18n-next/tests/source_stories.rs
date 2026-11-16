//! Stories of the M3 sources and lifecycle (DESIGN 3.3, 7.4, 8b.4, 8b.16): lazy and remote
//! files, `extend`, eviction, `reload`, `subscribe`. One event stream per test,
//! Given/When/Then in the doc comment, `// step n: action → check` in the body. Remote
//! fetches go through `FakeFetch` and the manual `Executor`: nothing sleeps.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{Change, Error, I18n};
use std::sync::{Arc, Mutex};
use support::{Executor, FakeFetch, Locales, loads};

/// Records every `Change` of an instance.
fn changes(i18n: &I18n) -> (Arc<Mutex<Vec<Change>>>, i18n_next::Subscription) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let sub = i18n.subscribe(move |c| log.lock().unwrap().push(c.clone()));
    (seen, sub)
}

fn versions(seen: &Mutex<Vec<Change>>) -> Vec<u64> {
    seen.lock().unwrap().iter().map(|c| c.version).collect()
}

/// Given a `lazy_dir` with a root and two pages, when the app builds and reads, then
/// nothing is read at `build`, each file is read by the first lookup that reaches it and
/// never again (a file deleted afterwards still answers), and a page nobody reads stays
/// unread.
#[test]
fn lazy_file_is_read_once() {
    let dir = Locales::new()
        .file("en.json", r#"{"title": "App", "ok": "OK"}"#)
        .file("pages/settings/en.json", r#"{"title": "Settings"}"#)
        .file("pages/about/en.json", r#"{"title": "About"}"#)
        .dir("lazy_file_is_read_once");
    let i18n = I18n::builder().lazy_dir(&dir).build().unwrap();
    let (seen, _sub) = changes(&i18n);

    // step 1: build lists the pairs only → pages known, nothing loaded
    assert_eq!(i18n.pages(), ["about", "settings"]);
    assert_eq!(loads(&i18n), 0);

    // step 2: a page hit reads the page file only (the key is there)
    assert_eq!(i18n.page("settings").t("title"), "Settings");
    assert_eq!(loads(&i18n), 1);

    // step 3: a page miss reaches the root layer → the root file is read
    assert_eq!(i18n.page("settings").t("ok"), "OK");
    assert_eq!(loads(&i18n), 2);

    // step 4: the files are gone from disk → the loaded pairs still answer, no re-read
    std::fs::remove_file(dir.join("pages/settings/en.json")).unwrap();
    std::fs::remove_file(dir.join("en.json")).unwrap();
    assert_eq!(i18n.page("settings").t("title"), "Settings");
    assert_eq!(i18n.t("title"), "App");
    assert_eq!(loads(&i18n), 2);

    // step 5: `about` was never read; a lazy load changes no answer → no Change, version 0
    assert_eq!(i18n.page("about").t("title"), "About");
    assert_eq!(loads(&i18n), 3);
    assert_eq!(i18n.version(), 0);
    assert!(seen.lock().unwrap().is_empty());
}

/// Given embedded `lazy_files` with a broken page among them, when the broken page is
/// read, then `build` did not fail, the page answers from the root, and it is not parsed
/// again on every lookup (one failed attempt, counted as no load).
#[test]
fn broken_lazy_file_is_skipped_once() {
    let i18n = I18n::builder()
        .lazy_files([
            ("en.json", br#"{"title": "App"}"#.as_slice()),
            ("pages/bad/en.json", b"{ nope"),
        ])
        .build()
        .unwrap();
    // step 1: the broken page reads the root
    assert_eq!(i18n.page("bad").t("title"), "App");
    assert_eq!(i18n.page("bad").t("title"), "App");
    // step 2: only the root counted as loaded
    assert_eq!(loads(&i18n), 1);
}

const MANIFEST: [&str; 4] = [
    "en.json",
    "de.json",
    "pages/cart/de.json",
    "pages/cart/en.json",
];

fn remote() -> I18n {
    I18n::builder()
        .locale("de")
        .fallback("en")
        .remote(MANIFEST)
        .build()
        .unwrap()
}

/// Given a remote manifest and a fake network, when `ensure_async` asks for the cart of
/// `de`, and the root answers while the cart page fails, then the root is in, the page
/// stays missing and answers fall back, the call returns `Error::Fetch` with the path,
/// and the next `ensure_async` fetches only the page again. What one call fetched is
/// shown together when it ends.
#[test]
fn remote_with_a_fetch_error() {
    let i18n = remote();
    let net = FakeFetch::new();
    let (seen, _sub) = changes(&i18n);

    // step 1: nothing is in → the chain's four files are missing
    assert_eq!(
        i18n.missing_files("de", Some("cart")),
        [
            "de.json",
            "pages/cart/de.json",
            "en.json",
            "pages/cart/en.json"
        ]
    );
    assert_eq!(i18n.t("hi"), "hi");

    // step 2: ensure → four requests at once
    let mut ex = Executor::new();
    let result = ex.spawn(i18n.ensure_async("de", Some("cart"), net.fetch()));
    ex.run_until_stalled();
    assert_eq!(net.pending().len(), 4);

    // step 3: answers in another order, the cart page of de fails
    net.answer("en.json", Ok(br#"{"hi": "Hi", "total": "Total"}"#));
    net.answer("pages/cart/de.json", Err("503"));
    net.answer("pages/cart/en.json", Ok(br#"{"title": "Cart"}"#));
    net.answer("de.json", Ok(br#"{"hi": "Hallo"}"#));
    ex.run_until_stalled();
    let err = result.borrow_mut().take().unwrap().unwrap_err();
    assert!(
        matches!(&err, Error::Fetch { path, message } if path == "pages/cart/de.json" && message == "503"),
        "{err:?}"
    );

    // step 4: what arrived is visible, all at once when the call ended (one Change); the
    // failed page falls back
    assert_eq!(versions(&seen), [1]);
    let cart = i18n.page("cart");
    assert_eq!(
        (cart.t("hi"), cart.t("title"), cart.t("total")),
        ("Hallo".into(), "Cart".into(), "Total".into())
    );
    assert_eq!(
        i18n.missing_files("de", Some("cart")),
        ["pages/cart/de.json"]
    );

    // step 5: ensure again → one request, for the page only
    let result = ex.spawn(i18n.ensure_async("de", Some("cart"), net.fetch()));
    ex.run_until_stalled();
    assert_eq!(net.pending(), ["pages/cart/de.json"]);
    net.answer("pages/cart/de.json", Ok(br#"{"title": "Warenkorb"}"#));
    ex.run_until_stalled();
    result.borrow_mut().take().unwrap().unwrap();
    assert_eq!(cart.t("title"), "Warenkorb");
    assert_eq!(net.count("pages/cart/de.json"), 2);
    assert_eq!(net.total(), 5);

    // step 6: everything in → ensure finishes without a request
    let result = ex.spawn(i18n.ensure_async("de", Some("cart"), net.fetch()));
    ex.run_until_stalled();
    assert!(matches!(*result.borrow(), Some(Ok(()))));
    assert_eq!(net.total(), 5);
}

/// Given two `ensure_async` calls for overlapping files, when they run together, then
/// each file is fetched once, the second call waits for the first one's fetch, and both
/// complete; when the leading call is dropped mid-fetch, the waiting call fetches it.
#[test]
fn concurrent_ensure_fetches_each_file_once() {
    let i18n = remote();
    let net = FakeFetch::new();
    let mut ex = Executor::new();

    // step 1: root of de, then the cart of de → de.json is shared
    let a = ex.spawn(i18n.ensure_async("de", None, net.fetch()));
    let b = ex.spawn(i18n.ensure_async("de", Some("cart"), net.fetch()));
    ex.run_until_stalled();
    assert_eq!(net.count("de.json"), 1);
    assert_eq!(net.count("en.json"), 1);
    assert_eq!(net.total(), 4);

    // step 2: answer the shared file → the first call still waits for en.json
    net.answer("de.json", Ok(br#"{"a": "A"}"#));
    ex.run_until_stalled();
    assert!(a.borrow().is_none());
    for p in ["en.json", "pages/cart/de.json", "pages/cart/en.json"] {
        net.answer(p, Ok(b"{}"));
    }
    ex.run_until_stalled();
    assert!(matches!(*a.borrow(), Some(Ok(()))));
    assert!(matches!(*b.borrow(), Some(Ok(()))));
    assert_eq!(net.total(), 4);

    // step 3: a leader dropped mid-fetch hands the file to the one waiting for it
    let i18n = remote();
    let net = FakeFetch::new();
    let mut lead = Executor::new();
    let mut wait = Executor::new();
    let first = lead.spawn(i18n.ensure_async("en", None, net.fetch()));
    lead.run_until_stalled();
    let second = wait.spawn(i18n.ensure_async("en", None, net.fetch()));
    wait.run_until_stalled();
    assert_eq!(net.pending(), ["en.json"]);
    drop(lead);
    assert!(first.borrow().is_none());
    wait.run_until_stalled();
    assert_eq!(net.pending(), ["en.json", "en.json"]);
    net.answer_at(1, Ok(br#"{"x": "X"}"#));
    wait.run_until_stalled();
    assert!(matches!(*second.borrow(), Some(Ok(()))));
    assert_eq!(i18n.with_locale("en").t("x"), "X");
}

/// Given a remote manifest, when `add_file` gets a path the manifest does not list or
/// bytes that are not a JSON object, then it fails and nothing changes; a known file
/// added twice counts once.
#[test]
fn add_file_takes_names_only_from_the_manifest() {
    let i18n = remote();
    let (seen, _sub) = changes(&i18n);
    // step 1: unknown names, traversal and absolute paths → UnknownFile
    for path in ["fr.json", "../etc/passwd", "/en.json", "pages/cart/fr.json"] {
        let err = i18n.add_file(path, b"{}").unwrap_err();
        assert!(
            matches!(&err, Error::UnknownFile(p) if p == path),
            "{err:?}"
        );
    }
    // step 2: bad bytes → the JSON error with the path, still missing
    let err = i18n.add_file("en.json", b"[1]").unwrap_err();
    assert!(matches!(&err, Error::NotObject { path } if path == "en.json"));
    assert!(
        i18n.missing_files("en", None)
            .contains(&"en.json".to_owned())
    );
    // step 3: Windows separators are the same path; a second add is skipped
    i18n.add_file("pages\\cart\\en.json", br#"{"t": "one"}"#)
        .unwrap();
    i18n.add_file("pages/cart/en.json", br#"{"t": "two"}"#)
        .unwrap();
    assert_eq!(i18n.with_locale("en").page("cart").t("t"), "one");
    assert_eq!(versions(&seen), [1]);
    assert_eq!(loads(&i18n), 1);
}

/// Given messages added to a remote page before its file arrives, when the file comes,
/// then the messages stay over the file, as they would over an eager file.
#[test]
fn messages_wait_over_a_remote_file() {
    let i18n = I18n::builder()
        .remote(["en.json"])
        .messages("en", None, r#"{"a": "builder"}"#)
        .build()
        .unwrap();
    // step 1: messages before the file → visible at once
    i18n.add_messages("en", None, r#"{"b": "runtime"}"#)
        .unwrap();
    assert_eq!(
        (i18n.t("a"), i18n.t("b")),
        ("builder".into(), "runtime".into())
    );
    assert_eq!(i18n.missing_files("en", None), ["en.json"]);
    // step 2: the file arrives → its keys appear, the messages still win
    i18n.add_file("en.json", br#"{"a": "file", "b": "file", "c": "file"}"#)
        .unwrap();
    assert_eq!(
        [i18n.t("a"), i18n.t("b"), i18n.t("c")],
        ["builder", "runtime", "file"]
    );
}

/// Given two libraries and the app's own files, when the catalog is built, then layers
/// merge per `(locale, page)` deep: a later library over an earlier one, the app's files
/// over both, `messages` over everything.
#[test]
fn extend_order_app_wins() {
    let lib_a: [(&str, &[u8]); 2] = [
        (
            "en.json",
            br#"{"ui": {"ok": "A ok", "cancel": "A cancel", "help": "A help"}}"#,
        ),
        (
            "pages/dialog/en.json",
            br#"{"title": "A dialog", "x": "A x"}"#,
        ),
    ];
    let lib_b: [(&str, &[u8]); 1] = [(
        "en.json",
        br#"{"ui": {"cancel": "B cancel", "only_b": "B"}}"#,
    )];
    let i18n = I18n::builder()
        .extend(lib_a)
        .extend(lib_b)
        .files([
            ("en.json", br#"{"ui": {"ok": "App ok"}}"#.as_slice()),
            ("pages/dialog/en.json", br#"{"title": "App dialog"}"#),
        ])
        .messages("en", None, r#"{"ui": {"help": "messages help"}}"#)
        .build()
        .unwrap();
    // step 1: the app beats both libraries; B beats A; objects merge, nothing is lost
    assert_eq!(i18n.t("ui.ok"), "App ok");
    assert_eq!(i18n.t("ui.cancel"), "B cancel");
    assert_eq!(i18n.t("ui.only_b"), "B");
    assert_eq!(i18n.t("ui.help"), "messages help");
    // step 2: per page too
    let dialog = i18n.page("dialog");
    assert_eq!(
        (dialog.t("title"), dialog.t("x")),
        ("App dialog".into(), "A x".into())
    );
    // step 3: a library's page is a known page even when the app has none
    let only_lib = I18n::builder().extend(lib_a).build().unwrap();
    assert_eq!(only_lib.pages(), ["dialog"]);
    // step 4: extend goes under lazy app files too
    let lazy = I18n::builder()
        .extend(lib_a)
        .lazy_files([("pages/dialog/en.json", br#"{"title": "Lazy"}"#.as_slice())])
        .build()
        .unwrap();
    assert_eq!(
        (lazy.page("dialog").t("title"), lazy.page("dialog").t("x")),
        ("Lazy".into(), "A x".into())
    );
}

/// Given lazy pages with `evict_pages(1)` and a pinned `tray`, when pages are read in
/// turn, then at most one unpinned page stays loaded, an evicted page answers the same
/// when read again (one more load), the pinned page and the root are never evicted, and
/// eviction sends no `Change` and keeps the version (the protocol's URLs stay valid).
#[test]
fn evict_and_pin() {
    let mut files: Vec<(String, String)> = vec![("en.json".into(), r#"{"root": "R"}"#.into())];
    for p in ["a", "b", "tray"] {
        files.push((format!("pages/{p}/en.json"), format!(r#"{{"t": "{p}"}}"#)));
    }
    let i18n = I18n::builder()
        .lazy_files(files.iter().map(|(p, b)| (p, b)))
        .evict_pages(1)
        .pin_page("tray")
        .build()
        .unwrap();
    let (seen, _sub) = changes(&i18n);

    // step 1: tray and a → both stay (tray is pinned)
    assert_eq!(i18n.page("tray").t("t"), "tray");
    assert_eq!(i18n.page("a").t("t"), "a");
    assert_eq!((loads(&i18n), i18n.version()), (2, 0));

    // step 2: b → a leaves the next snapshot, no Change, the same version
    assert_eq!(i18n.page("b").t("t"), "b");
    assert_eq!((loads(&i18n), i18n.version()), (3, 0));
    assert!(seen.lock().unwrap().is_empty());

    // step 3: tray and b still loaded (no load), the root read once
    assert_eq!(i18n.page("tray").t("t"), "tray");
    assert_eq!(i18n.page("b").t("root"), "R");
    assert_eq!(loads(&i18n), 4);

    // step 4: a again → same answer, read again, b evicted
    assert_eq!(i18n.page("a").t("t"), "a");
    assert_eq!((loads(&i18n), i18n.version()), (5, 0));
    assert_eq!(i18n.page("b").t("t"), "b");
    assert_eq!(loads(&i18n), 6);

    // step 5: a remote page evicted is missing again until the next ensure
    let i18n = I18n::builder()
        .remote(["pages/a/en.json", "pages/b/en.json"])
        .evict_pages(1)
        .build()
        .unwrap();
    i18n.add_file("pages/a/en.json", br#"{"t": "a"}"#).unwrap();
    i18n.add_file("pages/b/en.json", br#"{"t": "b"}"#).unwrap();
    assert_eq!(i18n.missing_files("en", Some("a")), ["pages/a/en.json"]);
    assert_eq!(i18n.page("a").t("t"), "t");
    assert_eq!(i18n.page("b").t("t"), "b");
}

/// Given a directory and a reader holding a value, when a file changes on disk, then
/// lookups answer the old text until `reload`, a reload that hits a broken file fails and
/// keeps the old catalog, and a good reload swaps everything at once (version + 1, one
/// `Change`), lazy pages included; a remote file is missing again after `reload`.
#[test]
fn reload_keeps_old_answers_until_the_swap() {
    let dir = Locales::new()
        .file("en.json", r#"{"a": "old", "keep": "same"}"#)
        .dir("reload_keeps_old_answers");
    let lazy = Locales::new()
        .file("pages/p/en.json", r#"{"t": "old page"}"#)
        .dir("reload_keeps_old_answers_lazy");
    let i18n = I18n::builder()
        .dir(&dir)
        .lazy_dir(&lazy)
        .messages("en", None, r#"{"m": "builder"}"#)
        .build()
        .unwrap();
    let (seen, _sub) = changes(&i18n);
    let held = i18n.get("a").unwrap();
    assert_eq!(i18n.page("p").t("t"), "old page");
    i18n.add_messages("en", None, r#"{"m": "runtime"}"#)
        .unwrap();

    // step 1: the files change → still the old answers
    std::fs::write(dir.join("en.json"), r#"{"a": "new", "keep": "same"}"#).unwrap();
    std::fs::write(lazy.join("pages/p/en.json"), r#"{"t": "new page"}"#).unwrap();
    assert_eq!(i18n.t("a"), "old");

    // step 2: a broken file → reload fails with its path, nothing changes
    std::fs::write(dir.join("en.json"), "{").unwrap();
    let err = i18n.reload().unwrap_err();
    assert!(matches!(&err, Error::Json { path, .. } if path.ends_with("en.json")));
    assert_eq!((i18n.t("a"), i18n.version()), ("old".into(), 1));

    // step 3: fixed → one swap: new root, new lazy page, builder messages kept, runtime
    // ones dropped
    std::fs::write(dir.join("en.json"), r#"{"a": "new", "keep": "same"}"#).unwrap();
    i18n.reload().unwrap();
    assert_eq!(i18n.t("a"), "new");
    assert_eq!(i18n.page("p").t("t"), "new page");
    assert_eq!(i18n.t("m"), "builder");
    assert_eq!(&*held, "old");
    assert_eq!(versions(&seen), [1, 2]);

    // step 4: remote files are fetched again after a reload
    let i18n = remote();
    i18n.add_file("en.json", br#"{"x": "X"}"#).unwrap();
    assert_eq!(i18n.with_locale("en").t("x"), "X");
    i18n.reload().unwrap();
    assert_eq!(i18n.with_locale("en").t("x"), "x");
    assert_eq!(i18n.missing_files("en", None), ["en.json"]);
}

/// Given two listeners, the first of which panics, when the catalog changes, then the
/// second still hears every change, in order, with the active locale; a write that
/// changes nothing sends nothing; after its `Subscription` is dropped a listener hears
/// nothing more.
#[test]
fn subscribe_unsubscribe_and_a_panicking_listener() {
    let i18n = I18n::builder()
        .locale("en")
        .messages("en", None, r#"{"a": "A"}"#)
        .messages("de", None, r#"{"a": "B"}"#)
        .remote(["pages/x/en.json"])
        .build()
        .unwrap();
    let bomb = i18n.subscribe(|_| panic!("listener bug"));
    let (seen, sub) = changes(&i18n);

    // step 1: set_locale to another locale → a Change with that locale
    i18n.set_locale("de").unwrap();
    // step 2: the same locale again → nothing
    i18n.set_locale("de").unwrap();
    // step 3: add_messages, add_file, reload → one each
    i18n.add_messages("de", None, r#"{"b": "C"}"#).unwrap();
    i18n.add_file("pages/x/en.json", b"{}").unwrap();
    i18n.reload().unwrap();
    let got: Vec<(String, u64)> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|c| (c.locale.to_string(), c.version))
        .collect();
    assert_eq!(
        got,
        [
            ("de".to_owned(), 1),
            ("de".to_owned(), 2),
            ("de".to_owned(), 3),
            ("de".to_owned(), 4)
        ]
    );

    // step 4: unsubscribe → nothing more; the panicking one is dropped too
    drop(sub);
    drop(bomb);
    i18n.set_locale("en").unwrap();
    assert_eq!(seen.lock().unwrap().len(), 4);

    // step 5: a listener may subscribe from inside a notification (it runs over a copy)
    let inner_seen = Arc::new(Mutex::new(0));
    let (i2, count) = (i18n.clone(), inner_seen.clone());
    let subs = Arc::new(Mutex::new(Vec::new()));
    let keep = subs.clone();
    let _outer = i18n.subscribe(move |_| {
        let count = count.clone();
        keep.lock()
            .unwrap()
            .push(i2.subscribe(move |_| *count.lock().unwrap() += 1));
    });
    i18n.set_locale("de").unwrap();
    assert_eq!(*inner_seen.lock().unwrap(), 0);
    i18n.set_locale("en").unwrap();
    assert_eq!(*inner_seen.lock().unwrap(), 1);
    drop(subs);
}

/// Given `pages(false)` with remote files, when the files of one locale arrive one by
/// one, then the root of that locale shows nothing until all of them are in (its page
/// files fold into it), then all at once.
#[test]
fn folded_remote_root_waits_for_all_its_files() {
    let i18n = I18n::builder()
        .pages(false)
        .remote(["en.json", "pages/a/en.json"])
        .build()
        .unwrap();
    assert_eq!(
        i18n.missing_files("en", None),
        ["en.json", "pages/a/en.json"]
    );
    // step 1: one of two → not visible yet, no new version
    i18n.add_file("pages/a/en.json", br#"{"k": "page"}"#)
        .unwrap();
    assert_eq!((i18n.t("k"), i18n.version()), ("k".into(), 0));
    assert_eq!(i18n.missing_files("en", None), ["en.json"]);
    // step 2: both → the page file wins over the root, as in an eager build
    i18n.add_file("en.json", br#"{"k": "root", "r": "root"}"#)
        .unwrap();
    assert_eq!(
        (i18n.t("k"), i18n.t("r"), i18n.version()),
        ("page".into(), "root".into(), 1)
    );
}

/// `n` distinct leaves, then the same non-empty object `n` times: every repeat replaces a
/// container with children (the M3 audit's adversarial input).
fn dup_containers(n: usize) -> String {
    let mut s = String::from("{");
    for k in 0..n {
        s.push_str(&format!("\"u{k}\":\"v\","));
    }
    for k in 0..n {
        s.push_str(&format!(
            "{}\"d\":{{\"a{k}\":\"b\"}}",
            if k > 0 { "," } else { "" }
        ));
    }
    s + "}"
}

/// Given documents where a key repeats with a non-empty object or array (a duplicate key
/// replaces what came before, as `JSON.parse` does), when they are parsed and merged, then
/// only the last value and its own children remain, at every depth; and 100k repeats in a
/// 2.9 MB file, or a merge replacing 100k non-empty objects, take linear work (a fraction of
/// a second; quadratic, half as many took 33 s here: the test's time limit catches a return).
#[test]
fn repeated_containers_are_replaced_in_linear_work() {
    // step 1: small duplicates at several depths, arrays and the empty key included
    let i18n = I18n::builder()
        .files([(
            "en.json",
            br#"{"x": {"y": {"z": "1", "l": [1, [2]]}}, "x": {"y": {"w": "2"}}, "x": {"q": "3", "y": {"v": "4"}},
                 "l": [1, [2, 3]], "l": [4], "": {"b": "1"}, "": {"c": "2"}, "keep": {"k": "K"}}"#
                .as_slice(),
        )])
        .build()
        .unwrap();
    for (key, want) in [
        ("x.q", "3"),
        ("x.y.v", "4"),
        ("l.0", "4"),
        (".c", "2"),
        ("keep.k", "K"),
    ] {
        assert_eq!(i18n.t(key), want, "{key}");
    }
    for gone in ["x.y.z", "x.y.w", "x.y.l", "x.y.l.1.0", "l.1", "l.1.0", ".b"] {
        assert!(i18n.try_t(gone).is_none() && !i18n.has(gone), "{gone}");
    }

    // step 2: 100k repeats of a non-empty object after 100k keys, through `add_file`
    let big = dup_containers(100_000);
    let remote = I18n::builder().remote(["en.json"]).build().unwrap();
    remote.add_file("en.json", big.as_bytes()).unwrap();
    assert_eq!(remote.t("d.a99999"), "b");
    assert!(!remote.has("d.a99998"));
    assert_eq!(remote.t("u99999"), "v");

    // step 3: a merge that replaces 100k non-empty objects with strings
    let mut objects = String::from("{");
    for k in 0..100_000 {
        objects.push_str(&format!(
            "{}\"o{k}\":{{\"x\":\"y\"}}",
            if k > 0 { "," } else { "" }
        ));
    }
    objects.push('}');
    let merged = I18n::builder()
        .messages("en", None, &objects)
        .build()
        .unwrap();
    merged
        .add_messages("en", None, &objects.replace("{\"x\":\"y\"}", "\"z\""))
        .unwrap();
    assert_eq!(merged.t("o99999"), "z");
    assert!(!merged.has("o99999.x"));
}

/// Given lazy pages with `evict_pages(2)`, when pages are read a, b, a, c (and b, a, b, c
/// on another catalog), then the page read longest ago goes (b, then a): a page read again
/// after the latest load counts as newer than it, so reading the one kept loads nothing.
#[test]
fn eviction_drops_the_least_recently_used_page() {
    let files: Vec<(String, String)> = ["a", "b", "c"]
        .iter()
        .map(|p| (format!("pages/{p}/en.json"), format!(r#"{{"t": "{p}"}}"#)))
        .collect();
    for (x, y) in [("a", "b"), ("b", "a")] {
        let i18n = I18n::builder()
            .lazy_files(files.iter().map(|(p, b)| (p, b)))
            .evict_pages(2)
            .build()
            .unwrap();
        // step 1: x, y, x again (a hit), then c → three loads, y evicted
        for p in [x, y, x, "c"] {
            assert_eq!(i18n.page(p).t("t"), p);
        }
        assert_eq!(loads(&i18n), 3);
        // step 2: x is still in → no load; y comes back → one load
        assert_eq!(i18n.page(x).t("t"), x);
        assert_eq!(loads(&i18n), 3, "{x} was evicted instead of {y}");
        assert_eq!(i18n.page(y).t("t"), y);
        assert_eq!(loads(&i18n), 4);
    }
}

/// Given a `missing` handler that writes to the catalog and a link that stays broken, when
/// the catalog reloads, then the handler runs after the writer is released: no deadlock,
/// and its write lands.
#[test]
fn a_missing_handler_may_write_during_reload() {
    let slot: Arc<Mutex<Option<I18n>>> = Arc::default();
    let handle = slot.clone();
    let i18n = I18n::builder()
        .linked_messages(true)
        .messages("en", None, r#"{"a": "@:nope"}"#)
        .missing(move |locale, key, _| {
            let i18n = handle.lock().unwrap().clone();
            if let Some(i18n) = i18n {
                i18n.add_messages(locale, None, &format!(r#"{{"seen_{key}": "yes"}}"#))
                    .unwrap();
            }
        })
        .build()
        .unwrap();
    *slot.lock().unwrap() = Some(i18n.clone());
    // step 1: reload → the broken link is reported, the handler writes
    i18n.reload().unwrap();
    assert_eq!(i18n.t("seen_a"), "yes");
    // the handler held a clone: drop it to free the catalog
    slot.lock().unwrap().take();
}

/// Given two listeners, the first of which drops the second's subscription, when the
/// catalog changes, then the second is not called by that notification nor later.
#[test]
fn a_listener_dropped_during_a_notification_is_not_called() {
    let i18n = I18n::builder()
        .messages("en", None, "{}")
        .messages("de", None, "{}")
        .build()
        .unwrap();
    let second: Arc<Mutex<Option<i18n_next::Subscription>>> = Arc::default();
    let drop_it = second.clone();
    let _first = i18n.subscribe(move |_| drop(drop_it.lock().unwrap().take()));
    let calls = Arc::new(Mutex::new(0));
    let count = calls.clone();
    *second.lock().unwrap() = Some(i18n.subscribe(move |_| *count.lock().unwrap() += 1));
    // step 1: a change → the first runs and drops the second before its turn
    i18n.set_locale("de").unwrap();
    assert_eq!(*calls.lock().unwrap(), 0);
    // step 2: later changes → still nothing
    i18n.set_locale("en").unwrap();
    assert_eq!(*calls.lock().unwrap(), 0);
}

/// Given two `ensure_async` calls for the same file, when the one fetch fails, then both
/// calls get its error: the waiting call reports the leader's failure, not success.
#[test]
fn a_waiting_call_gets_the_leaders_error() {
    let i18n = remote();
    let net = FakeFetch::new();
    let mut ex = Executor::new();
    // step 1: two calls, one request
    let a = ex.spawn(i18n.ensure_async("en", None, net.fetch()));
    let b = ex.spawn(i18n.ensure_async("en", None, net.fetch()));
    ex.run_until_stalled();
    assert_eq!(net.pending(), ["en.json"]);
    // step 2: it fails → both calls fail with the path and the message
    net.answer("en.json", Err("offline"));
    ex.run_until_stalled();
    for out in [a, b] {
        let err = out.borrow_mut().take().unwrap().unwrap_err();
        assert!(
            matches!(&err, Error::Fetch { path, message } if path == "en.json" && message == "offline"),
            "{err:?}"
        );
    }
    assert_eq!(i18n.missing_files("en", None), ["en.json"]);
}
