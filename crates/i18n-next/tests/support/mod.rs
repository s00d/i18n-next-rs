//! Environment simulator of the scenario tests (DESIGN 7.1), like `support/sim.rs` in
//! cargo-autoprune. A test binary pulls it in with `mod support;`.
//!
//! - [`Locales`]: the locale file tree, handed out as `files()` or as a real directory;
//! - [`Clock`]: virtual time (M3 `watch`, M6 `tdr`);
//! - [`Guard`]: records every `missing` call and fails the test on an unexpected one;
//! - [`FakeFetch`]: the fetch closure of `ensure_async`; requests wait until the test
//!   answers them, in any order, with bytes or an error;
//! - [`Executor`]: polls futures in the order a test chooses, no runtime (M4 reuses it for
//!   `scope().wrap()`);
//! - [`loads`]: lazy and remote loads of an instance (its `Debug`).
//!
//! Nothing here sleeps or reads the wall clock: time moves only when a test moves it.
#![allow(dead_code)] // each test binary uses a part

mod fetch;
#[allow(unused_imports)] // each test binary uses a part
pub use fetch::{Executor, FakeFetch, loads};

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
    /// (removed first, so a rerun starts clean), for `dir`; the directory goes when the
    /// returned guard is dropped, also when the test fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn dir(&self, name: &str) -> TempDir {
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        for (path, body) in &self.files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        TempDir(root)
    }
}

/// A directory a test wrote, removed on drop. Derefs to its path.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct TempDir(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl std::ops::Deref for TempDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AsRef<std::path::Path> for TempDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
    checked: bool,
}

impl Log {
    /// Every seen entry no `allow` paid for: each `allow` covers one occurrence.
    fn unexpected(&self) -> Vec<String> {
        let mut budget = self.allowed.clone();
        let mut out = Vec::new();
        for s in &self.seen {
            match budget.iter().position(|a| a == s) {
                Some(i) => {
                    budget.swap_remove(i);
                }
                None => out.push(s.clone()),
            }
        }
        out
    }
}

/// Records every `missing` call (as `locale|page|key`) and warning, and fails the test on
/// any entry that was not allowed with [`Guard::allow`], even when all asserts passed.
///
/// - `allow` is count-based: one `allow("x")` permits one `x`; a second `x` is unexpected.
/// - The check runs at [`Guard::finish`]; without it, on drop of the last clone. Clones
///   share one log and one of them lives in the catalog's `missing` hook, so the last
///   clone is whichever of guard and catalog goes last: the check runs in any
///   declaration order. Call `finish` to check at a point of the test's choosing.
#[derive(Debug, Default, Clone)]
pub struct Guard {
    log: Arc<Mutex<Log>>,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Permits one occurrence of `what` (`locale|page|key`, or a bare event).
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

    /// What the check would report now: entries no allowance covers.
    pub fn unexpected(&self) -> Vec<String> {
        self.log.lock().unwrap().unexpected()
    }

    /// Checks now, whoever else still holds a clone; the drop check is then skipped.
    pub fn finish(&self) {
        let unexpected = {
            let mut log = self.log.lock().unwrap();
            log.checked = true;
            log.unexpected()
        };
        assert!(unexpected.is_empty(), "unexpected events: {unexpected:#?}");
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if std::thread::panicking() || Arc::strong_count(&self.log) > 1 {
            return;
        }
        let unexpected = {
            let log = self.log.lock().unwrap();
            if log.checked {
                return;
            }
            log.unexpected()
        };
        assert!(unexpected.is_empty(), "unexpected events: {unexpected:#?}");
    }
}
