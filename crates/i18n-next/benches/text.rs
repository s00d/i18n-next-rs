//! criterion benches of the M4 API (DESIGN 4.3, 4.4, 7.7): `Text` built, shown and turned
//! into a string with 0 / 1 / 4 / 8 arguments, a plural, html, slots; `t!` against
//! `i18n.t`; `scope().run`, the poll overhead of `wrap`, the global lookup and `negotiate`.
//! Ids follow `tests/support/inventory.rs`: `<owner>/<method>/<case>`, free functions under
//! `global/<fn>/<case>`.

use criterion::{Criterion, criterion_group, criterion_main};
use i18n_next::{I18n, Key, Part, Text, key, scope, t};
use std::fmt::Write as _;
use std::future::Future;
use std::hint::black_box;
use std::pin::Pin;
use std::task::{Context, Waker};

fn catalog() -> I18n {
    I18n::builder()
        .locale("en")
        .fallback("en")
        .messages(
            "en",
            None,
            r#"{"plain": "Settings", "a1": "Hi, {a}!", "a4": "{a} {b} {c} {d}",
                "a8": "{a} {b} {c} {d} {e} {f} {g} {h}",
                "apples": "no apples | one apple | {count} apples",
                "terms": "I accept the {link} and the {policy}", "markup": "<b>{a}</b>",
                "blank": "  "}"#,
        )
        .messages(
            "de",
            None,
            r#"{"plain": "Einstellungen", "a1": "Hallo, {a}!"}"#,
        )
        .messages("en", Some("cart"), r#"{"plain": "Cart"}"#)
        .build()
        .unwrap()
}

fn with_args(i18n: &I18n, n: usize) -> Text {
    let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let key = match n {
        0 => "plain",
        1 => "a1",
        4 => "a4",
        _ => "a8",
    };
    names[..n]
        .iter()
        .fold(i18n.text(key), |t, name| t.arg(*name, "value"))
}

fn text(c: &mut Criterion) {
    let i18n = catalog();
    c.bench_function("i18n/text/key_only", |b| {
        b.iter(|| black_box(&i18n).text(black_box("plain")))
    });
    for n in [0, 1, 4, 8] {
        c.bench_function(&format!("i18n/text/build_args_{n}"), |b| {
            b.iter(|| black_box(with_args(black_box(&i18n), n)))
        });
        let t = with_args(&i18n, n);
        let mut buf = String::with_capacity(256);
        c.bench_function(&format!("text/write_to/display_args_{n}"), |b| {
            b.iter(|| {
                buf.clear();
                black_box(&t).write_to(&mut buf).unwrap();
            })
        });
        c.bench_function(&format!("text/to_string/args_{n}"), |b| {
            b.iter(|| black_box(&t).to_string())
        });
    }
    let mut buf = String::with_capacity(256);
    let shown = i18n.text("a1").arg("a", "Ann");
    c.bench_function("text/write_to/display_fmt", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&shown)).unwrap();
        })
    });
    c.bench_function("text/arg/one", |b| {
        b.iter(|| i18n.text("a1").arg(black_box("a"), black_box(3)))
    });
    c.bench_function("text/args/list_2", |b| {
        b.iter(|| i18n.text("a1").args(black_box(["x", "y"])))
    });
    c.bench_function("text/count/build", |b| {
        b.iter(|| i18n.text("apples").count(black_box(5)))
    });
    let plural = i18n.text("apples").count(21);
    c.bench_function("text/count/to_string", |b| {
        b.iter(|| black_box(&plural).to_string())
    });
    c.bench_function("text/or/miss_default", |b| {
        b.iter(|| i18n.text("nope").or(black_box("Default")).to_string())
    });
    c.bench_function("text/hide_if_empty/blank", |b| {
        b.iter(|| i18n.text("blank").hide_if_empty().resolve())
    });
    c.bench_function("text/plural_rule/custom", |b| {
        b.iter(|| {
            i18n.text("apples")
                .count(black_box(2))
                .plural_rule(|c| Some(c.get(c.key)))
                .to_string()
        })
    });
    c.bench_function("text/page/hit", |b| {
        b.iter(|| i18n.text("plain").page(black_box("cart")).to_string())
    });
    c.bench_function("text/locale/hit", |b| {
        b.iter(|| i18n.text("plain").locale(black_box("de")).to_string())
    });
    c.bench_function("text/html/escaped_arg", |b| {
        b.iter(|| {
            i18n.text("markup")
                .arg("a", black_box("<i>&</i>"))
                .html()
                .to_string()
        })
    });
    let hit = i18n.text("a1").arg("a", "Ann");
    c.bench_function("text/resolve/args_1", |b| {
        b.iter(|| black_box(&hit).resolve())
    });
    let terms = i18n.text("terms");
    c.bench_function("text/parts/two_slots", |b| {
        b.iter(|| black_box(&terms).parts().collect::<Vec<Part>>())
    });
    let link = || "link".to_owned();
    let policy = || "policy".to_owned();
    let slots: [(&str, &dyn Fn() -> String); 2] = [("link", &link), ("policy", &policy)];
    c.bench_function("text/fold/two_slots", |b| {
        b.iter(|| black_box(&terms).fold(str::to_owned, &slots))
    });
    c.bench_function("plural_ctx/get/in_rule", |b| {
        b.iter(|| {
            i18n.text("apples")
                .count(1)
                .plural_rule(|c| Some(c.get(black_box("plain"))))
                .to_string()
        })
    });
    let settings = i18n.page("cart");
    c.bench_function("view/text/page_hit", |b| {
        b.iter(|| black_box(&settings).text("plain").to_string())
    });
}

