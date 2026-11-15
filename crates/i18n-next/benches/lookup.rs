//! criterion benches of the M1 API. Every public function has a bench named
//! `group/function/case` (`tests/bench_inventory.rs` checks it). Cases follow DESIGN 7.7:
//! lookup root hit, page hit, page miss → root, a miss down a 3-locale chain, a 6-deep
//! key; `t_with` with 0 / 1 / 4 / 8 parameters; build from 1k / 10k keys (files and dir);
//! readers on 1 / 4 / 16 threads while one thread writes.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use i18n_next::{Args, I18n};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A locale file with `keys` leaves in sections of 10 groups of 10.
fn doc(keys: usize, word: &str) -> String {
    let mut s = String::from("{");
    for sec in 0..keys.div_ceil(100) {
        if sec > 0 {
            s.push(',');
        }
        s.push_str(&format!("\"s{sec}\":{{"));
        for g in 0..10 {
            if g > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"g{g}\":{{"));
            for k in 0..10 {
                if k > 0 {
                    s.push(',');
                }
                s.push_str(&format!("\"k{k}\":\"{word} {{name}} {sec}.{g}.{k}\""));
            }
            s.push('}');
        }
        s.push('}');
    }
    s.push('}');
    s
}

fn catalog() -> I18n {
    I18n::builder()
        .config_json(r#"{"locales": [{"code": "en", "iso": "en-US"}, {"code": "de"}, {"code": "de-at"}]}"#)
        .locale("de-at")
        .fallback("en")
        .fallback_for("de-at", "de")
        .messages("en", None, &doc(1000, "en"))
        .messages("en", None, r#"{"a": {"b": {"c": {"d": {"e": {"f": "deep"}}}}}, "only_en": "x"}"#)
        .messages("de", None, &doc(1000, "de"))
        .messages("de-at", None, &doc(1000, "at"))
        .messages("de-at", Some("settings"), r#"{"title": "Einstellungen"}"#)
        .messages(
            "de-at",
            None,
            r#"{"g0": "{a}", "g1": "Hi {a}!", "g4": "{a} {b} {c} {d}", "g8": "{a} {b} {c} {d} {e} {f} {g} {h}"}"#,
        )
        .build()
        .unwrap()
}

fn lookup(c: &mut Criterion) {
    let i18n = catalog();
    let settings = i18n.page("settings");
    c.bench_function("lookup/t/root_hit", |b| {
        b.iter(|| i18n.t(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/t/page_hit", |b| {
        b.iter(|| settings.t(black_box("title")))
    });
    c.bench_function("lookup/t/page_miss_root", |b| {
        b.iter(|| settings.t(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/t/chain_3_to_en", |b| {
        b.iter(|| i18n.t(black_box("only_en")))
    });
    c.bench_function("lookup/t/chain_3_miss", |b| {
        b.iter(|| i18n.t(black_box("s3.g4.nope")))
    });
    c.bench_function("lookup/t/deep_6", |b| {
        b.iter(|| i18n.t(black_box("a.b.c.d.e.f")))
    });
    c.bench_function("lookup/try_t/root_hit", |b| {
        b.iter(|| i18n.try_t(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/get/root_hit", |b| {
        b.iter(|| i18n.get(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/get/page_hit", |b| {
        b.iter(|| settings.get(black_box("title")))
    });
    c.bench_function("lookup/has/root_hit", |b| {
        b.iter(|| i18n.has(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/has/miss", |b| {
        b.iter(|| i18n.has(black_box("only_en")))
    });

    let a1 = Args::new().str("a", "Ann");
    let a4 = Args::new()
        .str("a", "A")
        .str("b", "B")
        .int("c", 3)
        .bool("d", true);
    let a8 = Args::new()
        .str("a", "A")
        .str("b", "B")
        .int("c", 3)
        .bool("d", true)
        .str("e", "E")
        .float("f", 1.5)
        .null("g")
        .str("h", "H");
    c.bench_function("lookup/t_with/args_0", |b| {
        b.iter(|| i18n.t_with(black_box("g0"), &Args::new()))
    });
    c.bench_function("lookup/t_with/args_1", |b| {
        b.iter(|| i18n.t_with(black_box("g1"), &a1))
    });
    c.bench_function("lookup/t_with/args_4", |b| {
        b.iter(|| i18n.t_with(black_box("g4"), &a4))
    });
    c.bench_function("lookup/t_with/args_8", |b| {
        b.iter(|| i18n.t_with(black_box("g8"), &a8))
    });
    c.bench_function("lookup/t_with/miss_default", |b| {
        b.iter(|| i18n.t_with(black_box("nope"), &Args::new().int("n", 2).or("{n} items")))
    });

    c.bench_function("lookup/page/view", |b| {
        b.iter(|| i18n.page(black_box("settings")))
    });
    c.bench_function("lookup/with_locale/view", |b| {
        b.iter(|| i18n.with_locale(black_box("en")))
    });
    c.bench_function("lookup/with_locale/t_hit", |b| {
        let en = i18n.with_locale("en");
        b.iter(|| en.t(black_box("s3.g4.k5")))
    });
    c.bench_function("lookup/version/static", |b| {
        b.iter(|| black_box(i18n_next::version()))
    });
    c.bench_function("lookup/version/instance", |b| {
        b.iter(|| black_box(&i18n).version())
    });
    c.bench_function("lookup/locale/active", |b| {
        b.iter(|| black_box(&i18n).locale())
    });
    c.bench_function("lookup/locales/three", |b| {
        b.iter(|| black_box(&i18n).locales())
    });
    c.bench_function("lookup/pages/one", |b| b.iter(|| black_box(&i18n).pages()));
}

fn args(c: &mut Criterion) {
    c.bench_function("args/new/empty", |b| b.iter(|| black_box(Args::new())));
    c.bench_function("args/str/one", |b| {
        b.iter(|| black_box(Args::new().str("a", black_box("A"))))
    });
    c.bench_function("args/int/one", |b| {
        b.iter(|| black_box(Args::new().int("a", black_box(3))))
    });
    c.bench_function("args/float/one", |b| {
        b.iter(|| black_box(Args::new().float("a", black_box(1.5))))
    });
    c.bench_function("args/bool/one", |b| {
        b.iter(|| black_box(Args::new().bool("a", black_box(true))))
    });
    c.bench_function("args/null/one", |b| {
        b.iter(|| black_box(Args::new().null(black_box("a"))))
    });
    c.bench_function("args/arg/one", |b| {
        b.iter(|| black_box(Args::new().arg("a", black_box(7))))
    });
    c.bench_function("args/or/default", |b| {
        b.iter(|| black_box(Args::new().or(black_box("x"))))
    });
    c.bench_function("args/list/two", |b| {
        b.iter(|| black_box(Args::list(black_box(["Ann", "Bob"]))))
    });
    c.bench_function("args/str/five_overflow", |b| {
        b.iter(|| {
            black_box(
                Args::new()
                    .str("a", "1")
                    .str("b", "2")
                    .str("c", "3")
                    .str("d", "4")
                    .str("e", "5"),
            )
        })
    });
}

fn write(c: &mut Criterion) {
    let i18n = catalog();
    let mut flip = false;
    c.bench_function("write/set_locale/switch", |b| {
        b.iter(|| {
            flip = !flip;
            i18n.set_locale(if flip { "de" } else { "de-at" }).unwrap()
        })
    });
    c.bench_function("write/add_messages/one_key_into_1k", |b| {
        b.iter(|| {
            i18n.add_messages(
                "en",
                None,
                black_box(r#"{"s1": {"g1": {"k1": "patched"}}}"#),
            )
            .unwrap()
        })
    });
}

fn build(c: &mut Criterion) {
    let tmp = std::env::temp_dir().join(format!("i18n-next-bench-{}", std::process::id()));
    for keys in [1_000usize, 10_000] {
        let files: Vec<(String, Vec<u8>)> = ["en", "de", "ru"]
            .iter()
            .map(|l| (format!("{l}.json"), doc(keys, l).into_bytes()))
            .chain(std::iter::once((
                "pages/settings/en.json".to_owned(),
                br#"{"title":"Settings"}"#.to_vec(),
            )))
            .collect();
        let dir = tmp.join(keys.to_string());
        for (p, body) in &files {
            let full = dir.join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        let case = format!("keys_{keys}x3");
        let mut g = c.benchmark_group("build/files");
        g.bench_function(&case, |b| {
            b.iter(|| {
                I18n::builder()
                    .files(files.iter().map(|(p, b)| (p, b)))
                    .build()
                    .unwrap()
            })
        });
        g.finish();
        let mut g = c.benchmark_group("build/dir");
        g.bench_function(&case, |b| {
            b.iter(|| I18n::builder().dir(&dir).build().unwrap())
        });
        g.finish();
        let text = doc(keys, "en");
        let mut g = c.benchmark_group("build/messages");
        g.bench_function(&case, |b| {
            b.iter(|| I18n::builder().messages("en", None, &text).build().unwrap())
        });
        g.finish();
    }
    let config = r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [{"code": "en", "iso": "en-US"}, {"code": "de-at", "fallbackLocale": "de"}, {"code": "es", "disabled": true}]}"#;
    let config_path = tmp.join("i18n.json");
    std::fs::write(&config_path, config).unwrap();
    c.bench_function("build/config_json/three_locales", |b| {
        b.iter(|| {
            I18n::builder()
                .config_json(black_box(config))
                .build()
                .unwrap()
        })
    });
    c.bench_function("build/config/three_locales", |b| {
        b.iter(|| I18n::builder().config(&config_path).build().unwrap())
    });
    c.bench_function("build/build/empty", |b| {
        b.iter(|| I18n::builder().build().unwrap())
    });
    c.bench_function("build/builder/new", |b| {
        b.iter(|| black_box(I18n::builder()))
    });
    // The setters alone: each consumes and returns the builder.
    fn set(b: &mut criterion::Bencher<'_>, f: fn(i18n_next::Builder) -> i18n_next::Builder) {
        b.iter_batched(
            I18n::builder,
            |bld| black_box(f(bld)),
            BatchSize::SmallInput,
        );
    }
    c.bench_function("build/locale/set", |b| set(b, |b| b.locale("de")));
    c.bench_function("build/fallback/set", |b| set(b, |b| b.fallback("en")));
    c.bench_function("build/fallback_for/set", |b| {
        set(b, |b| b.fallback_for("de-at", "de"))
    });
    c.bench_function("build/iso/set", |b| set(b, |b| b.iso("ua", "uk-UA")));
    c.bench_function("build/pages/set", |b| set(b, |b| b.pages(false)));
    c.bench_function("build/page_alias/set", |b| {
        set(b, |b| b.page_alias("about-us", "about"))
    });
    c.bench_function("build/missing/set", |b| set(b, |b| b.missing(|_, _, _| {})));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `threads` readers each do their share of `iters` root hits while one thread keeps
/// writing; the time is the readers' wall time.
fn readers(i18n: &I18n, threads: usize, iters: u64) -> Duration {
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (i18n, stop) = (i18n.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                i18n.add_messages("en", None, r#"{"w": "x"}"#).unwrap();
            }
        })
    };
    let per = iters.div_ceil(threads as u64);
    let start = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                for _ in 0..per {
                    black_box(i18n.get(black_box("s3.g4.k5")));
                }
            });
        }
    });
    let took = start.elapsed();
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    took
}

fn concurrency(c: &mut Criterion) {
    let i18n = I18n::builder()
        .locale("en")
        .messages("en", None, &doc(1000, "en"))
        .build()
        .unwrap();
    let mut g = c.benchmark_group("concurrency/get");
    for threads in [1, 4, 16] {
        g.bench_function(format!("readers_{threads}_one_writer"), |b| {
            b.iter_custom(|iters| readers(&i18n, threads, iters))
        });
    }
    g.finish();
}

criterion_group!(benches, lookup, args, write, build, concurrency);
criterion_main!(benches);
