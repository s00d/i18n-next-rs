//! Linked messages cost nothing at lookup (DESIGN 2.2, 7.7, M3): a resolved message, one
//! resolved for a view from a weaker layer, and a plain message allocate exactly the same
//! (dhat in testing mode; one profiler per process, so one test in this binary).
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{Args, I18n};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Blocks allocated while `f` runs.
fn blocks(f: impl FnOnce()) -> u64 {
    let before = dhat::HeapStats::get().total_blocks;
    f();
    dhat::HeapStats::get().total_blocks - before
}

#[test]
fn linked_lookups_allocate_like_plain_ones() {
    let i18n = I18n::builder()
        .locale("en")
        .fallback("en")
        .linked_messages(true)
        .messages(
            "en",
            None,
            r#"{"brand": "acme", "plain": "Welcome to acme", "title": "Welcome to @:brand",
                "greet": "Hi @:who", "who": "{name}", "hi": "Hi {name}",
                "cart": "none | one @:brand | {count} @:brand", "carts": "none | one acme | {count} acme"}"#,
        )
        .messages("de", None, r#"{"brand": "Akme"}"#)
        .build()
        .unwrap();
    let de = i18n.with_locale("de");
    assert_eq!(de.t("title"), "Welcome to Akme"); // a text stored for the German view
    let _ = i18n.t("plain");
    let ann = Args::new().str("name", "Ann");

    let _profiler = dhat::Profiler::builder().testing().build();

    // case: t → 1 block for a plain, a resolved and a view-resolved message alike.
    for (view, key) in [
        (&i18n.page(""), "plain"),
        (&i18n.page(""), "title"),
        (&de, "title"),
    ] {
        let n = blocks(|| {
            std::hint::black_box(view.t(key));
        });
        dhat::assert_eq!(n, 1, "t({})", key);
    }

    // case: get / has → 0 blocks, linked or not.
    let n = blocks(|| {
        std::hint::black_box(i18n.get("plain"));
        std::hint::black_box(i18n.get("title"));
        std::hint::black_box(de.get("title"));
        std::hint::black_box(de.has("title"));
    });
    dhat::assert_eq!(n, 0);

    // case: t_with on a linked placeholder → 1 block, like a written one.
    for key in ["hi", "greet"] {
        let n = blocks(|| {
            std::hint::black_box(i18n.t_with(key, &ann));
        });
        dhat::assert_eq!(n, 1, "t_with({})", key);
    }

    // case: tc on a plural with links → 1 block, like the same plural written out.
    for key in ["carts", "cart"] {
        let n = blocks(|| {
            std::hint::black_box(i18n.tc(key, 3));
        });
        dhat::assert_eq!(n, 1, "tc({})", key);
    }
    assert_eq!(i18n.tc("cart", 3), i18n.tc("carts", 3));
}
