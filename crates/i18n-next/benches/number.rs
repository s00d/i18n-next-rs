//! criterion benches of M5 (DESIGN 5.5, 7.7): `tn` per style (decimal, integer, percent,
//! currency by symbol and code, compact), a locale from the snapshot's table (warm) against
//! one resolved on every call (cold), `Text` with `{number}` written into a buffer, the
//! builder's format parsing and `language_name`. Ids follow `tests/support/inventory.rs`.

use criterion::{Criterion, criterion_group, criterion_main};
use i18n_next::{I18n, scope, tn, tn_as};
use std::fmt::Write as _;
use std::hint::black_box;

const FORMATS: &str = r#"{"en": {"pct": {"style": "percent", "maximumFractionDigits": 1},
    "usd": {"style": "currency", "currency": "USD"},
    "code": {"style": "currency", "currency": "EUR", "currencyDisplay": "code"},
    "short": {"notation": "compact"}},
  "ar": {"egp": {"style": "currency", "currency": "EGP"}}}"#;

fn catalog() -> I18n {
    I18n::builder()
        .locale("en")
        .iso("ar", "ar-EG")
        .fallback("ar")
        .messages("en", None, r#"{"total": "Total: {number}"}"#)
        .formats(FORMATS)
        .build()
        .unwrap()
}

fn numbers(c: &mut Criterion) {
    let i18n = catalog();
    c.bench_function("i18n/tn/decimal", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(1234567.891)))
    });
    c.bench_function("i18n/tn/integer", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(1234567)))
    });
    c.bench_function("i18n/tn/long_fraction", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(0.1 + 0.2)))
    });
    c.bench_function("i18n/tn_as/percent", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(0.1234), black_box("pct")))
    });
    c.bench_function("i18n/tn_as/currency", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(1234.5), black_box("usd")))
    });
    c.bench_function("i18n/tn_as/currency_code", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(-1234.5), black_box("code")))
    });
    c.bench_function("i18n/tn_as/compact", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(1234567.0), black_box("short")))
    });
    // warm: a known locale (data looked up once per snapshot); cold: a code nobody
    // declared, its tag resolved on every call
    let ar = i18n.with_locale("ar");
    c.bench_function("view/tn/warm_arabic_digits", |b| {
        b.iter(|| black_box(&ar).tn(black_box(1234567.891)))
    });
    let cold = i18n.with_locale("zh-Hant-TW");
    c.bench_function("view/tn/cold_resolve", |b| {
        b.iter(|| black_box(&cold).tn(black_box(1234567.891)))
    });
    c.bench_function("view/tn_as/currency", |b| {
        b.iter(|| black_box(&ar).tn_as(black_box(1234.5), black_box("egp")))
    });
    let total = i18n.text("total").number(1234.5);
    let priced = i18n.text("total").number_as(1234.5, "usd");
    let mut buf = String::with_capacity(64);
    c.bench_function("text/number/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&total)).unwrap();
        })
    });
    c.bench_function("text/number/to_string", |b| {
        b.iter(|| black_box(&total).to_string())
    });
    c.bench_function("text/number_as/display", |b| {
        b.iter(|| {
            buf.clear();
            black_box(&priced).write_to(&mut buf).unwrap();
        })
    });
    c.bench_function("i18n/language_name/known", |b| {
        b.iter(|| black_box(&i18n).language_name(black_box("ar")))
    });
    c.bench_function("builder/formats/two_locales", |b| {
        b.iter(|| I18n::builder().formats(black_box(FORMATS)).build().unwrap())
    });
    scope().instance(i18n.clone()).locale("ar").run(|| {
        c.bench_function("global/tn/scope", |b| b.iter(|| tn(black_box(1234.5))));
        c.bench_function("global/tn_as/scope", |b| {
            b.iter(|| tn_as(black_box(1234.5), black_box("egp")))
        });
    });
}

criterion_group!(benches, numbers);
criterion_main!(benches);
