//! `remote` sources (DESIGN 3.3): `missing_files`, `add_file` and `ensure_async`.
//!
//! `ensure_async` needs no runtime: it awaits the app's fetch futures and its own two
//! small futures, a join over the files of one call and a wait on a fetch another call
//! already started. One file has at most one fetch in flight per instance: the first
//! caller leads (fetches and adds the file), later callers wait for its outcome. A leader
//! dropped mid-fetch hands the file back: its waiters check again and one of them leads.

use crate::catalog::Tree;
use crate::error::Error;
use crate::evict;
use crate::i18n::{DataMap, I18n, Snapshot};
use crate::lazy::{Layer, LazyCtl, lock};
use crate::source::{Pair, normalize};
use crate::trace;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// How one shared fetch ended.
enum Outcome {
    Ok,
    Err(Error),
    /// The leader was dropped before it finished.
    Cancelled,
}

impl Outcome {
    fn copy(&self, path: &str) -> Outcome {
        match self {
            Outcome::Ok => Outcome::Ok,
            Outcome::Err(e) => Outcome::Err(e.duplicate(path)),
            Outcome::Cancelled => Outcome::Cancelled,
        }
    }
}

/// One fetch in flight: its outcome, once known, and who waits for it.
#[derive(Default)]
pub(crate) struct Flight {
    state: Mutex<(Option<Outcome>, Vec<Waker>)>,
}

/// Waits for another caller's fetch of `path`.
struct Wait<'a> {
    flight: &'a Flight,
    path: &'a str,
}

impl Future for Wait<'_> {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Outcome> {
        let mut state = lock(&self.flight.state);
        if let Some(outcome) = &state.0 {
            return Poll::Ready(outcome.copy(self.path));
        }
        if !state.1.iter().any(|w| w.will_wake(cx.waker())) {
            state.1.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// The leader of one fetch: publishes the outcome, `Cancelled` when dropped unfinished.
struct Lead<'a> {
    ctl: &'a LazyCtl,
    flight: &'a Arc<Flight>,
    path: &'a str,
    done: bool,
}

impl Lead<'_> {
    fn finish(&mut self, outcome: Outcome) {
        self.done = true;
        let wakers = {
            let mut state = lock(&self.flight.state);
            state.0 = Some(outcome);
            std::mem::take(&mut state.1)
        };
        {
            let mut flights = lock(&self.ctl.flights);
            if flights
                .get(self.path)
                .is_some_and(|f| Arc::ptr_eq(f, self.flight))
            {
                flights.remove(self.path);
            }
        }
        for w in wakers {
            w.wake();
        }
    }
}

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.finish(Outcome::Cancelled);
        }
    }
}

/// Polls every future to the end; the outputs in order.
struct JoinAll<F> {
    futures: Vec<Option<Pin<Box<F>>>>,
    outputs: Vec<Option<Result<(), Error>>>,
}

impl<F: Future<Output = Result<(), Error>>> Future for JoinAll<F> {
    type Output = Vec<Result<(), Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut pending = false;
        for (slot, out) in this.futures.iter_mut().zip(&mut this.outputs) {
            if let Some(f) = slot {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => {
                        *out = Some(v);
                        *slot = None;
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(this.outputs.drain(..).flatten().collect())
        }
    }
}

impl I18n {
    /// Paths of the `remote` files that the chain of `locale` still lacks for `page`
    /// (`None` = the root only): for every locale of the chain, its root file, then its
    /// page file; files of pages folded into the root (`pages(false)`) count as root
    /// files. Files being fetched are listed too, until they arrive (the `ensure_async`
    /// call that fetched them shows them when its last file is in). Local lazy files are
    /// not listed: they load on first use.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .locale("de")
    ///     .fallback("en")
    ///     .remote(["en.json", "de.json", "pages/settings/de.json"])
    ///     .build()?;
    /// assert_eq!(i18n.missing_files("de", Some("settings")), ["de.json", "pages/settings/de.json", "en.json"]);
    /// i18n.add_file("de.json", br#"{"hi": "Hallo"}"#)?;
    /// assert_eq!(i18n.missing_files("de", None), ["en.json"]);
    /// assert_eq!(i18n.t("hi"), "Hallo");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn missing_files(&self, locale: &str, page: Option<&str>) -> Vec<String> {
        self.inner.read(|s| {
            let page = s.meta.page(page);
            let mut out: Vec<String> = Vec::new();
            for data in s.layers(Some(locale)) {
                let layers = [data.layer(None), page.and_then(|p| data.layer(Some(p)))];
                for layer in layers.into_iter().flatten() {
                    let Layer::Lazy(cell) = layer else { continue };
                    if !cell.remote || cell.loaded().is_some() {
                        continue;
                    }
                    let arrived = lock(&cell.arrived);
                    for (f, a) in cell.files.iter().zip(arrived.iter()) {
                        if f.is_remote() && a.is_none() && !out.iter().any(|p| **p == *f.path) {
                            out.push(String::from(&*f.path));
                        }
                    }
                }
            }
            out
        })
    }

