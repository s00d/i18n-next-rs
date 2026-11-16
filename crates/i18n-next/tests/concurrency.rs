//! Readers and writers at once (DESIGN 4.5, S1): every answer a reader sees belongs to
//! some published snapshot, a reader never goes back to an older one, a write is visible
//! to every thread once it returned, and writers on one instance lose nothing. Readers run
//! on one instance (every read goes through the thread cache) and on six (more than the
//! cache's 4 slots, so slots are evicted and refilled). Threads start together on a
//! `Barrier`; nothing sleeps, readers stop when they see the writers' last values.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::I18n;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

const READERS: usize = 4;

fn build(n: usize) -> I18n {
    I18n::builder()
        .locale("en")
        .messages("en", None, &format!(r#"{{"k": "0", "id": "{n}"}}"#))
        .build()
        .unwrap()
}

/// `instances` catalogs, one writer each publishing `writes` versions (`k` = its own
/// version), four readers reading all catalogs in turn.
fn readers_and_one_writer_each(instances: usize, writes: u64) {
    let catalogs: Vec<I18n> = (0..instances).map(build).collect();
    let start = Arc::new(Barrier::new(READERS + instances));
    let mut handles = Vec::new();

    for i18n in &catalogs {
        let (i18n, start) = (i18n.clone(), start.clone());
        handles.push(thread::spawn(move || {
            start.wait();
            for v in 1..=writes {
                // step: one write = one snapshot whose `k` is its own version
                i18n.add_messages("en", None, &format!(r#"{{"k": "{v}"}}"#))
                    .unwrap();
                assert_eq!(i18n.version(), v);
            }
        }));
    }
    for _ in 0..READERS {
        let (catalogs, start) = (catalogs.clone(), start.clone());
        handles.push(thread::spawn(move || {
            start.wait();
            let mut last = vec![0u64; instances];
            let mut done = 0;
            while done < instances {
                done = 0;
                for (n, i18n) in catalogs.iter().enumerate() {
                    let before = i18n.version();
                    let k: u64 = i18n.get("k").unwrap().parse().unwrap();
                    let after = i18n.version();
                    // step: the value is a published snapshot, between the versions seen
                    // around it: a stale cached snapshot reads below `before`
                    assert!(
                        before <= k && k <= after,
                        "instance {n}: {before} <= {k} <= {after}"
                    );
                    assert!(k >= last[n], "instance {n} went back: {} -> {k}", last[n]);
                    assert_eq!(
                        i18n.t("id"),
                        n.to_string(),
                        "the thread cache mixed instances"
                    );
                    last[n] = k;
                    done += usize::from(k == writes);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

/// Given one catalog and one writer publishing 4000 versions, when four readers read it
/// in a loop, then every read is the snapshot of a version seen around it: the thread
/// cache is hit on every read and refreshed on every version change.
#[test]
fn one_instance_readers_follow_every_version() {
    readers_and_one_writer_each(1, 4000);
}

/// Given six catalogs and one writer per catalog, when four readers read all six in turn
/// while the writers publish 2000 versions each, then every value read is a published one,
/// never older than the last one this reader saw, and the version agrees with it (the
/// 4-slot cache evicts and refills, never mixing instances).
#[test]
fn six_instances_readers_see_whole_snapshots_in_order() {
    readers_and_one_writer_each(6, 2000);
}

const WRITERS: usize = 4;
const PER_WRITER: u64 = 500;

/// Given one catalog, when four writers each add 500 distinct keys at once while four
/// readers check that every key a writer reported done is visible, then no write is lost:
/// the version is exactly 4 × 500, every key is there, each writer saw the version grow
/// after each of its writes, and a reader never misses a key whose write returned.
#[test]
fn writers_on_one_instance_lose_nothing() {
    let i18n = build(0);
    let start = Arc::new(Barrier::new(WRITERS + READERS));
    // progress[w] = how many keys writer `w` has published (its write returned)
    let progress: Arc<Vec<AtomicU64>> = Arc::new((0..WRITERS).map(|_| AtomicU64::new(0)).collect());
    let finished = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for w in 0..WRITERS {
        let (i18n, start, progress, finished) = (
            i18n.clone(),
            start.clone(),
            progress.clone(),
            finished.clone(),
        );
        handles.push(thread::spawn(move || {
            start.wait();
            let mut seen = 0u64;
            for i in 0..PER_WRITER {
                // step: a distinct key per write
                i18n.add_messages("en", None, &format!(r#"{{"w{w}_{i}": "{w}.{i}"}}"#))
                    .unwrap();
                let v = i18n.version();
                assert!(v > seen, "writer {w}: version {v} after {seen}");
                seen = v;
                progress[w].store(i + 1, Ordering::Release);
            }
            finished.fetch_add(1, Ordering::Release);
        }));
    }
    for _ in 0..READERS {
        let (i18n, start, progress, finished) = (
            i18n.clone(),
            start.clone(),
            progress.clone(),
            finished.clone(),
        );
        handles.push(thread::spawn(move || {
            start.wait();
            let mut last_version = 0;
            loop {
                let all_done = finished.load(Ordering::Acquire) == WRITERS;
                for (w, p) in progress.iter().enumerate() {
                    let n = p.load(Ordering::Acquire);
                    if n == 0 {
                        continue;
                    }
                    // step: a write that returned is visible to this thread
                    let key = format!("w{w}_{}", n - 1);
                    assert_eq!(
                        i18n.get(&key).as_deref(),
                        Some(format!("{w}.{}", n - 1).as_str()),
                        "reader lost {key}"
                    );
                }
                let v = i18n.version();
                assert!(
                    v >= last_version,
                    "version went back: {last_version} -> {v}"
                );
                last_version = v;
                if all_done {
                    break;
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // step: nothing lost
    assert_eq!(i18n.version(), WRITERS as u64 * PER_WRITER);
    for w in 0..WRITERS {
        for i in 0..PER_WRITER {
            assert!(i18n.has(&format!("w{w}_{i}")), "w{w}_{i} lost");
        }
    }
}

/// Lazy and remote loads of `i18n` (its `Debug` shows them).
fn loads(i18n: &I18n) -> u64 {
    let d = format!("{i18n:?}");
    let at = d.find("loads: ").unwrap() + 7;
    d[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Given a lazy page nobody has read, when eight threads released by one barrier read it
/// at once, then the file is parsed exactly once (the others wait for that load) and all
/// eight see its text.
#[test]
fn lazy_load_race_on_one_pair_loads_once() {
    const THREADS: usize = 8;
    for round in 0..50 {
        let body = format!(r#"{{"t": "page {round}"}}"#);
        let i18n = I18n::builder()
            .lazy_files([("pages/p/en.json", body.as_bytes())])
            .build()
            .unwrap();
        let start = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (i18n, start) = (i18n.clone(), start.clone());
                thread::spawn(move || {
                    start.wait();
                    i18n.page("p").t("t")
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), format!("page {round}"));
        }
        // step: one load for eight racing readers
        assert_eq!(loads(&i18n), 1, "round {round}");
    }
}

/// Given 16 lazy pages with `evict_pages(3)`, when eight threads read all pages in
/// different orders (loading, evicting and reloading under each other), then every answer
/// is the page's own text: eviction never shows a reader a missing or foreign page.
#[test]
fn eviction_under_concurrent_readers_is_invisible() {
    const THREADS: usize = 8;
    const PAGES: usize = 16;
    let files: Vec<(String, String)> = (0..PAGES)
        .map(|p| (format!("pages/p{p}/en.json"), format!(r#"{{"t": "{p}"}}"#)))
        .chain(std::iter::once((
            "en.json".to_owned(),
            r#"{"root": "R"}"#.to_owned(),
        )))
        .collect();
    let i18n = I18n::builder()
        .lazy_files(files.iter().map(|(p, b)| (p, b)))
        .evict_pages(3)
        .build()
        .unwrap();
    let start = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let (i18n, start) = (i18n.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                for round in 0..200 {
                    let p = (t * 5 + round * 7) % PAGES;
                    let page = i18n.page(&format!("p{p}"));
                    assert_eq!(page.t("t"), p.to_string());
                    assert_eq!(page.t("root"), "R");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // step: loads happened again after evictions; an eviction changes no answer, so the
    // public version stays
    assert!(loads(&i18n) > PAGES as u64);
    assert_eq!(i18n.version(), 0);
}

/// Given two locales that fall back to each other with lazy pages linking across and links
/// on, when two threads released by one barrier read the two pages of a fresh catalog at
/// once (200 times), then no load waits on another that waits for it: both finish, and each
/// page resolves its link into the other locale's root.
#[test]
fn lazy_links_in_a_fallback_cycle_never_deadlock() {
    let files: [(&str, &[u8]); 4] = [
        ("en.json", br#"{"brand": "Acme"}"#),
        ("de.json", br#"{"marke": "Akme"}"#),
        ("pages/p/en.json", br#"{"a": "A @:marke"}"#),
        ("pages/p/de.json", br#"{"c": "C @:brand"}"#),
    ];
    for _ in 0..200 {
        let i18n = I18n::builder()
            .locale("de")
            .fallback_for("de", "en")
            .fallback_for("en", "de")
            .linked_messages(true)
            .lazy_files(files)
            .build()
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = [("en", "a", "A Akme"), ("de", "c", "C Acme")]
            .into_iter()
            .map(|(locale, key, want)| {
                let (i18n, barrier) = (i18n.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    assert_eq!(i18n.with_locale(locale).page("p").t(key), want);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}

/// Given a listener that is still busy with version 1 on one thread (held by a gate),
/// when another thread switches the locale again, then that write returns at once and the
/// listener sees version 2 only after version 1, never `[(fr, 2), (de, 1)]`. A write
/// from inside a listener does not deadlock: it is delivered after the current change.
#[test]
fn listeners_see_versions_in_order_across_threads() {
    use std::sync::Mutex;
    use std::sync::mpsc::channel;
    let i18n = I18n::builder()
        .messages("de", None, "{}")
        .messages("fr", None, "{}")
        .messages("it", None, "{}")
        .build()
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered) = channel::<()>();
    let (gate, gate_rx) = channel::<()>();
    let gate_rx = Mutex::new(gate_rx);
    let log = seen.clone();
    let inner = i18n.clone();
    let _sub = i18n.subscribe(move |c| {
        if c.version == 1 {
            entered_tx.send(()).unwrap();
            gate_rx.lock().unwrap().recv().unwrap();
            // a write from inside a listener: queued behind this change
            inner.set_locale("it").unwrap();
        }
        log.lock().unwrap().push((c.locale.to_string(), c.version));
    });
    // step 1: thread A switches to de; its listener waits at the gate
    let a = {
        let i18n = i18n.clone();
        thread::spawn(move || i18n.set_locale("de").unwrap())
    };
    entered.recv().unwrap();
    // step 2: fr from here returns without running the listener
    i18n.set_locale("fr").unwrap();
    assert!(seen.lock().unwrap().is_empty());
    // step 3: the gate opens; A delivers 1, then 2, then the listener's own 3
    gate.send(()).unwrap();
    a.join().unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [("de".into(), 1), ("fr".into(), 2), ("it".into(), 3)]
    );
}
