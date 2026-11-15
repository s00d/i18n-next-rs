//! criterion benches of linked messages and `check()` (DESIGN 7.7, M3). Ids are
//! `group/function/case`:
//!
//! - `builder/linked_messages/<n>_links_x3`: `build` of three locales and a page whose root
//!   holds `n` linked messages (and as many plain ones), against `.../<n>_plain_x3`, the same
//!   catalog with links off: what resolving costs at load;
//! - `i18n/t/linked_hit` vs `i18n/t/plain_hit`, `view/t/linked_view_hit` vs
//!   `view/t/plain_view_hit`: a resolved message, one resolved for another locale's view, and
//!   plain ones of the same text in the same layer: the same lookup (zero cost);
//! - `i18n/check/<n>_keys_x3`, `check_report/is_clean/<case>`.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use i18n_next::I18n;
use std::hint::black_box;

/// `n` plain keys `k<i>`, `n` linked ones `l<i>` (`"see @:k<i> and @.upper:{'k<i>'}"`) and
/// `n` plain ones `s<i>` with the text `l<i>` resolves to.
fn root(n: usize, word: &str) -> String {
    let mut s = String::from("{");
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#""k{i}": "{word} {i}", "l{i}": "see @:k{i} and @.upper:{{'k{i}'}}", "s{i}": "see {word} {i} and {} {i}""#,
            word.to_uppercase()
        ));
    }
    s.push('}');
    s
}

fn builder(n: usize, linked: bool) -> i18n_next::Builder {
    let en = root(n, "word");
    let de = root(n / 2, "Wort");
    I18n::builder()
        .locale("en")
        .fallback("en")
        .linked_messages(linked)
        .files([
            ("en.json", en.into_bytes()),
            ("de.json", de.into_bytes()),
            (
                "fr.json",
                br#"{"k0": "mot 0", "same": "see mot 0 and MOT 0"}"#.to_vec(),
            ),
            (
                "pages/about/en.json",
                br#"{"k1": "about", "p": "on @:k1"}"#.to_vec(),
            ),
        ])
}

fn build(c: &mut Criterion) {
    let mut g = c.benchmark_group("builder/linked_messages");
    for n in [10, 1000] {
        g.bench_function(format!("{n}_links_x3"), |b| {
            b.iter_batched(
                || builder(n, true),
                |b| b.build().unwrap(),
                BatchSize::SmallInput,
            )
        });
        g.bench_function(format!("{n}_plain_x3"), |b| {
            b.iter_batched(
                || builder(n, false),
                |b| b.build().unwrap(),
                BatchSize::SmallInput,
            )
        });
    }
    g.finish();
}

fn lookup(c: &mut Criterion) {
    let i18n = builder(100, true).build().unwrap();
    let fr = i18n.with_locale("fr");
    assert_eq!(i18n.t("l7"), "see word 7 and WORD 7");
    assert_eq!(fr.t("l0"), "see mot 0 and MOT 0"); // stored for the French view
    assert_eq!(i18n.t("s7"), i18n.t("l7"));
    c.bench_function("i18n/t/plain_hit", |b| {
        b.iter(|| black_box(&i18n).t(black_box("s7")))
    });
    c.bench_function("i18n/t/linked_hit", |b| {
        b.iter(|| black_box(&i18n).t(black_box("l7")))
    });
    assert_eq!(fr.t("same"), fr.t("l0"));
    c.bench_function("view/t/plain_view_hit", |b| {
        b.iter(|| black_box(&fr).t(black_box("same")))
    });
    c.bench_function("view/t/linked_view_hit", |b| {
        b.iter(|| black_box(&fr).t(black_box("l0")))
    });
}

fn check(c: &mut Criterion) {
    let mut g = c.benchmark_group("i18n/check");
    for n in [10, 1000] {
        let i18n = builder(n, true).build().unwrap();
        g.bench_function(format!("{n}_keys_x3"), |b| {
            b.iter(|| black_box(&i18n).check())
        });
    }
    g.finish();
    let i18n = builder(10, true).build().unwrap();
    let dirty = i18n.check();
    let clean = I18n::builder().build().unwrap().check();
    c.bench_function("check_report/is_clean/clean", |b| {
        b.iter(|| black_box(&clean).is_clean())
    });
    c.bench_function("check_report/is_clean/dirty", |b| {
        b.iter(|| black_box(&dirty).is_clean())
    });
}

criterion_group!(benches, build, lookup, check);
criterion_main!(benches);
