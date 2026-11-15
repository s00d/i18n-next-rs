//! Allocation counts on the hot path (DESIGN 7.7, dhat in testing mode).
//!
//! dhat allows one profiler per process, so this binary holds a single test that walks
//! every case in order and resets the baseline before each one. An extra allocation is
//! a red test: fix the design, do not raise the number.
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
fn hot_path_allocations() {
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"nav": {"home": "Home"}, "only_en": "English"}"#,
        )
        .messages(
            "de",
            None,
            r#"{"nav": {"home": "Startseite"}, "greet": "Hallo, {name}! {n} neu, {a}{b}"}"#,
        )
        .messages("de", Some("settings"), r#"{"title": "Einstellungen"}"#)
        .build()
        .unwrap();
    let settings = i18n.page("settings");
    // Warm this thread's snapshot cache (and the thread-local registration).
    let _ = i18n.t("nav.home");
    let _ = settings.t("title");

    let _profiler = dhat::Profiler::builder().testing().build();

    // case: version() → 0 blocks (a `&'static str`).
    let n = blocks(|| {
        std::hint::black_box(i18n_next::version());
    });
    dhat::assert_eq!(n, 0);

    // case: get() hit, root and through the fallback → 0 blocks (a shared `Arc<str>`).
    let n = blocks(|| {
        std::hint::black_box(i18n.get("nav.home"));
        std::hint::black_box(i18n.get("only_en"));
        std::hint::black_box(settings.get("title"));
    });
    dhat::assert_eq!(n, 0);

    // case: has() / try_t() miss → 0 blocks.
    let n = blocks(|| {
        std::hint::black_box(i18n.has("nav.home"));
        std::hint::black_box(i18n.try_t("nope"));
    });
    dhat::assert_eq!(n, 0);

    // case: t() hit without args → exactly 1 block (the result).
    let n = blocks(|| {
        std::hint::black_box(i18n.t("nav.home"));
    });
    dhat::assert_eq!(n, 1);

    // case: page view t() hit → 1 block.
    let n = blocks(|| {
        std::hint::black_box(settings.t("title"));
    });
    dhat::assert_eq!(n, 1);

    // case: building Args with ≤ 4 parameters → 0 blocks (inline storage).
    let n = blocks(|| {
        let args = Args::new()
            .str("name", "Ann")
            .int("n", 3)
            .float("a", 1.5)
            .bool("b", true);
        std::hint::black_box(&args);
    });
    dhat::assert_eq!(n, 0);

    // case: t_with() with 4 parameters → 1 block (the result, sized once).
    let args = Args::new()
        .str("name", "Ann")
        .int("n", 3)
        .float("a", 1.5)
        .bool("b", true);
    let n = blocks(|| {
        std::hint::black_box(i18n.t_with("greet", &args));
    });
    dhat::assert_eq!(n, 1);
    assert_eq!(i18n.t_with("greet", &args), "Hallo, Ann! 3 neu, 1.5true");

    // case: values much longer than their placeholders (a 40-char name, the longest
    // integer) → still 1 block: the result is sized for the values up front, so an
    // undersized buffer would show up as a reallocation (dhat counts it as a block).
    let long = "Maximiliane-Theodora von Hohenzollern-Si";
    assert_eq!(long.len(), 40);
    let args = Args::new()
        .str("name", long)
        .int("n", i64::MIN)
        .str("a", long)
        .str("b", long);
    let n = blocks(|| {
        std::hint::black_box(i18n.t_with("greet", &args));
    });
    dhat::assert_eq!(n, 1);
    assert_eq!(
        i18n.t_with("greet", &args),
        format!("Hallo, {long}! {} neu, {long}{long}", i64::MIN)
    );

    // case: building Args with 5 parameters → 1 block (the overflow `Vec`), and t_with
    // with them → 1 block (the result), the fifth value read from the overflow.
    let n = blocks(|| {
        let args = Args::new()
            .str("x", "x")
            .str("name", "Ann")
            .int("n", 3)
            .str("a", "A")
            .str("b", long);
        std::hint::black_box(&args);
    });
    dhat::assert_eq!(n, 1);
    let args = Args::new()
        .str("x", "x")
        .str("name", "Ann")
        .int("n", 3)
        .str("a", "A")
        .str("b", long);
    let n = blocks(|| {
        std::hint::black_box(i18n.t_with("greet", &args));
    });
    dhat::assert_eq!(n, 1);
    assert_eq!(
        i18n.t_with("greet", &args),
        format!("Hallo, Ann! 3 neu, A{long}")
    );

    // case: a miss down the whole chain with a default → 1 block (the interpolated
    // default). Last on purpose: a failure here leaves every case above checked.
    let args = Args::new().int("n", 2).or("{n} items");
    let n = blocks(|| {
        std::hint::black_box(i18n.t_with("nope", &args));
    });
    assert_eq!(i18n.t_with("nope", &args), "2 items");
    dhat::assert_eq!(n, 1);
}
