//! criterion benches of the M3 sources and lifecycle (ids per `tests/support/inventory.rs`):
//! construction of `lazy_dir` / `lazy_files` / `remote` / `extend`, the first access of a
//! lazy page, `ensure_async` with an in-memory fetch on a manual poll, `add_file`,
//! `missing_files`, `reload` of 1k / 10k keys, `subscribe` and its dispatch, and the page
//! hit with eviction on and off (DESIGN 8b.16: the difference is the LRU stamp), plus a
//! cycle through 1000 pages kept at 50.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use i18n_next::I18n;
use std::future::Future;
use std::hint::black_box;
use std::task::{Context, Poll, Waker};

/// A locale file with `keys` leaves.
fn doc(keys: usize, word: &str) -> String {
    let mut s = String::from("{");
    for k in 0..keys {
        if k > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "\"s{}\":{{\"k{}\":\"{word} {k}\"}}",
            k / 10,
            k % 10
        ));
    }
    s.push('}');
    s
}

/// `pages` page files of `en` plus a root, as `(path, body)`.
fn pages(pages: usize) -> Vec<(String, String)> {
    (0..pages)
        .map(|p| {
            (
                format!("pages/p{p}/en.json"),
                format!(r#"{{"t": "page {p}", "x": "x"}}"#),
            )
        })
        .chain(std::iter::once((
            "en.json".to_owned(),
            r#"{"root": "R"}"#.to_owned(),
        )))
        .collect()
}

/// Polls `f` once with a no-op waker: an in-memory fetch is ready at once.
fn poll_once<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("an in-memory fetch is ready at once"),
    }
}