    /// Whether the manifest file `path` still has to be added.
    fn is_missing(&self, path: &str) -> bool {
        let Some((locale, page)) = self.inner.remote.get(path) else {
            return false;
        };
        self.inner.read(|s| {
            let Some(Layer::Lazy(cell)) = s.data.get(locale).and_then(|d| d.layer(page.as_deref()))
            else {
                return false;
            };
            if cell.loaded().is_some() {
                return false;
            }
            let arrived = lock(&cell.arrived);
            cell.files
                .iter()
                .zip(arrived.iter())
                .any(|(f, a)| *f.path == *path && a.is_none())
        })
    }

    /// Adds the bytes of one `remote` manifest file. The pair it belongs to becomes visible
    /// once all of its files are in (one file, as a rule): a new snapshot, version + 1,
    /// subscribers notified. A file that is already in is skipped (`Ok`): each file loads
    /// once, until `reload` or an eviction asks for it again. Pairs of the file's view
    /// whose files are all in but not shown yet (a dropped `ensure_async`) are shown with
    /// it, weakest first, so a link resolves over the layers under it.
    ///
    /// # Errors
    /// [`Error::UnknownFile`] for a path the manifest does not list (names only from the
    /// catalog, 8b.2), [`Error::Json`] / [`Error::NotObject`] with the path for bad bytes.
    /// Nothing changes then.
    pub fn add_file(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let path = normalize(path);
        let (locale, page) = self.arrive(&path, bytes)?;
        self.fill_view(&locale, page.as_deref());
        Ok(())
    }

    /// Parses one manifest file and keeps it in its pair, not shown yet; the file's pair.
    fn arrive(&self, path: &str, bytes: &[u8]) -> Result<Pair, Error> {
        let Some((locale, page)) = self.inner.remote.get(path) else {
            return Err(Error::UnknownFile(path.to_owned()));
        };
        let timer = trace::Timer::start();
        let tree = Tree::parse(bytes, path)?;
        timer.done("add_file", path, bytes.len());
        // Under the writer: a `reload` or an eviction swapping the cell waits.
        self.inner.write(|s| {
            if let Some(Layer::Lazy(cell)) =
                s.data.get(locale).and_then(|d| d.layer(page.as_deref()))
                && cell.loaded().is_none()
                && let Some(i) = cell.files.iter().position(|f| *f.path == *path)
                && let Some(slot @ None) = lock(&cell.arrived).get_mut(i)
            {
                *slot = Some(Arc::new(tree));
            }
            Ok(None)
        })?;
        Ok((locale.clone(), page.clone()))
    }

    /// Shows every remote pair of the view of `(locale, page)` whose files are all in,
    /// weakest layer first (a pair's links resolve over the layers under it, as Nuxt
    /// resolves a view with all of its files): one new snapshot when any was shown.
    fn fill_view(&self, locale: &str, page: Option<&str>) {
        let inner = &*self.inner;
        let limit = inner.ctl.limit;
        let change = inner.write(|s| {
            let page = s.meta.page(page);
            let mut view: Vec<Arc<crate::lazy::LazyChunk>> = Vec::new();
            for data in s.layers(Some(locale)) {
                for layer in [page.and_then(|p| data.layer(Some(p))), data.layer(None)] {
                    if let Some(Layer::Lazy(cell)) = layer
                        && cell.remote
                    {
                        view.push(cell.clone());
                    }
                }
            }
            let mut filled = false;
            let mut evict = false;
            for cell in view.iter().rev() {
                if let Some(evictable) = cell.fill() {
                    filled = true;
                    evict |= evictable;
                }
            }
            if !filled {
                return Ok(None);
            }
            let mut data = match limit {
                Some(n) if evict => evict::plan(&s.data, n, &inner.ctl),
                _ => None,
            }
            .unwrap_or_else(|| DataMap::clone(&s.data));
            if s.meta.linked() {
                respawn_partials(&mut data);
            }
            Ok(Some(Snapshot::new(
                s.meta.clone(),
                data,
                s.locale.clone(),
                s.version + 1,
            )))
        });
        if let Ok(Some(change)) = change {
            self.inner.listeners.send(&change);
        }
    }

