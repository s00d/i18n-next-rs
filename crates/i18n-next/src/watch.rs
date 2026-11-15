//! `watch` (DESIGN 8b.5): reload on file changes in dev, without `notify`. A thread polls
//! the size and mtime of every file under the `dir` / `lazy_dir` sources; once the files
//! stop changing for [`SETTLE_MS`] it calls `reload` (a burst of saves is one reload). Only
//! in debug builds and not on wasm: elsewhere `watch()` returns a guard that does nothing.
//!
//! The loop reads time and file stats through [`Env`], so tests feed synthetic stats and
//! move a virtual clock: nothing in them sleeps.

use crate::i18n::{I18n, Inner};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::SystemTime;

/// Time between two polls.
pub(crate) const POLL_MS: u64 = 100;
/// How long the files must stay unchanged before the reload.
pub(crate) const SETTLE_MS: u64 = 150;

/// One file as the poll sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stat {
    pub(crate) path: PathBuf,
    pub(crate) modified: Option<SystemTime>,
    pub(crate) len: u64,
}

/// Debounce of the poll loop: what the last scan saw and when it last changed.
#[derive(Debug)]
pub(crate) struct Poller {
    seen: Vec<Stat>,
    changed_at: Option<u64>,
}

impl Poller {
    pub(crate) fn new(seen: Vec<Stat>) -> Poller {
        Poller {
            seen,
            changed_at: None,
        }
    }

    /// One poll at `now_ms` that saw `scan`: whether to reload now.
    pub(crate) fn poll(&mut self, now_ms: u64, scan: Vec<Stat>) -> bool {
        if scan != self.seen {
            self.seen = scan;
            self.changed_at = Some(now_ms);
            return false;
        }
        match self.changed_at {
            Some(at) if now_ms.saturating_sub(at) >= SETTLE_MS => {
                self.changed_at = None;
                true
            }
            _ => false,
        }
    }
}

/// Time and file stats of the poll loop.
pub(crate) trait Env {
    fn now_ms(&self) -> u64;
    fn scan(&self) -> Vec<Stat>;
    /// Waits `ms`; `false` when the loop must stop.
    fn wait(&self, ms: u64) -> bool;
}

