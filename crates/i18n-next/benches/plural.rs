//! criterion benches of the crate-private plural module (DESIGN 5, 7.7), included with
//! `#[path]` like the `json` bench. Ids are `group/function/case`:
//!
//! - `plural/category/<locale>`: the category of a count through the cached locale data
//!   (a table under 200, the generated rule above);
//! - `plural/rule/<locale>_<n>`: one generated rule call;
//! - `plural/for_tag/<tag>`: resolving a tag (aliases, `BestAvailableLocale`, the cache), as
//!   `tc` on a view in another locale does on every call;
//! - `plural/compute/<locale>`: what the first `tc` of a rule set costs (200 rule calls), the
//!   price of computing the locale data instead of storing ~200 bytes per rule set.

#[path = "../src/plural/mod.rs"]
#[allow(dead_code)]
mod plural;

use criterion::{Criterion, criterion_group, criterion_main};
use plural::{Operands, Whole};
use std::hint::black_box;

fn plurals(c: &mut Criterion) {
    for tag in ["en", "ru", "ar", "fr", "cy"] {
        let data = plural::for_tag(tag).unwrap();
        c.bench_function(&format!("plural/category/{tag}_21"), |b| {
            b.iter(|| data.category(black_box(Whole::Int(21))))
        });
        c.bench_function(&format!("plural/category/{tag}_1021"), |b| {
            b.iter(|| data.category(black_box(Whole::Int(1021))))
        });
        let i = plural::rule_index(tag).unwrap();
        c.bench_function(&format!("plural/compute/{tag}"), |b| {
            b.iter(|| plural::compute(black_box(i)))
        });
    }
    c.bench_function("plural/category/ru_big_f64", |b| {
        let data = plural::for_tag("ru").unwrap();
        b.iter(|| data.category(black_box(Whole::Big(2f64.powi(61)))))
    });
    c.bench_function("plural/rule/ru_operands_int", |b| {
        b.iter(|| Operands::int(black_box(1021)))
    });
    for (case, tag) in [
        ("ru", "ru"),
        ("de_at", "de-AT"),
        ("zh_hant_tw", "zh-Hant-TW"),
        ("pt_pt", "pt-PT"),
        ("alias_iw", "iw"),
        ("unsupported_xx", "xx"),
        ("invalid_en_us", "en_US"),
        ("extension", "en-US-u-nu-latn"),
    ] {
        c.bench_function(&format!("plural/for_tag/{case}"), |b| {
            b.iter(|| plural::for_tag(black_box(tag)).is_some())
        });
    }
}

criterion_group!(benches, plurals);
criterion_main!(benches);