/// `t!` against `i18n.t` and `i18n.t_with`: the same lookup, the macro builds a `Text`.
fn macros(c: &mut Criterion) {
    let i18n = catalog();
    c.bench_function("i18n/t/macro_baseline", |b| {
        b.iter(|| i18n.t(black_box("plain")))
    });
    c.bench_function("text/to_string/macro_explicit", |b| {
        b.iter(|| t!(i18n => black_box("plain")).to_string())
    });
    c.bench_function("text/to_string/macro_explicit_args_1", |b| {
        b.iter(|| t!(i18n => black_box("a1"), a = "Ann").to_string())
    });
    scope().instance(i18n.clone()).run(|| {
        c.bench_function("text/to_string/macro_scope", |b| {
            b.iter(|| t!(black_box("plain")).to_string())
        });
    });
    const PLAIN: Key = key!("plain");
    c.bench_function("key/new/const", |b| b.iter(|| Key::new(black_box("plain"))));
    scope().instance(i18n.clone()).run(|| {
        c.bench_function("key/args/to_string", |b| {
            b.iter(|| black_box(PLAIN).args().to_string())
        });
    });
}

fn scopes(c: &mut Criterion) {
    let i18n = catalog();
    c.bench_function("global/scope/new", |b| b.iter(|| black_box(scope())));
    c.bench_function("scope/locale/set", |b| {
        b.iter(|| scope().locale(black_box("de")))
    });
    c.bench_function("scope/page/set", |b| {
        b.iter(|| scope().page(black_box("cart")))
    });
    c.bench_function("scope/instance/set", |b| {
        b.iter(|| scope().instance(black_box(&i18n).clone()))
    });
    c.bench_function("scope/run/empty", |b| {
        b.iter(|| scope().run(|| black_box(1)))
    });
    c.bench_function("scope/run/locale_and_text", |b| {
        b.iter(|| {
            scope()
                .locale("de")
                .run(|| i18n.text(black_box("plain")).to_string())
        })
    });
    c.bench_function("scope/wrap/build", |b| {
        b.iter(|| scope().locale("de").wrap(async { black_box(1) }))
    });
    // the poll overhead: one poll of a wrapped future that is pending forever
    let mut cx = Context::from_waker(Waker::noop());
    let mut pending = Box::pin(scope().locale("de").wrap(std::future::pending::<()>()));
    c.bench_function("scope/wrap/poll", |b| {
        b.iter(|| black_box(Pin::as_mut(&mut pending).poll(&mut cx)))
    });
    let mut bare = Box::pin(std::future::pending::<()>());
    c.bench_function("scope/wrap/poll_baseline_unwrapped", |b| {
        b.iter(|| black_box(Pin::as_mut(&mut bare).poll(&mut cx)))
    });
}

/// The global instance: `install` once, then lookups through it; `negotiate`.
#[cfg(feature = "global")]
fn global(c: &mut Criterion) {
    let i18n = catalog();
    i18n_next::install(i18n.clone()).unwrap();
    c.bench_function("global/install/already_installed", |b| {
        b.iter(|| i18n_next::install(black_box(&i18n).clone()))
    });
    c.bench_function("builder/install/already_installed", |b| {
        b.iter(|| I18n::builder().install())
    });
    c.bench_function("global/global/get", |b| {
        b.iter(|| black_box(i18n_next::global()))
    });
    c.bench_function("global/t/to_string", |b| {
        b.iter(|| i18n_next::t(black_box("plain")).to_string())
    });
    c.bench_function("global/t/macro_to_string", |b| {
        b.iter(|| t!(black_box("plain")).to_string())
    });
    c.bench_function("global/locale/active", |b| {
        b.iter(|| black_box(i18n_next::locale()))
    });
    c.bench_function("global/page/view", |b| {
        b.iter(|| i18n_next::page(black_box("cart")))
    });
    c.bench_function("global/set_locale/same", |b| {
        b.iter(|| i18n_next::set_locale(black_box("en")))
    });
    let available = ["en", "de", "de-at", "fr", "pt-br", "ru", "uk", "ja"];
    c.bench_function("global/negotiate/header_3", |b| {
        b.iter(|| i18n_next::negotiate(black_box("de-CH,de;q=0.9,en;q=0.8"), black_box(available)))
    });
    c.bench_function("global/negotiate/no_match", |b| {
        b.iter(|| i18n_next::negotiate(black_box("zh-Hant-TW, ko;q=0.5"), black_box(available)))
    });
}

#[cfg(not(feature = "global"))]
fn global(_: &mut Criterion) {}

criterion_group!(benches, text, macros, scopes, global);
criterion_main!(benches);