fn construction(c: &mut Criterion) {
    let tmp = std::env::temp_dir().join(format!("i18n-next-bench-src-{}", std::process::id()));
    let files = pages(1000);
    for (p, body) in &files {
        let full = tmp.join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    c.bench_function("builder/lazy_dir/pages_1000", |b| {
        b.iter(|| I18n::builder().lazy_dir(&tmp).build().unwrap())
    });
    c.bench_function("builder/lazy_files/pages_1000", |b| {
        b.iter(|| {
            I18n::builder()
                .lazy_files(files.iter().map(|(p, b)| (p, b)))
                .build()
                .unwrap()
        })
    });
    let manifest: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    c.bench_function("builder/remote/manifest_1000", |b| {
        b.iter(|| I18n::builder().remote(&manifest).build().unwrap())
    });
    let lib = doc(1000, "lib");
    let app = doc(100, "app");
    c.bench_function("builder/extend/lib_1000_under_app_100", |b| {
        b.iter(|| {
            I18n::builder()
                .extend([("en.json", lib.as_bytes())])
                .files([("en.json", app.as_bytes())])
                .build()
                .unwrap()
        })
    });
    fn set(b: &mut criterion::Bencher<'_>, f: fn(i18n_next::Builder) -> i18n_next::Builder) {
        b.iter_batched(
            I18n::builder,
            |bld| black_box(f(bld)),
            BatchSize::SmallInput,
        );
    }
    c.bench_function("builder/evict_pages/set", |b| set(b, |b| b.evict_pages(50)));
    c.bench_function("builder/pin_page/set", |b| set(b, |b| b.pin_page("tray")));
    let _ = std::fs::remove_dir_all(&tmp);
}

fn lazy(c: &mut Criterion) {
    let files = pages(16);
    let lazy = || {
        I18n::builder()
            .lazy_files(files.iter().map(|(p, b)| (p, b)))
            .build()
            .unwrap()
    };
    // The first access parses the page file; the build is outside the timing.
    c.bench_function("view/t/lazy_first_access", |b| {
        b.iter_batched(
            lazy,
            |i18n| black_box(i18n.page("p3").t("t")),
            BatchSize::SmallInput,
        )
    });
    // Hot path: a loaded lazy page with eviction off and on (one LRU stamp more).
    let off = lazy();
    let on = I18n::builder()
        .lazy_files(files.iter().map(|(p, b)| (p, b)))
        .evict_pages(50)
        .build()
        .unwrap();
    let (p_off, p_on) = (off.page("p3"), on.page("p3"));
    let _ = (p_off.t("t"), p_on.t("t"));
    let mut g = c.benchmark_group("view/get");
    g.bench_function("lazy_page_hit_evict_off", |b| {
        b.iter(|| p_off.get(black_box("t")))
    });
    g.bench_function("lazy_page_hit_evict_on", |b| {
        b.iter(|| p_on.get(black_box("t")))
    });
    g.finish();
    // 1000 pages kept at 50: every access loads one page and evicts one.
    let many = pages(1000);
    let cycle = I18n::builder()
        .lazy_files(many.iter().map(|(p, b)| (p, b)))
        .evict_pages(50)
        .build()
        .unwrap();
    let names: Vec<String> = (0..1000).map(|p| format!("p{p}")).collect();
    let mut at = 0usize;
    c.bench_function("view/t/evict_cycle_1000_pages_keep_50", |b| {
        b.iter(|| {
            at = (at + 1) % names.len();
            black_box(cycle.page(&names[at]).t("t"))
        })
    });
}

fn remote(c: &mut Criterion) {
    let manifest = [
        "en.json",
        "de.json",
        "pages/cart/de.json",
        "pages/cart/en.json",
    ];
    let remote = || {
        I18n::builder()
            .locale("de")
            .fallback("en")
            .remote(manifest)
            .build()
            .unwrap()
    };
    let body = doc(100, "x");
    c.bench_function("i18n/add_file/root_100_keys", |b| {
        b.iter_batched(
            remote,
            |i18n| i18n.add_file("de.json", body.as_bytes()).unwrap(),
            BatchSize::SmallInput,
        )
    });
    let fetch = |_: String| {
        let bytes = body.clone().into_bytes();
        async move { Ok::<_, &str>(bytes) }
    };
    c.bench_function("i18n/ensure_async/four_files_in_memory", |b| {
        b.iter_batched(
            remote,
            |i18n| poll_once(i18n.ensure_async("de", Some("cart"), fetch)).unwrap(),
            BatchSize::SmallInput,
        )
    });
    let fresh = remote();
    c.bench_function("i18n/missing_files/chain_2_page", |b| {
        b.iter(|| fresh.missing_files(black_box("de"), Some("cart")))
    });
}

fn lifecycle(c: &mut Criterion) {
    let tmp = std::env::temp_dir().join(format!("i18n-next-bench-reload-{}", std::process::id()));
    let mut g = c.benchmark_group("i18n/reload");
    for keys in [1_000usize, 10_000] {
        let dir = tmp.join(keys.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        for l in ["en", "de", "ru"] {
            std::fs::write(dir.join(format!("{l}.json")), doc(keys, l)).unwrap();
        }
        let i18n = I18n::builder().dir(&dir).build().unwrap();
        g.bench_function(format!("dir_{keys}x3"), |b| {
            b.iter(|| i18n.reload().unwrap())
        });
    }
    g.finish();
    let _ = std::fs::remove_dir_all(&tmp);

    let i18n = I18n::builder()
        .messages("en", None, "{}")
        .messages("de", None, "{}")
        .build()
        .unwrap();
    c.bench_function("i18n/subscribe/and_drop", |b| {
        b.iter(|| drop(black_box(i18n.subscribe(|_| {}))))
    });
    let subs: Vec<_> = (0..4)
        .map(|_| {
            i18n.subscribe(|c| {
                black_box(c.version);
            })
        })
        .collect();
    let mut flip = false;
    c.bench_function("i18n/subscribe/dispatch_4_on_set_locale", |b| {
        b.iter(|| {
            flip = !flip;
            i18n.set_locale(if flip { "de" } else { "en" }).unwrap()
        })
    });
    drop(subs);
}

criterion_group!(benches, construction, lazy, remote, lifecycle);
criterion_main!(benches);