/// Every file under `dirs`, sorted by path. A directory that cannot be listed (removed
/// mid-edit) contributes nothing: its return is a change like any other.
pub(crate) fn scan_dirs(dirs: &[PathBuf]) -> Vec<Stat> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(files) = crate::loader::list_dir(dir) else {
            continue;
        };
        for (_, path) in files {
            let meta = std::fs::metadata(&path).ok();
            out.push(Stat {
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                len: meta.map_or(0, |m| m.len()),
                path,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The watched directories of an instance.
pub(crate) fn dirs(inner: &Inner) -> Vec<PathBuf> {
    let s = &inner.sources;
    s.dirs.iter().chain(&s.lazy_dirs).cloned().collect()
}

/// The poll loop: until `env` says stop or the instance is gone. A reload that fails (a
/// file saved half-way) is reported and the old catalog stays; the next save retries.
pub(crate) fn run(env: &impl Env, i18n: &Weak<Inner>) {
    let mut poller = Poller::new(env.scan());
    while env.wait(POLL_MS) {
        let Some(inner) = i18n.upgrade() else {
            return;
        };
        if poller.poll(env.now_ms(), env.scan())
            && let Err(e) = (I18n { inner }).reload()
        {
            crate::trace::broken(&e);
        }
    }
}

type Stop = Arc<(Mutex<bool>, Condvar)>;

/// The real clock and file system, stopped through the guard's condvar.
struct Real {
    dirs: Vec<PathBuf>,
    start: std::time::Instant,
    stop: Stop,
}

impl Env for Real {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn scan(&self) -> Vec<Stat> {
        scan_dirs(&self.dirs)
    }

    fn wait(&self, ms: u64) -> bool {
        let (flag, cv) = &*self.stop;
        let stopped = flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (stopped, _) = cv
            .wait_timeout_while(stopped, std::time::Duration::from_millis(ms), |s| !*s)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !*stopped
    }
}

/// Keeps [`I18n::watch`] polling; dropping it stops the thread and joins it.
#[must_use = "dropping the guard stops watching"]
pub struct WatchGuard {
    stop: Stop,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for WatchGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchGuard")
            .field("polling", &self.thread.is_some())
            .finish()
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        let (flag, cv) = &*self.stop;
        *flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl I18n {
    /// Reloads the catalog when files under its `dir` / `lazy_dir` sources change (dev,
    /// feature `watch`): a thread polls sizes and mtimes every 100 ms and reloads once they
    /// stay unchanged for 150 ms, so subscribers see one [`crate::Change`] per burst of
    /// saves. A reload that fails keeps the old catalog. Dropping the guard stops the
    /// thread. In release builds, on wasm and without such sources it does nothing.
    pub fn watch(&self) -> WatchGuard {
        let stop: Stop = Arc::default();
        let dirs = dirs(&self.inner);
        let active = cfg!(all(debug_assertions, not(target_arch = "wasm32"))) && !dirs.is_empty();
        let thread = active
            .then(|| {
                let env = Real {
                    dirs,
                    start: std::time::Instant::now(),
                    stop: stop.clone(),
                };
                let weak = Arc::downgrade(&self.inner);
                std::thread::Builder::new()
                    .name("i18n-next-watch".to_owned())
                    .spawn(move || run(&env, &weak))
                    .ok()
            })
            .flatten();
        WatchGuard { stop, thread }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::time::Duration;

    fn stat(name: &str, secs: u64, len: u64) -> Stat {
        Stat {
            path: PathBuf::from(name),
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            len,
        }
    }

    /// A virtual clock and a script of scans: each `wait` moves the clock, each `scan`
    /// takes the next scripted one (the last one repeats); the loop stops after `polls`.
    struct Virtual {
        now: Cell<u64>,
        scans: RefCell<VecDeque<Vec<Stat>>>,
        last: RefCell<Vec<Stat>>,
        polls: Cell<usize>,
    }

    impl Env for Virtual {
        fn now_ms(&self) -> u64 {
            self.now.get()
        }
        fn scan(&self) -> Vec<Stat> {
            if let Some(next) = self.scans.borrow_mut().pop_front() {
                *self.last.borrow_mut() = next;
            }
            self.last.borrow().clone()
        }
        fn wait(&self, ms: u64) -> bool {
            self.now.set(self.now.get() + ms);
            let left = self.polls.get();
            self.polls.set(left.saturating_sub(1));
            left > 0
        }
    }

    /// Given a poller that saw two files, when saves land at 100 and 200 ms and the files
    /// then stay still, then exactly one reload is due, at the first poll 150 ms after
    /// the last change; a later unchanged poll reloads nothing.
    #[test]
    fn a_burst_of_saves_is_one_reload() {
        let a = vec![stat("en.json", 1, 10), stat("de.json", 1, 10)];
        let mut p = Poller::new(a.clone());
        // step 1: nothing changed → no reload
        assert!(!p.poll(0, a.clone()));
        // step 2: two saves in a row → no reload while they keep coming
        let b = vec![stat("en.json", 2, 11), stat("de.json", 1, 10)];
        let c = vec![stat("en.json", 3, 12), stat("de.json", 1, 10)];
        assert!(!p.poll(100, b));
        assert!(!p.poll(200, c.clone()));
        // step 3: still at 300 (100 ms after the last change) → not yet
        assert!(!p.poll(300, c.clone()));
        // step 4: at 350 (150 ms) → the reload
        assert!(p.poll(350, c.clone()));
        // step 5: unchanged afterwards → nothing more
        assert!(!p.poll(450, c.clone()));
        // step 6: a size-only change (same mtime: coarse file systems) counts
        let d = vec![stat("en.json", 3, 99), stat("de.json", 1, 10)];
        assert!(!p.poll(500, d.clone()));
        assert!(p.poll(650, d));
        // step 7: a file appears, then disappears before it settles → one reload of the end state
        let e = vec![stat("en.json", 3, 99)];
        assert!(!p.poll(700, vec![stat("en.json", 3, 99), stat("x.json", 4, 1)]));
        assert!(!p.poll(800, e.clone()));
        assert!(p.poll(950, e));
    }

    /// Given an instance and the poll loop on a virtual clock, when the scripted scans
    /// change twice in a burst, then the loop reloads once (version + 1, one `Change`),
    /// and it stops when the instance is dropped.
    #[test]
    fn the_loop_reloads_once_per_burst_and_stops_without_the_instance() {
        let i18n = I18n::builder()
            .messages("en", None, r#"{"a": "A"}"#)
            .build()
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let _sub = i18n.subscribe(move |c| log.lock().unwrap().push(c.version));
        let one = vec![stat("en.json", 1, 1)];
        let two = vec![stat("en.json", 2, 2)];
        let three = vec![stat("en.json", 3, 3)];
        // step 1: initial scan, then two changes, then quiet for 8 polls
        let env = Virtual {
            now: Cell::new(0),
            scans: RefCell::new(VecDeque::from([one, two, three])),
            last: RefCell::new(Vec::new()),
            polls: Cell::new(10),
        };
        run(&env, &Arc::downgrade(&i18n.inner));
        // step 2: one reload for the burst
        assert_eq!(*seen.lock().unwrap(), [1]);
        assert_eq!(i18n.version(), 1);
        assert_eq!(i18n.t("a"), "A");
        // step 3: with the instance gone the loop returns at its next poll
        let weak = Arc::downgrade(&i18n.inner);
        drop(i18n);
        let env = Virtual {
            now: Cell::new(0),
            scans: RefCell::new(VecDeque::new()),
            last: RefCell::new(Vec::new()),
            polls: Cell::new(usize::MAX),
        };
        run(&env, &weak);
        assert_eq!(env.now.get(), POLL_MS);
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("i18n-next-watch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages/settings")).unwrap();
        dir
    }

    /// Given a real directory (eager root, lazy page) and the poller driven by hand on a
    /// virtual clock, when a file is saved, then half-saved, then saved whole, then the
    /// catalog reloads after each settled change, a broken save keeps the old answers,
    /// and the lazy page shows the new text on its next use.
    #[test]
    fn a_real_directory_driven_by_hand() {
        let dir = temp("real");
        std::fs::write(dir.join("en.json"), r#"{"a": "one"}"#).unwrap();
        std::fs::write(dir.join("pages/settings/en.json"), r#"{"t": "Settings"}"#).unwrap();
        let lazy = temp("real-lazy");
        std::fs::write(
            lazy.join("pages/settings/de.json"),
            r#"{"t": "Einstellungen"}"#,
        )
        .unwrap();
        let i18n = I18n::builder().dir(&dir).lazy_dir(&lazy).build().unwrap();
        let watched = dirs(&i18n.inner);
        assert_eq!(watched, [dir.clone(), lazy.clone()]);
        let mut poller = Poller::new(scan_dirs(&watched));
        assert_eq!(
            i18n.with_locale("de").page("settings").t("t"),
            "Einstellungen"
        );

        // step 1: save the root with new text (another length: mtime alone may not move)
        std::fs::write(dir.join("en.json"), r#"{"a": "two!"}"#).unwrap();
        assert!(!poller.poll(0, scan_dirs(&watched)));
        assert!(poller.poll(SETTLE_MS, scan_dirs(&watched)));
        i18n.reload().unwrap();
        assert_eq!((i18n.t("a"), i18n.version()), ("two!".to_owned(), 1));

        // step 2: a half-written save → the reload fails, the old answers stay
        std::fs::write(dir.join("en.json"), r#"{"a": "thr"#).unwrap();
        assert!(!poller.poll(1000, scan_dirs(&watched)));
        assert!(poller.poll(1000 + SETTLE_MS, scan_dirs(&watched)));
        assert!(i18n.reload().is_err());
        assert_eq!((i18n.t("a"), i18n.version()), ("two!".to_owned(), 1));

        // step 3: the lazy page changes → reloaded, read again on next use
        std::fs::write(dir.join("en.json"), r#"{"a": "three"}"#).unwrap();
        std::fs::write(lazy.join("pages/settings/de.json"), r#"{"t": "Optionen"}"#).unwrap();
        assert!(!poller.poll(2000, scan_dirs(&watched)));
        assert!(poller.poll(2000 + SETTLE_MS, scan_dirs(&watched)));
        i18n.reload().unwrap();
        assert_eq!(i18n.t("a"), "three");
        assert_eq!(i18n.with_locale("de").page("settings").t("t"), "Optionen");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&lazy);
    }

    /// Given a watched directory, when the guard is dropped, then the thread stops at once
    /// (the condvar wakes it; no wait for the poll period) and is joined.
    #[test]
    fn dropping_the_guard_joins_the_thread() {
        let dir = temp("guard");
        std::fs::write(dir.join("en.json"), "{}").unwrap();
        let i18n = I18n::builder().dir(&dir).build().unwrap();
        let guard = i18n.watch();
        assert_eq!(guard.thread.is_some(), cfg!(debug_assertions));
        drop(guard);
        // without sources nothing runs
        let none = I18n::builder().build().unwrap().watch();
        assert!(none.thread.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
