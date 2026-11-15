//! Readers and writers at once (DESIGN 4.5, S1): every answer a reader sees belongs to
//! some published snapshot, a reader never goes back to an older one, and a reader that
//! alternates between more instances than its thread cache holds (4) still reads each one
//! right. Threads start together on a `Barrier`; nothing sleeps, readers stop when they see
//! the writer's last value.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::I18n;
use std::sync::{Arc, Barrier};
use std::thread;

const WRITES: u64 = 2000;
const INSTANCES: usize = 6;
const READERS: usize = 4;

fn build(n: usize) -> I18n {
    I18n::builder()
        .locale("en")
        .messages("en", None, &format!(r#"{{"k": "0", "id": "{n}"}}"#))
        .build()
        .unwrap()
}

/// Given six catalogs and one writer per catalog, when four readers read all six in turn
/// while the writers publish 2000 versions each, then every value read is a published one,
/// never older than the last one this reader saw, and the version agrees with it.
#[test]
fn readers_see_whole_snapshots_in_order() {
    let catalogs: Vec<I18n> = (0..INSTANCES).map(build).collect();
    let start = Arc::new(Barrier::new(READERS + INSTANCES));
    let mut handles = Vec::new();

    for i18n in &catalogs {
        let (i18n, start) = (i18n.clone(), start.clone());
        handles.push(thread::spawn(move || {
            start.wait();
            for v in 1..=WRITES {
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
            let mut last = [0u64; INSTANCES];
            let mut done = 0;
            while done < INSTANCES {
                done = 0;
                for (n, i18n) in catalogs.iter().enumerate() {
                    let before = i18n.version();
                    let k: u64 = i18n.get("k").unwrap().parse().unwrap();
                    let after = i18n.version();
                    // step: the value is a published snapshot, between the versions seen around it
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
                    done += usize::from(k == WRITES);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}
