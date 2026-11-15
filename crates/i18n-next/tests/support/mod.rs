//! Environment simulator of the scenario tests (DESIGN 7.1), like `support/sim.rs` in
//! cargo-autoprune. A test binary pulls it in with `mod support;`.
//!
//! - [`Locales`]: the locale file tree, handed out as `files()` or as a real directory;
//! - [`Clock`]: virtual time (M3 `watch`, M6 `tdr`);
//! - [`Guard`]: records every `missing` call and fails the test on an unexpected one;
//! - `FakeFetch` (M3) and the manual executor for `scope().wrap()` (M4) come later.
//!
//! Nothing here sleeps or reads the wall clock: time moves only when a test moves it.
#![allow(dead_code)] // each test binary uses a part

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A tree of locale files kept in memory: root files (`en.json`), page files
/// (`pages/settings/en.json`), and broken ones (bad JSON, BOM, `__proto__`).
#[derive(Debug, Default, Clone)]
pub struct Locales {
    files: BTreeMap<String, Vec<u8>>,
}

impl Locales {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file at `path` (relative, `/`-separated).
    pub fn file(mut self, path: &str, body: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.to_owned(), body.into());
        self
    }

    /// Files in path order, the way `files()` takes them.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(p, b)| (p.as_str(), b.as_slice()))
    }

    /// Writes the tree into a fresh directory `name` under the target's test temp dir
    /// (removed first, so a rerun starts clean) and returns its path, for `dir`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn dir(&self, name: &str) -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        for (path, body) in &self.files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}

/// Virtual clock: milliseconds since the Unix epoch that move only on [`Clock::advance`].
/// Passed to the crate through `.clock()` (M6) and to `watch` debounce (M3).
#[derive(Debug, Default)]
pub struct Clock {
    now_ms: Cell<i64>,
}

impl Clock {
    pub fn at(now_ms: i64) -> Self {
        Self {
            now_ms: Cell::new(now_ms),
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.now_ms.get()
    }

    pub fn advance(&self, ms: i64) {
        self.now_ms.set(self.now_ms.get() + ms);
    }
}

#[derive(Debug, Default)]
struct Log {
    seen: Vec<String>,
    allowed: Vec<String>,
}

/// Records every `missing` call (as `locale|page|key`) and warning; on drop of the last
/// clone, fails the test on any entry that was not allowed with [`Guard::allow`], even
/// when all asserts passed. Clones share one log, so one goes into the `missing` hook.
#[derive(Debug, Default, Clone)]
pub struct Guard {
    log: Arc<Mutex<Log>>,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `what` (`locale|page|key`, or a bare event) as expected.
    pub fn allow(&self, what: &str) {
        self.log.lock().unwrap().allowed.push(what.to_owned());
    }

    /// Records one event.
    pub fn record(&self, what: &str) {
        self.log.lock().unwrap().seen.push(what.to_owned());
    }

    /// Everything recorded so far, in order.
    pub fn seen(&self) -> Vec<String> {
        self.log.lock().unwrap().seen.clone()
    }

    /// A `missing` hook for the builder that records `locale|page|key`.
    pub fn missing(&self) -> impl Fn(&str, &str, &str) + Send + Sync + 'static {
        let guard = self.clone();
        move |locale, key, page| guard.record(&format!("{locale}|{page}|{key}"))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if std::thread::panicking() || Arc::strong_count(&self.log) > 1 {
            return;
        }
        let log = self.log.lock().unwrap();
        let unexpected: Vec<_> = log
            .seen
            .iter()
            .filter(|s| !log.allowed.contains(s))
            .cloned()
            .collect();
        assert!(unexpected.is_empty(), "unexpected events: {unexpected:#?}");
    }
}
