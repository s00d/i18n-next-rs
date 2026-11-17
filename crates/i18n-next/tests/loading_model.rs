//! Loading model (DESIGN 7.3, 8b.16): random sessions of lookups, locale switches,
//! `ensure_async` on random pairs, fetch answers in random order (some failing), manual
//! `add_file`, `reload`, against five instances built from the same files:
//!
//! - `eager` (`files`): the truth for the local instances;
//! - `lazy` (`lazy_files`): answers as `eager`; a repeated lookup loads nothing new;
//! - `lazy_ev` (`lazy_files`, `evict_pages(2)`, `pin_page("tray")`): answers as `eager`,
//!   eviction is invisible;
//! - `rem` (`remote` + `FakeFetch`): answers as an eager build of exactly the files the
//!   model says are shown; every fill counted once; never two requests of one file in flight.
//!   A fetched file is in (not listed by `missing_files`) at once, shown when its
//!   `ensure_async` call ends (or an `add_file` / later call of its view), with the rest of
//!   the call's view;
//! - `rem_ev` (`remote`, `evict_pages(2)`, pinned `tray`): answers as an eager build of the
//!   files `missing_files` says are in, less those waiting to be shown; at most 2 unpinned
//!   pages in; `tray` never leaves.
//!
//! Versions only grow, on every instance. Fixed seed, no shrinking persistence.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::I18n;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::{Executor, FakeFetch, loads};

const LOCALES: [&str; 2] = ["en", "de"];
const PAGES: [&str; 4] = ["p0", "p1", "p2", "tray"];
const KEYS: [&str; 6] = ["id", "k0", "k1", "k2", "k3", "nope"];

// What the sessions reached: evictions seen in `rem_ev`, failed fetches, pages `lazy_ev`
// loaded again after an eviction.
static EVICTED: AtomicUsize = AtomicUsize::new(0);
static ERRORS: AtomicUsize = AtomicUsize::new(0);
static RELOADED: AtomicUsize = AtomicUsize::new(0);

/// A file of the catalog: path, locale index, page index (`None` = root), body.
struct File {
    path: String,
    locale: usize,
    page: Option<usize>,
    body: String,
}

