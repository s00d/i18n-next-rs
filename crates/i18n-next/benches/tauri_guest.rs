//! criterion benches of `tauri_guest` (DESIGN 6.2, 7.7; ids per `tests/support/inventory.rs`).
//! The module is wasm-only and criterion is native-only, so each function is measured by the
//! Rust work it does in the WebView, through the same public calls and on the same bytes:
//!
//! - the guest reads `meta` (`Meta::read`; `payload::builder` is that plus a copy of the
//!   text, the closest public call) and compares its tags with the ones it holds;
//! - the chunks it does not hold come over the protocol as `payload::chunk` bodies and go in
//!   with one `payload::apply` of the guest's own wrapper (`wrap` below, as in
//!   `src/tauri_guest.rs`) of only those chunks;
//! - the instance a step replaces is dropped outside the timing (`iter_batched` /
//!   `iter_with_large_drop`), as the WebView drops it after the call returns.
//!
//! `load` and `preload` do in Rust what `set_page` does without routing: the same
//! `visit`, so their cases are the first visit and the cached one. `disconnect` keeps the
//! instance and its tags for the next `connect`; its unlisten is JS (the tauri-app
//! inspector measures the transport, M10).

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use i18n_next::{I18n, payload};
use std::collections::HashMap;
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

/// The plugin's catalog: `de` → `en` with 1k root keys each and a 50-key `settings` page.
fn server() -> I18n {
    I18n::builder()
        .locale("en")
        .fallback("en")
        .messages("de", None, &doc(1_000, "de"))
        .messages("en", None, &doc(1_000, "en"))
        .messages("de", Some("settings"), &doc(50, "Einstellung"))
        .messages("en", Some("settings"), &doc(50, "Setting"))
        .config_json(r#"{"locales": [{"code": "de", "iso": "de-DE"}, {"code": "en"}]}"#)
        .build()
        .unwrap()
}

/// The guest's payload of protocol bodies (`payload::chunk`) by `"<locale>/<page>"`,
/// switching to `locale`: `wrap` of `src/tauri_guest.rs`.
fn wrap(server: &I18n, locale: &str, keys: &[&str]) -> Vec<u8> {
    let mut out = format!("{{\"locale\":\"{locale}\",\"chunks\":[");
    for (i, key) in keys.iter().enumerate() {
        let (l, p) = key.split_once('/').unwrap();
        let (_, body) = payload::chunk(server, l, Some(p));
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"locale\":\"{l}\",\"page\":\"{p}\","));
        out.push_str(&body[1..]);
    }
    out.push_str("]}");
    out.into_bytes()
}

/// The tags of a meta answer, as the guest holds them after showing it.
fn tags(meta: &[u8]) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_slice(meta).unwrap();
    v["tags"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, t)| (k.clone(), t.as_str().unwrap().to_owned()))
        .collect()
}

/// The Rust work of a step that finds nothing to load: read the meta, compare its tags.
fn unchanged(meta: &[u8], held: &HashMap<String, String>, shown: &[(String, String)]) -> usize {
    let read = payload::builder(meta).unwrap();
    black_box(read);
    shown.iter().filter(|(k, t)| held.get(k) != Some(t)).count()
}

fn benches(c: &mut Criterion) {
    let server = server();
    let meta = payload::meta(&server, None, None);
    let meta_page = payload::meta(&server, None, Some("settings"));
    let shown = tags(&meta_page);
    let held: HashMap<String, String> = shown.iter().cloned().collect();
    let root = wrap(&server, "en", &["en/index"]);
    let page = wrap(&server, "en", &["en/settings"]);
    // `settings` shown in `en`: the first switch reads `de`'s two chunks only
    let de = wrap(&server, "de", &["de/index", "de/settings"]);
    let back = wrap(&server, "en", &[]);
    let connected = || {
        let g = payload::builder(&meta).unwrap().build().unwrap();
        payload::apply(&g, &root).unwrap();
        g
    };
    let on_page = || {
        let g = connected();
        payload::apply(&g, &page).unwrap();
        g
    };

    c.bench_function("tauri_guest/connect/meta_and_root_1000", |b| {
        b.iter_with_large_drop(|| {
            let g = payload::builder(black_box(&meta)).unwrap().build().unwrap();
            payload::apply(&g, black_box(&root)).unwrap();
            g
        })
    });

    // a page's first visit: its chunk in; the instance before is dropped after the timing
    let first_visit = |b: &mut criterion::Bencher| {
        b.iter_batched(
            connected,
            |g| {
                payload::apply(&g, black_box(&page)).unwrap();
                g
            },
            BatchSize::LargeInput,
        )
    };
    // a page checked at an older version whose chunks did not change: meta read, no load
    let cached = |b: &mut criterion::Bencher| {
        b.iter(|| unchanged(black_box(&meta_page), &held, &shown))
    };
    c.bench_function("tauri_guest/set_page/first_visit_page_50", first_visit);
    c.bench_function("tauri_guest/set_page/unchanged_meta", cached);
    c.bench_function("tauri_guest/load/first_visit_page_50", first_visit);
    c.bench_function("tauri_guest/load/unchanged_meta", cached);
    c.bench_function("tauri_guest/preload/first_visit_page_50", first_visit);
    c.bench_function("tauri_guest/preload/unchanged_meta", cached);

    c.bench_function("tauri_guest/set_locale/first_switch_1000_page_50", |b| {
        b.iter_batched(
            on_page,
            |g| {
                payload::builder(black_box(&meta_page)).unwrap();
                payload::apply(&g, black_box(&de)).unwrap();
                g
            },
            BatchSize::LargeInput,
        )
    });
    // back to a locale whose chunks it holds: meta read, a locale switch, nothing read
    c.bench_function("tauri_guest/set_locale/cached_chunks", |b| {
        b.iter_batched(
            || {
                let g = on_page();
                payload::apply(&g, &de).unwrap();
                g
            },
            |g| {
                unchanged(black_box(&meta_page), &held, &shown);
                payload::apply(&g, black_box(&back)).unwrap();
                g
            },
            BatchSize::LargeInput,
        )
    });
    c.bench_function("tauri_guest/refresh/unchanged_meta", |b| {
        b.iter(|| unchanged(black_box(&meta_page), &held, &shown))
    });
    // the instance and its tags kept for the next connect
    c.bench_function("tauri_guest/disconnect/keep_for_reconnect", |b| {
        let g = on_page();
        b.iter_batched(
            || held.clone(),
            |mut tags| (g.clone(), std::mem::take(&mut tags)),
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(tauri_guest, benches);
criterion_main!(tauri_guest);
