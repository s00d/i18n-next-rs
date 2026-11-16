//! criterion benches of `tauri_guest` (DESIGN 6.2, 7.7; ids per `tests/support/inventory.rs`).
//! The module is wasm-only and criterion is native-only, so each function is measured by the
//! Rust work it does in the WebView, through the same public calls: `connect` builds the
//! instance from the meta (`payload::builder`) and applies the chain's chunks, `set_page` /
//! `load` / `preload` apply a page chunk, `set_locale` applies another locale's chain,
//! `refresh` reads an unchanged meta, `disconnect` drops the guest. The transport (IPC and
//! the protocol's `fetch`) is the WebView's: the tauri-app inspector measures it (M10).

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
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

fn guest(meta: &[u8]) -> I18n {
    payload::builder(meta).unwrap().build().unwrap()
}

fn benches(c: &mut Criterion) {
    let server = server();
    let meta = payload::meta(&server, None, None);
    let (_, root) = payload::encode(&server, None, None, false);
    let (_, page) = payload::encode_page(&server, "settings");
    let (_, de) = payload::encode(&server, Some("de"), Some("settings"), false);
    c.bench_function("tauri_guest/connect/meta_and_chain_1000", |b| {
        b.iter(|| {
            let g = guest(black_box(&meta));
            payload::apply(&g, black_box(&root)).unwrap();
            g
        })
    });
    let g = guest(&meta);
    payload::apply(&g, &root).unwrap();
    // the three apply one page chunk; the ids are literal for the inventory
    let apply_page = |b: &mut criterion::Bencher| {
        b.iter(|| black_box(payload::apply(&g, black_box(&page)).unwrap()))
    };
    c.bench_function("tauri_guest/set_page/page_50", apply_page);
    c.bench_function("tauri_guest/load/page_50", apply_page);
    c.bench_function("tauri_guest/preload/page_50", apply_page);
    c.bench_function("tauri_guest/set_locale/chain_1000x2_page_50", |b| {
        b.iter(|| black_box(payload::apply(&g, black_box(&de)).unwrap()))
    });
    c.bench_function("tauri_guest/refresh/meta_unchanged", |b| {
        b.iter(|| black_box(payload::builder(black_box(&meta)).unwrap()))
    });
    c.bench_function("tauri_guest/disconnect/drop_guest", |b| {
        b.iter_batched(
            || {
                let g = guest(&meta);
                payload::apply(&g, &root).unwrap();
                g
            },
            drop,
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(tauri_guest, benches);
criterion_main!(tauri_guest);
