//! criterion benches of the front-end payload (feature `payload`, DESIGN 6.1, 7.7; ids per
//! `tests/support/inventory.rs`): `encode` of the root chain of 1k / 10k keys with and
//! without meta, `encode_page` of a typical page, one `chunk` (a cached `Arc`), the
//! protocol's `bundle` (served and with a stale tag, which builds no body), `meta`, and
//! `apply` of the 1k payload (one pass), and a guest's `builder` from the meta (alone and
//! built).

use criterion::{Criterion, criterion_group, criterion_main};
use i18n_next::{I18n, payload};
use std::hint::black_box;

/// A locale file with `keys` leaves, ten per section.
fn doc(keys: usize, word: &str) -> String {
    let mut s = String::from("{");
    for k in 0..keys {
        if k % 10 == 0 {
            if k > 0 {
                s.push_str("},");
            }
            s.push_str(&format!("\"s{}\":{{", k / 10));
        } else {
            s.push(',');
        }
        s.push_str(&format!("\"k{}\":\"{word} {k}\"", k % 10));
    }
    if keys > 0 {
        s.push('}');
    }
    s + "}"
}

/// `de` → `en` with `keys` root keys each, and a 50-key `settings` page.
fn catalog(keys: usize) -> I18n {
    I18n::builder()
        .locale("de")
        .fallback("en")
        .messages("de", None, &doc(keys, "de"))
        .messages("en", None, &doc(keys, "en"))
        .messages("de", Some("settings"), &doc(50, "Einstellung"))
        .config_json(r#"{"locales": [{"code": "de", "iso": "de-DE"}, {"code": "en"}]}"#)
        .build()
        .unwrap()
}

fn encode(c: &mut Criterion) {
    for keys in [1_000, 10_000] {
        let i18n = catalog(keys);
        c.bench_function(&format!("payload/encode/chain_{keys}x2"), |b| {
            b.iter(|| black_box(payload::encode(&i18n, None, None, false)))
        });
    }
    let i18n = catalog(1_000);
    c.bench_function("payload/encode/meta_page_1000x2", |b| {
        b.iter(|| black_box(payload::encode(&i18n, None, Some("settings"), true)))
    });
    c.bench_function("payload/encode_page/page_50", |b| {
        b.iter(|| black_box(payload::encode_page(&i18n, black_box("settings"))))
    });
    c.bench_function("payload/chunk/root_1000", |b| {
        b.iter(|| black_box(payload::chunk(&i18n, black_box("de"), None)))
    });
    let (tag, _) = payload::bundle(&i18n, "de", Some("settings"), None);
    c.bench_function("payload/bundle/page_1000x2", |b| {
        b.iter(|| black_box(payload::bundle(&i18n, "de", Some("settings"), Some(tag))))
    });
    c.bench_function("payload/bundle/stale_tag", |b| {
        b.iter(|| {
            black_box(payload::bundle(
                &i18n,
                "de",
                Some("settings"),
                Some(tag ^ 1),
            ))
        })
    });
    c.bench_function("payload/meta/chain_page", |b| {
        b.iter(|| black_box(payload::meta(&i18n, None, Some("settings"))))
    });
    let (_, bytes) = payload::encode(&i18n, None, Some("settings"), true);
    let guest = I18n::builder().build().unwrap();
    c.bench_function("payload/apply/meta_page_1000x2", |b| {
        b.iter(|| black_box(payload::apply(&guest, black_box(&bytes)).unwrap()))
    });
    // a guest's start: the builder from the meta, alone and built (config, chains, formats)
    let meta = payload::meta(&i18n, None, Some("settings"));
    let mut g = c.benchmark_group("payload/builder");
    g.bench_function("meta", |b| {
        b.iter(|| black_box(payload::builder(black_box(&meta)).unwrap()))
    });
    g.bench_function("meta_and_build", |b| {
        b.iter(|| black_box(payload::builder(black_box(&meta)).unwrap().build().unwrap()))
    });
    g.finish();
}

criterion_group!(benches, encode);
criterion_main!(benches);