fn files() -> Vec<File> {
    let mut out = Vec::new();
    for (li, l) in LOCALES.iter().enumerate() {
        for pi in std::iter::once(None).chain((0..PAGES.len()).map(Some)) {
            // `de` has no p2: that page reads the en layers there.
            if li == 1 && pi == Some(2) {
                continue;
            }
            let (path, name) = match pi {
                None => (format!("{l}.json"), "root"),
                Some(p) => (format!("pages/{}/{l}.json", PAGES[p]), PAGES[p]),
            };
            let mut body = format!(r#"{{"id": "{l}/{name}""#);
            for k in 0..4 {
                if (li * 7 + pi.unwrap_or(4) * 3 + k) % 3 != 0 {
                    body.push_str(&format!(r#", "k{k}": "{l}/{name}/k{k}""#));
                }
            }
            body.push('}');
            out.push(File {
                path,
                locale: li,
                page: pi,
                body,
            });
        }
    }
    out
}

#[derive(Debug, Clone)]
enum Action {
    Lookup {
        locale: Option<usize>,
        page: Option<usize>,
        key: usize,
    },
    SetLocale(usize),
    /// `ensure_async` on `rem` (false) or `rem_ev` (true).
    Ensure {
        ev: bool,
        locale: usize,
        page: Option<usize>,
    },
    /// Answers pending request `index % len` of one fake network, then polls.
    Answer {
        ev: bool,
        index: usize,
        ok: bool,
    },
    AddFile {
        ev: bool,
        file: usize,
    },
    Reload,
}

fn action(n_files: usize) -> impl Strategy<Value = Action> {
    prop_oneof![
        5 => (
            proptest::option::of(0..LOCALES.len()),
            proptest::option::of(0..PAGES.len()),
            0..KEYS.len()
        )
            .prop_map(|(locale, page, key)| Action::Lookup { locale, page, key }),
        1 => (0..LOCALES.len()).prop_map(Action::SetLocale),
        3 => (any::<bool>(), 0..LOCALES.len(), proptest::option::of(0..PAGES.len()))
            .prop_map(|(ev, locale, page)| Action::Ensure { ev, locale, page }),
        4 => (any::<bool>(), any::<usize>(), proptest::bool::weighted(0.8))
            .prop_map(|(ev, index, ok)| Action::Answer { ev, index, ok }),
        1 => (any::<bool>(), 0..n_files).prop_map(|(ev, file)| Action::AddFile { ev, file }),
        1 => Just(Action::Reload),
    ]
}

/// An eager build of `files` (by index): what a remote instance with exactly those files
/// in must answer.
fn reference(all: &[File], loaded: &BTreeSet<usize>, active: &str) -> I18n {
    I18n::builder()
        .locale(active)
        .fallback("en")
        .files(
            loaded
                .iter()
                .map(|&i| (all[i].path.as_str(), all[i].body.as_bytes())),
        )
        .build()
        .unwrap()
}

/// `try_t` through the same view on any instance.
fn ask(i: &I18n, locale: Option<usize>, page: Option<usize>, key: usize) -> Option<String> {
    let view = match locale {
        Some(l) => i.with_locale(LOCALES[l]),
        None => i.page(""),
    };
    match page {
        Some(p) => view.page(PAGES[p]).get(KEYS[key], |t| t.map(str::to_owned)),
        None => view.get(KEYS[key], |t| t.map(str::to_owned)),
    }
}

/// Files `missing_files` does not list: what the instance has in.
fn observed(i: &I18n, all: &[File]) -> BTreeSet<usize> {
    (0..all.len())
        .filter(|&f| {
            let file = &all[f];
            let page = file.page.map(|p| PAGES[p]);
            !i.missing_files(LOCALES[file.locale], page)
                .contains(&file.path)
        })
        .collect()
}

/// Files in the view of `(locale, page)`: the locale and the global fallback `en`, the page
/// (if any) and the root.
fn in_view(f: &File, locale: usize, page: Option<usize>) -> bool {
    (f.locale == locale || f.locale == 0) && (f.page.is_none() || f.page == page)
}

/// Where an `ensure_async` call's result lands.
type Outcome = Rc<RefCell<Option<Result<(), i18n_next::Error>>>>;

/// One instance's remote files in the model: fetched and waiting to be shown, shown.
#[derive(Default)]
struct Remote {
    waiting: BTreeSet<usize>,
    shown: BTreeSet<usize>,
    fills: u64,
    /// `ensure_async` calls not ended yet: their view and their result cell.
    calls: Vec<(usize, Option<usize>, Outcome)>,
}

impl Remote {
    /// `f` arrived (fetched or added) on `i`, which had the files `before` in: a file
    /// shown already stays as it is (`rem_ev` may have evicted a shown one, so `shown` alone
    /// cannot tell).
    fn arrive(&mut self, f: usize, before: &BTreeSet<usize>) {
        if !before.contains(&f) || self.waiting.contains(&f) {
            self.waiting.insert(f);
        }
    }

    /// Shows the waiting files of a view.
    fn show(&mut self, all: &[File], locale: usize, page: Option<usize>) {
        let now: Vec<usize> = self
            .waiting
            .iter()
            .copied()
            .filter(|&f| in_view(&all[f], locale, page))
            .collect();
        for f in now {
            self.waiting.remove(&f);
            self.shown.insert(f);
            self.fills += 1;
        }
    }

    /// Calls that ended show their views.
    fn ended(&mut self, all: &[File]) {
        let calls = std::mem::take(&mut self.calls);
        for (locale, page, out) in calls {
            if out.borrow().is_some() {
                self.show(all, locale, page);
            } else {
                self.calls.push((locale, page, out));
            }
        }
    }
}

fn run(actions: &[Action]) -> Result<(), TestCaseError> {
    let all = files();
    let paths: Vec<&str> = all.iter().map(|f| f.path.as_str()).collect();
    let bodies = || all.iter().map(|f| (f.path.as_str(), f.body.as_bytes()));
    let base = || I18n::builder().locale("en").fallback("en");
    let eager = base().files(bodies()).build().unwrap();
    let lazy = base().lazy_files(bodies()).build().unwrap();
    let lazy_ev = base()
        .lazy_files(bodies())
        .evict_pages(2)
        .pin_page("tray")
        .build()
        .unwrap();
    let rem = base().remote(&paths).build().unwrap();
    let rem_ev = base()
        .remote(&paths)
        .evict_pages(2)
        .pin_page("tray")
        .build()
        .unwrap();
    let (net, net_ev) = (FakeFetch::new(), FakeFetch::new());
    let mut ex = Executor::new();
    let mut ex_ev = Executor::new();

    let mut active = 0usize;
    let mut model = Remote::default();
    let mut model_ev = Remote::default();
    let mut seen_lookups: BTreeSet<(Option<usize>, Option<usize>, usize)> = BTreeSet::new();
    let mut lazy_base = 0u64;
    let mut tray_in: BTreeSet<usize> = BTreeSet::new();
    let mut ev_in: BTreeSet<usize> = BTreeSet::new();
    let mut versions = [0u64; 5];
    let instances = [&eager, &lazy, &lazy_ev, &rem, &rem_ev];
    let tray = 3;

    for (step, a) in actions.iter().enumerate() {
        match a {
            Action::Lookup { locale, page, key } => {
                let want = ask(&eager, *locale, *page, *key);
                let before = loads(&lazy);
                prop_assert_eq!(
                    &ask(&lazy, *locale, *page, *key),
                    &want,
                    "lazy, step {}",
                    step
                );
                if !seen_lookups.insert((*locale, *page, *key)) {
                    prop_assert_eq!(
                        loads(&lazy),
                        before,
                        "a repeated lookup loaded, step {}",
                        step
                    );
                }
                prop_assert!(loads(&lazy) - lazy_base <= all.len() as u64);
                let ev_before = loads(&lazy_ev);
                prop_assert_eq!(
                    &ask(&lazy_ev, *locale, *page, *key),
                    &want,
                    "lazy_ev, step {}",
                    step
                );
                if loads(&lazy_ev) > ev_before && seen_lookups.contains(&(*locale, *page, *key)) {
                    RELOADED.fetch_add(1, Ordering::Relaxed);
                }
                let r = reference(&all, &model.shown, LOCALES[active]);
                prop_assert_eq!(
                    ask(&rem, *locale, *page, *key),
                    ask(&r, *locale, *page, *key),
                    "rem with {:?}, step {}",
                    model.shown,
                    step
                );
                let got = ask(&rem_ev, *locale, *page, *key);
                let shown_ev: BTreeSet<usize> = observed(&rem_ev, &all)
                    .difference(&model_ev.waiting)
                    .copied()
                    .collect();
                let r = reference(&all, &shown_ev, LOCALES[active]);
                prop_assert_eq!(got, ask(&r, *locale, *page, *key), "rem_ev, step {}", step);
            }
            Action::SetLocale(l) => {
                active = *l;
                for i in instances {
                    i.set_locale(LOCALES[*l]).unwrap();
                }
            }
            Action::Ensure { ev, locale, page } => {
                let name = page.map(|p| PAGES[p]);
                if *ev {
                    let out =
                        ex_ev.spawn(rem_ev.ensure_async(LOCALES[*locale], name, net_ev.fetch()));
                    model_ev.calls.push((*locale, *page, out));
                    ex_ev.run_until_stalled();
                    model_ev.ended(&all);
                } else {
                    let out = ex.spawn(rem.ensure_async(LOCALES[*locale], name, net.fetch()));
                    model.calls.push((*locale, *page, out));
                    ex.run_until_stalled();
                    model.ended(&all);
                }
            }
            Action::Answer { ev, index, ok } => {
                let (fake, exec, m, i) = if *ev {
                    (&net_ev, &mut ex_ev, &mut model_ev, &rem_ev)
                } else {
                    (&net, &mut ex, &mut model, &rem)
                };
                let pending = fake.pending();
                if pending.is_empty() {
                    continue;
                }
                let at = index % pending.len();
                let file = paths.iter().position(|p| *p == pending[at]).unwrap();
                let answer = if *ok {
                    Ok(all[file].body.as_bytes())
                } else {
                    Err("net down")
                };
                ERRORS.fetch_add(usize::from(!*ok), Ordering::Relaxed);
                let before = observed(i, &all);
                fake.answer_at(at, answer);
                if *ok {
                    m.arrive(file, &before);
                }
                exec.run_until_stalled();
                m.ended(&all);
            }
            Action::AddFile { ev, file } => {
                let f = &all[*file];
                let (i, m) = if *ev {
                    (&rem_ev, &mut model_ev)
                } else {
                    (&rem, &mut model)
                };
                let before = observed(i, &all);
                i.add_file(&f.path, f.body.as_bytes()).unwrap();
                m.arrive(*file, &before);
                m.show(&all, f.locale, f.page);
            }
            Action::Reload => {
                for i in instances {
                    i.reload().unwrap();
                }
                for m in [&mut model, &mut model_ev] {
                    m.waiting.clear();
                    m.shown.clear();
                }
                seen_lookups.clear();
                lazy_base = loads(&lazy);
                tray_in.clear();
            }
        }
        // after every step
        for (n, i) in instances.iter().enumerate() {
            let v = i.version();
            prop_assert!(
                v >= versions[n],
                "instance {} version went back, step {}",
                n,
                step
            );
            versions[n] = v;
        }
        // every fill of `rem` counted once, errors added nothing
        prop_assert_eq!(loads(&rem), model.fills, "rem loads, step {}", step);
        prop_assert_eq!(
            observed(&rem, &all),
            model
                .shown
                .union(&model.waiting)
                .copied()
                .collect::<BTreeSet<_>>(),
            "rem files in, step {}",
            step
        );
        // one request per file in flight
        let pending = net.pending();
        let unique: BTreeSet<&String> = pending.iter().collect();
        prop_assert_eq!(
            unique.len(),
            pending.len(),
            "two requests of one file: {:?}",
            pending
        );
        // rem_ev: at most 2 unpinned pages shown, tray never leaves
        let now: BTreeSet<usize> = observed(&rem_ev, &all)
            .difference(&model_ev.waiting)
            .copied()
            .collect();
        if !matches!(a, Action::Reload) && !ev_in.is_subset(&now) {
            EVICTED.fetch_add(1, Ordering::Relaxed);
        }
        let pages_in = now
            .iter()
            .filter(|&&f| all[f].page.is_some_and(|p| p != tray))
            .count();
        prop_assert!(pages_in <= 2, "{} pages in, step {}", pages_in, step);
        for f in now.iter().filter(|&&f| all[f].page == Some(tray)) {
            tray_in.insert(*f);
        }
        prop_assert!(tray_in.is_subset(&now), "a pinned page left, step {}", step);
        ev_in = now;
    }
    Ok(())
}

#[test]
fn loading_matches_the_model() {
    let config = Config {
        cases: 256,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[3; 32]));
    let n = files().len();
    runner
        .run(&proptest::collection::vec(action(n), 1..60), |a| run(&a))
        .unwrap();
    let (evicted, errors, reloaded) = (
        EVICTED.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
        RELOADED.load(Ordering::Relaxed),
    );
    eprintln!(
        "remote evictions: {evicted}, failed fetches: {errors}, lazy reloads after eviction: {reloaded}"
    );
    assert!(evicted > 0 && errors > 0 && reloaded > 0);
}
