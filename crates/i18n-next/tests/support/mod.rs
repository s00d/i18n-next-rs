//! Environment simulator of the scenario tests (DESIGN 7.1), like `support/sim.rs` in
//! cargo-autoprune. A test binary pulls it in with `mod support;`.
//!
//! M0 holds the skeleton only; each piece grows with the stage that needs it:
//! - [`Locales`]: the locale file tree (M1);
//! - [`Clock`]: virtual time (M3 `watch`, M6 `tdr`);
//! - [`Guard`]: fails a test on any unexpected `missing` or warning (M1);
//! - `FakeFetch` (M3) and the manual executor for `scope().wrap()` (M4) come later.
//!
//! Nothing here sleeps or reads the wall clock: time moves only when a test moves it.
#![allow(dead_code)] // each test binary uses a part

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

/// A tree of locale files kept in memory: root files (`en.json`), page files
/// (`pages/settings/en.json`), and broken ones (bad JSON, BOM, `__proto__`).
///
/// Handed to the crate as `files()`, as a temporary directory for `dir` / `lazy_dir`,
/// or as a `remote` source once those exist (M1, M3).
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

/// Records every `missing` call and warning; on drop, fails the test on any entry
/// that was not allowed with [`Guard::allow`], even when all asserts passed.
#[derive(Debug, Default)]
pub struct Guard {
    seen: RefCell<Vec<String>>,
    allowed: RefCell<Vec<String>>,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `what` (a key such as `"nav.unknown"`) as expected.
    pub fn allow(&self, what: &str) {
        self.allowed.borrow_mut().push(what.to_owned());
    }

    /// Records one event; the crate's `missing` hook calls this (M1).
    pub fn record(&self, what: &str) {
        self.seen.borrow_mut().push(what.to_owned());
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let allowed = self.allowed.borrow();
        let unexpected: Vec<_> = self
            .seen
            .borrow()
            .iter()
            .filter(|s| !allowed.contains(s))
            .cloned()
            .collect();
        assert!(unexpected.is_empty(), "unexpected events: {unexpected:#?}");
    }
}
