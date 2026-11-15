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

    // case: tc() → exactly 1 block (the result) on every path: a plural by category
    // (ru, unmarked), by index, `=0 `, a big f64 count (shortest digits on the stack), one
    // trimmed form, a number, a subtree and a miss (the key), tc_with with 4 parameters
    // and a default, a view in another locale (its tag resolved on the call).
    let ru = I18n::builder()
        .locale("ru")
        .fallback("en")
        .messages(
            "ru",
            None,
            r#"{"apples": "нет | {count} яблоко | {count} яблока | {count} яблок",
                "car": "car | cars", "marked": "=0 none | {count} one | {count} other",
                "plain": "  {count} things {name}  ", "n": 5, "tree": {"a": "b"},
                "greet": "{name} {n} {a} {b} | {name}: {count} | {name}, {count}, {a}{b}"}"#,
        )
        .messages("en", None, r#"{"apples": "one | {count} apples"}"#)
        .missing(|_, _, _| {})
        .build()
        .unwrap();
    let en = ru.with_locale("en");
    let _ = (ru.tc("apples", 1), en.tc("apples", 2));
    let cases: [(&str, &dyn Fn() -> String); 12] = [
        ("tc unmarked", &|| ru.tc("apples", 21)),
        ("tc index", &|| ru.tc("car", 3)),
        ("tc marked", &|| ru.tc("marked", 1000)),
        ("tc big f64", &|| ru.tc("apples", 2f64.powi(61))),
        ("tc one form", &|| ru.tc("plain", 3)),
        ("tc number", &|| ru.tc("n", 3)),
        ("tc subtree", &|| ru.tc("tree", 3)),
        ("tc miss", &|| ru.tc("nope", 3)),
        ("tc_with", &|| ru.tc_with("greet", 7, &args)),
        ("tc_with long", &|| {
            ru.tc_with(
                "greet",
                i64::MIN,
                &Args::new().str("name", long).str("a", long),
            )
        }),
        ("tc_with miss default", &|| {
            ru.tc_with(
                "nope",
                1.5,
                &Args::new().str("x", long).or("{x} {count} {x}"),
            )
        }),
        ("view tc", &|| en.tc("apples", 21)),
    ];
    for (what, f) in cases {
        let n = blocks(|| {
            std::hint::black_box(f());
        });
        assert_eq!(n, 1, "{what}: {n} blocks for {:?}", f());
    }
    assert_eq!(ru.tc("apples", 21), "21 яблоко");
    assert_eq!(ru.tc("apples", 2f64.powi(61)), "2305843009213694000 яблок");
    assert_eq!(en.tc("apples", 21), "21 apples");

    // case: a dropped instance leaves no snapshot behind in thread caches. Two live
    // threads cache a big catalog, every handle is dropped, then each thread reads another
    // instance → live bytes fall back near the baseline (the reads purge the dead slots;
    // the helper thread stays alive, so no thread-exit destructor frees its cache).
    let big = {
        let mut s = String::from("{");
        for d in 0..20_000 {
            if d > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"key{d}\":\"value number {d}\""));
        }
        s.push('}');
        s
    };
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let (back, done) = std::sync::mpsc::channel::<()>();
    let other = i18n.clone();
    let base = dhat::HeapStats::get().curr_bytes;
    let x = I18n::builder().messages("en", None, &big).build().unwrap();
    drop(big);
    let grown = dhat::HeapStats::get().curr_bytes - base;
    let x2 = x.clone();
    let helper = std::thread::spawn(move || {
        assert_eq!(x2.t("key1"), "value number 1");
        drop(x2);
        back.send(()).unwrap();
        wait.recv().unwrap();
        std::hint::black_box(other.get("nav.home"));
        back.send(()).unwrap();
        wait.recv().unwrap();
    });
    done.recv().unwrap();
    assert_eq!(x.t("key2"), "value number 2");
    drop(x);
    go.send(()).unwrap();
    done.recv().unwrap();
    std::hint::black_box(i18n.get("nav.home"));
    let left = dhat::HeapStats::get().curr_bytes.saturating_sub(base);
    go.send(()).unwrap();
    helper.join().unwrap();
    assert!(grown > 1_000_000, "the catalog is big: {grown} bytes");
    assert!(
        left < grown / 20,
        "{left} of {grown} bytes still live after both threads read another instance"
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