    /// Fetches the `remote` files the chain of `locale` lacks for `page` (see
    /// [`I18n::missing_files`]) with `fetch(path)` and adds them; returns when all are in.
    /// The files of one call become visible together when the last one is in: one new
    /// snapshot, the weakest layer filled first, so linked messages resolve the same in
    /// whatever order the files arrive.
    ///
    /// Any runtime: `fetch` is a closure returning a future (gloo-net or `fetch` in the
    /// browser, `tokio::fs` in native). Files are fetched at the same time. A file another
    /// `ensure_async` is already fetching is not fetched again: this call waits for that
    /// one. A failed file leaves the catalog as it was (the other files are still added)
    /// and the call returns the first error.
    ///
    /// ```
    /// use i18n_next::I18n;
    /// use std::future::Future;
    /// use std::task::{Context, Poll, Waker};
    ///
    /// let i18n = I18n::builder().locale("de").remote(["de.json", "pages/cart/de.json"]).build()?;
    /// let fetch = |path: String| async move {
    ///     match path.as_str() {
    ///         "de.json" => Ok(br#"{"hi": "Hallo"}"#.to_vec()),
    ///         _ => Err("offline"),
    ///     }
    /// };
    /// // Any executor; this future completes on its first poll.
    /// let mut fut = Box::pin(i18n.ensure_async("de", None, fetch));
    /// let Poll::Ready(result) = fut.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
    ///     unreachable!()
    /// };
    /// result?;
    /// assert_eq!(i18n.t("hi"), "Hallo");
    /// let mut fut = Box::pin(i18n.ensure_async("de", Some("cart"), fetch));
    /// let Poll::Ready(Err(e)) = fut.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
    ///     unreachable!()
    /// };
    /// assert_eq!(e.to_string(), "cannot fetch pages/cart/de.json: offline");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`Error::Fetch`] with the path and the closure's message, or the error of
    /// [`I18n::add_file`] for bytes that are not a JSON object.
    pub async fn ensure_async<F, Fut, E>(
        &self,
        locale: &str,
        page: Option<&str>,
        fetch: F,
    ) -> Result<(), Error>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, E>>,
        E: fmt::Display,
    {
        let paths = self.missing_files(locale, page);
        let mut result = Ok(());
        if !paths.is_empty() {
            let futures: Vec<_> = paths
                .into_iter()
                .map(|p| Some(Box::pin(self.ensure_file(p, &fetch))))
                .collect();
            let outputs = (0..futures.len()).map(|_| None).collect();
            let results = JoinAll { futures, outputs }.await;
            result = results.into_iter().find(Result::is_err).unwrap_or(Ok(()));
        }
        // The whole set is in (or failed): show it at once, weakest layer first, so the
        // answers do not depend on the order the files arrived in. Files a dropped call
        // left waiting are shown here too.
        self.fill_view(locale, page);
        result
    }

    async fn ensure_file<F, Fut, E>(&self, path: String, fetch: &F) -> Result<(), Error>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, E>>,
        E: fmt::Display,
    {
        let ctl = &*self.inner.ctl;
        loop {
            let (flight, leads) = {
                let mut flights = lock(&ctl.flights);
                match flights.get(path.as_str()) {
                    Some(f) => (f.clone(), false),
                    None => {
                        let f = Arc::new(Flight::default());
                        flights.insert(Box::from(path.as_str()), f.clone());
                        (f, true)
                    }
                }
            };
            if leads {
                let mut lead = Lead {
                    ctl,
                    flight: &flight,
                    path: &path,
                    done: false,
                };
                // A leader that finished just before this one started already added it.
                if !self.is_missing(&path) {
                    lead.finish(Outcome::Ok);
                    return Ok(());
                }
                let fetched = fetch(path.clone());
                #[cfg(feature = "tracing")]
                let fetched = tracing::Instrument::instrument(
                    fetched,
                    tracing::debug_span!("i18n_next::ensure", path = %path),
                );
                let result = match fetched.await {
                    // kept in its pair; the call shows its whole set when it is in
                    Ok(bytes) => self.arrive(&path, &bytes).map(|_| ()),
                    Err(e) => Err(Error::Fetch {
                        path: path.clone(),
                        message: e.to_string(),
                    }),
                };
                lead.finish(match &result {
                    Ok(()) => Outcome::Ok,
                    Err(e) => Outcome::Err(e.duplicate(&path)),
                });
                return result;
            }
            let wait = Wait {
                flight: &flight,
                path: &path,
            };
            match wait.await {
                Outcome::Ok => return Ok(()),
                Outcome::Err(e) => return Err(e),
                Outcome::Cancelled if !self.is_missing(&path) => return Ok(()),
                Outcome::Cancelled => {}
            }
        }
    }
}

/// Remote pairs not in yet whose partial view was made: made again on next use, since a
/// layer their links read may have come in.
fn respawn_partials(data: &mut DataMap) {
    let stale = |c: &crate::lazy::LazyChunk| c.remote && c.loaded().is_none() && c.has_partial();
    for slot in data.values_mut() {
        let any = slot
            .root
            .iter()
            .chain(slot.pages.values())
            .any(|l| matches!(l, Layer::Lazy(c) if stale(c)));
        if !any {
            continue;
        }
        let slot = Arc::make_mut(slot);
        for layer in slot.root.iter_mut().chain(slot.pages.values_mut()) {
            if let Layer::Lazy(c) = layer
                && stale(c)
            {
                *layer = Layer::Lazy(Arc::new(c.respawn()));
            }
        }
    }
}
