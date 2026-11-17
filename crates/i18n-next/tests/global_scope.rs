//! The global instance (DESIGN 4.4, 7.3): its own test binary, since `install` is once per
//! process; one test, so its steps run in order.
#![cfg(all(feature = "global", not(target_arch = "wasm32")))]

use i18n_next::{Error, I18n, Key, key, scope, t, tc};

const HI: Key = key!("hi");

/// Given no instance, when texts are shown, then the key stands in and the free functions
/// say so; when an instance is installed, then `t!`, `key!` and the free functions read it,
/// a second `install` fails and keeps the first, a scope overrides it on this thread only.
#[test]
fn global_instance_and_scope() {
    // step 1: before install → the key, no locale, no view, `set_locale` refuses
    assert_eq!(t!("hi"), "hi");
    assert_eq!(tc!("apples", 2), "apples");
    assert_eq!(t!("hi", @or = "Default"), "Default");
    assert_eq!(HI.to_string(), "hi");
    assert!(i18n_next::global().is_none());
    assert_eq!(i18n_next::locale(), None);
    assert!(i18n_next::page("p").is_none());
    assert!(matches!(
        i18n_next::set_locale("de"),
        Err(Error::NotInstalled)
    ));

    // step 2: install through the builder → the global instance answers
    let installed = I18n::builder()
        .locale("en")
        .messages(
            "en",
            None,
            r#"{"hi": "Hi", "apples": "none | one | {count} apples"}"#,
        )
        .messages("de", None, r#"{"hi": "Hallo"}"#)
        .messages("de", Some("p"), r#"{"hi": "Hallo, Seite"}"#)
        .install()
        .unwrap();
    assert_eq!(t!("hi"), "Hi");
    assert_eq!(tc!("apples", 2), "2 apples");
    assert_eq!(HI.to_string(), "Hi");
    assert_eq!(i18n_next::t("hi"), "Hi");

    // step 3: a second install → AlreadyInstalled, the first instance stays
    let other = I18n::builder()
        .messages("en", None, r#"{"hi": "Other"}"#)
        .build()
        .unwrap();
    assert!(matches!(
        i18n_next::install(other),
        Err(Error::AlreadyInstalled)
    ));
    assert!(matches!(
        I18n::builder().install(),
        Err(Error::AlreadyInstalled)
    ));
    assert_eq!(t!("hi"), "Hi");

    // step 4: the free functions switch and read it; a stored text follows
    let stored = t!("hi");
    i18n_next::set_locale("de").unwrap();
    assert_eq!(stored, "Hallo");
    assert_eq!(i18n_next::locale().as_deref(), Some("de"));
    assert_eq!(
        i18n_next::page("p").unwrap().t("hi", str::to_owned),
        "Hallo, Seite"
    );
    assert!(std::ptr::eq(installed, i18n_next::global().unwrap()));
    assert!(matches!(
        i18n_next::set_locale("fr"),
        Err(Error::UnknownLocale(_))
    ));

    // step 5: a scope overrides locale and page of implicit texts, or the instance itself
    scope().locale("en").run(|| {
        assert_eq!(t!("hi"), "Hi");
        assert_eq!(i18n_next::locale().as_deref(), Some("en"));
        // step 6: another thread sees the global instance without this thread's scope
        std::thread::scope(|s| {
            s.spawn(|| assert_eq!(t!("hi"), "Hallo")).join().unwrap();
        });
    });
    scope().page("p").run(|| assert_eq!(stored, "Hallo, Seite"));
    let mine = I18n::builder()
        .messages("en", None, r#"{"hi": "Mine"}"#)
        .build()
        .unwrap();
    scope().instance(mine).run(|| {
        assert_eq!(t!("hi"), "Mine");
        // the free functions act on the scope's instance
        assert_eq!(i18n_next::locale().as_deref(), Some("en"));
        assert!(matches!(
            i18n_next::set_locale("de"),
            Err(Error::UnknownLocale(_))
        ));
    });
    assert_eq!(t!("hi"), "Hallo");
}
