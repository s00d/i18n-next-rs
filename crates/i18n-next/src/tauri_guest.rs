//! A Rust front end inside Tauri (Leptos, Yew, Dioxus in wasm): the Rust twin of guest-js
//! (DESIGN 2.1, 6.2). Feature `tauri-guest`, `wasm32-unknown-unknown` only, through `js-sys`
//! alone (no `web-sys`).
//!
//! [`connect`] builds an [`I18n`] from the plugin's `meta` ([`payload::builder`]) and fills it
//! with the chunks of the active chain. Data comes over the `i18n-next://` protocol (the
//! platform's `fetch` of `convertFileSrc` URLs, `/{tag}/{locale}/{page}.json`, cached by
//! content tag), control over commands; when `fetch` rejects with a `TypeError` (no protocol
//! for this WebView), raw invoke `load` carries the same chunks from then on. Chunks go in
//! through [`payload::apply`]: the instance reads the whole fallback chain itself, `linked`
//! kept apart, as on the server. `i18n-next://changed` from Rust and other windows is
//! followed; an event of an older version than the one shown is dropped, a `410` (the
//! catalog moved between `meta` and a read) reads `meta` again.
//!
//! Every call waits for the ones before it, events included. Rust has no "shown page": a
//! page is read with `t!(@page = ..)`, [`I18n::page`] or a `scope`; [`set_page`] loads it
//! and keeps it following locale switches and changes.
//!
//! ```no_run
//! # async fn run() -> Result<(), i18n_next::Error> {
//! let i18n = i18n_next::tauri_guest::connect("index").await?;
//! i18n_next::install(i18n)?;                              // t!("key") in components
//! i18n_next::tauri_guest::set_page("settings").await?;    // its chunks, then t!(@page = "settings")
//! i18n_next::tauri_guest::set_locale("de").await?;        // Rust, native labels, every window
//! # Ok(()) }
//! ```

use crate::catalog::Tree;
use crate::error::Error;
use crate::i18n::I18n;
use crate::json::write_str;
use crate::payload::{self, Meta, Stored};
use js_sys::futures::{JsFuture, spawn_local};
use js_sys::wasm_bindgen::closure::Closure;
use js_sys::wasm_bindgen::{JsCast, JsValue};
use js_sys::{Array, ArrayBuffer, Function, Object, Promise, Reflect, Uint8Array};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

/// The event every window gets after a change (DESIGN 6.1).
const CHANGED: &str = "i18n-next://changed";

thread_local! {
    static LINK: RefCell<Option<Rc<Link>>> = const { RefCell::new(None) };
    static REPORT: RefCell<Report> = RefCell::new(Report::default());
    /// Ids of links, so a `missing` handler knows the connection it belongs to.
    static LINKS: Cell<u64> = const { Cell::new(0) };
    /// The instance of the last connection and the tag of each chunk it holds: the next
    /// [`connect`] takes the chunks with the same tag from it instead of reading them.
    static LAST: RefCell<Option<(I18n, HashMap<String, String>)>> = const { RefCell::new(None) };
}

/// The connection: one per WebView.
struct Link {
    id: u64,
    /// Set by the first turn of [`connect`]: a call queued behind it finds it there, or the
    /// link closed when that `connect` failed.
    i18n: OnceCell<I18n>,
    queue: Queue,
    /// The catalog version shown.
    version: Cell<u64>,
    /// The page shown ([`set_page`]); `index` is the root alone.
    page: RefCell<String>,
    /// [`load`]ed, not shown yet.
    pending: RefCell<Option<String>>,
    /// [`preload`]ed pages: they follow every locale.
    pinned: RefCell<Vec<String>>,
    /// The content tag each `"<locale>/<page>"` chunk in the instance came with.
    held: RefCell<HashMap<String, String>>,
    /// The version each page (`index` too) was last checked at: showing a page whose root
    /// and own chunks were checked at the version shown needs no call.
    fresh: RefCell<HashMap<String, u64>>,
    /// While [`connect`] runs: the instance of the connection before and its tags.
    last: RefCell<Option<(I18n, HashMap<String, String>)>>,
    /// Raw invoke carries the data: the protocol failed once.
    invoke: Cell<bool>,
    closed: Cell<bool>,
    /// The `changed` listener while connected.
    listener: RefCell<Option<Listening>>,
}

type Listener = Closure<dyn FnMut(JsValue)>;

/// A `changed` listener: the closure, its `transformCallback` id and the plugin's event id.
struct Listening {
    closure: Listener,
    handler: JsValue,
    id: JsValue,
}

impl Link {
    fn i18n(&self) -> Result<&I18n, Error> {
        self.i18n.get().ok_or(Error::NotInstalled)
    }
}

/// Misses for `report_missing` (8b.7) of the link `link`: deduped, sent per microtask; the
/// first refusal (a release build has no such command) turns reporting off.
#[derive(Default)]
struct Report {
    link: u64,
    on: bool,
    seen: std::collections::HashSet<(String, String, String)>,
    batch: Vec<(String, String, String)>,
}

/// Why a read stopped: the catalog moved under it (read `meta` again), or an error.
enum Fail {
    Stale,
    Err(Error),
}

impl From<Error> for Fail {
    fn from(e: Error) -> Self {
        Fail::Err(e)
    }
}

/// Connects to the plugin and returns the instance it feeds: `meta` (with the WebView's
/// `navigator.languages`, so a first start picks the user's locale, 8b.3, and its zone),
/// the chunks of the active chain and `page` (`"index"`: the root alone), then the
/// `changed` events. Misses are sent to `report_missing` in debug builds of the app. A
/// second call while connected (or connecting) shows `page` on the same instance, in turn.
/// After a [`disconnect`], chunks the last instance had are taken from it, not read again.
///
/// Install the instance (`i18n_next::install(i18n)`) for `t!` without `=>`; subscribe to
/// it ([`I18n::subscribe`]) to render again after a change.
///
/// # Errors
/// [`Error::Fetch`] when there is no `__TAURI_INTERNALS__` (not in a Tauri WebView), a
/// command fails or a protocol read answers an HTTP error; what [`payload::builder`] and
/// [`payload::apply`] return for a broken answer.
pub async fn connect(page: &str) -> Result<I18n, Error> {
    loop {
        let Some(link) = current() else {
            return start(page).await;
        };
        let _turn = Turn::new(&link).await;
        // the `connect` this one waited for failed, or a disconnect came: start again
        let Some(i18n) = link.i18n.get().filter(|_| !link.closed.get()).cloned() else {
            continue;
        };
        visit(&link, page, true).await?;
        return Ok(i18n);
    }
}

/// The first [`connect`]: the link is there before the first `await` and its first turn
/// lasts until it listens, so calls made meanwhile queue behind it.
async fn start(page: &str) -> Result<I18n, Error> {
    let id = LINKS.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    let link = Rc::new(Link {
        id,
        i18n: OnceCell::new(),
        queue: Queue::default(),
        version: Cell::new(0),
        page: RefCell::new("index".to_owned()),
        pending: RefCell::new(None),
        pinned: RefCell::new(Vec::new()),
        held: RefCell::new(HashMap::new()),
        fresh: RefCell::new(HashMap::new()),
        last: RefCell::new(LAST.with_borrow_mut(Option::take)),
        invoke: Cell::new(false),
        closed: Cell::new(false),
        listener: RefCell::new(None),
    });
    LINK.with_borrow_mut(|l| *l = Some(link.clone()));
    REPORT.with_borrow_mut(|r| {
        *r = Report {
            link: id,
            on: true,
            ..Report::default()
        }
    });
    let _turn = Turn::new(&link).await;
    let opened = open(&link, page).await;
    link.last.take();
    if opened.is_err() {
        link.closed.set(true);
        LINK.with_borrow_mut(|l| {
            if l.as_ref().is_some_and(|l| Rc::ptr_eq(l, &link)) {
                *l = None;
            }
        });
        release(&link);
    } else if link.closed.get() {
        release(&link); // disconnected while it connected
    }
    opened
}

async fn open(link: &Rc<Link>, page: &str) -> Result<I18n, Error> {
    let first = Object::new();
    let global = js_sys::global();
    if let Some(languages) = get(&global, "navigator").and_then(|n| get(&n, "languages"))
        && languages.is_object()
    {
        set(&first, "languages", &Array::from(&languages));
    }
    if let Some(zone) = web_zone() {
        set(&first, "timeZone", &JsValue::from_str(&zone));
    }
    let raw = meta(page, Some(&first)).await?;
    let m = Meta::read(&raw)?;
    let text = std::str::from_utf8(&raw).map_err(|_| Error::Config {
        path: "payload".to_owned(),
        reason: "meta is not UTF-8",
    })?;
    let id = link.id;
    let i18n = m
        .builder(text)
        .missing(move |l, k, p| report(id, l, k, p))
        .build()?;
    let _ = link.i18n.set(i18n.clone());
    show(link, page, true, Some(m)).await?;
    listen(link).await?;
    Ok(i18n)
}

/// Loads the chunks of `page` in the active chain (none when they are loaded and current)
/// and makes it the page that follows locale switches and changes.
///
/// # Errors
/// [`Error::NotInstalled`] before [`connect`]; else as [`connect`].
pub async fn set_page(page: &str) -> Result<(), Error> {
    let link = current().ok_or(Error::NotInstalled)?;
    // nothing queued and the page checked at the version shown: no call
    if !link.queue.busy() && fresh(&link, page) {
        route(&link, page);
        return Ok(());
    }
    let _turn = Turn::new(&link).await;
    visit(&link, page, true).await
}

/// Switches Rust, its native labels and every window to `code` (the plugin's `set_locale`),
/// then loads the chain of the shown page (and the preloaded ones) in it.
///
/// # Errors
/// [`Error::UnknownLocale`] for a code the catalog does not know; else as [`connect`].
pub async fn set_locale(code: &str) -> Result<(), Error> {
    let link = current().ok_or(Error::NotInstalled)?;
    let _turn = Turn::new(&link).await;
    let args = Object::new();
    set(&args, "locale", &JsValue::from_str(code));
    call("set_locale", &args, code).await?;
    let page = link.page.borrow().clone();
    show(&link, &page, true, None).await?;
    follow(&link).await
}

/// Loads a page about to be shown (a router guard, before the navigation is confirmed)
/// without making it the shown page; it follows changes until the next [`set_page`].
///
/// # Errors
/// As [`set_page`].
pub async fn load(page: &str) -> Result<(), Error> {
    let link = current().ok_or(Error::NotInstalled)?;
    let _turn = Turn::new(&link).await;
    *link.pending.borrow_mut() = Some(page.to_owned());
    visit(&link, page, false).await
}

/// Loads a page without showing it; it follows every locale and change from then on.
///
/// # Errors
/// As [`set_page`].
pub async fn preload(page: &str) -> Result<(), Error> {
    let link = current().ok_or(Error::NotInstalled)?;
    let _turn = Turn::new(&link).await;
    {
        let mut pinned = link.pinned.borrow_mut();
        if !pinned.iter().any(|p| p == page) {
            pinned.push(page.to_owned());
        }
    }
    visit(&link, page, false).await
}

/// Reads `meta` again and loads what changed (events do this on their own).
///
/// # Errors
/// As [`set_page`].
pub async fn refresh() -> Result<(), Error> {
    let link = current().ok_or(Error::NotInstalled)?;
    let _turn = Turn::new(&link).await;
    let page = link.page.borrow().clone();
    show(&link, &page, true, None).await
}

/// Stops following `changed` and reporting misses; the instance keeps what it has. A later
/// [`connect`] builds a new one.
pub fn disconnect() {
    let Some(link) = LINK.with_borrow_mut(Option::take) else {
        return;
    };
    link.closed.set(true);
    if let Some(i18n) = link.i18n.get() {
        let held = link.held.take();
        LAST.with_borrow_mut(|l| *l = Some((i18n.clone(), held)));
    }
    release(&link);
    REPORT.with_borrow_mut(|r| *r = Report::default());
}

/// Unlistens as tauri's `_unlisten` does: the JS handler goes first (an event already on its
/// way finds none: the event plugin's `unregisterListener`, else `unregisterCallback`), then
/// the plugin's listener; the closure lives until the plugin answers.
fn release(link: &Link) {
    let Some(l) = link.listener.take() else {
        return;
    };
    let event = JsValue::from_str(CHANGED);
    let global = js_sys::global();
    let unregistered = get(&global, "__TAURI_EVENT_PLUGIN_INTERNALS__")
        .filter(JsValue::is_object)
        .and_then(|i| {
            Some(
                method(&i, "unregisterListener")
                    .ok()?
                    .call2(&i, &event, &l.id),
            )
        })
        .is_some_and(|r| r.is_ok());
    if !unregistered
        && let Ok(tauri) = internals()
        && let Ok(f) = method(&tauri, "unregisterCallback")
    {
        let _ = f.call1(&tauri, &l.handler);
    }
    let args = Object::new();
    set(&args, "event", &event);
    set(&args, "eventId", &l.id);
    if let Ok(unlisten) = invoke("plugin:event|unlisten", &args) {
        spawn_local(async move {
            let _ = unlisten.await;
            drop(l.closure);
        });
    }
}

fn current() -> Option<Rc<Link>> {
    LINK.with_borrow(Clone::clone)
}

/// The page's root and own chunks were checked at the version shown.
fn fresh(link: &Link, page: &str) -> bool {
    let v = link.version.get();
    let fresh = link.fresh.borrow();
    fresh.get("index") == Some(&v) && (page == "index" || fresh.get(page) == Some(&v))
}

fn route(link: &Link, page: &str) {
    *link.page.borrow_mut() = page.to_owned();
    let mut pending = link.pending.borrow_mut();
    if pending.as_deref() == Some(page) {
        *pending = None;
    }
}

/// [`show`] unless the page is fresh; `route` makes it the shown page.
async fn visit(link: &Link, page: &str, route_it: bool) -> Result<(), Error> {
    if !fresh(link, page) {
        return show(link, page, route_it, None).await;
    }
    if route_it {
        route(link, page);
    }
    Ok(())
}

/// Visits the preloaded pages and a `load`ed one in the shown locale.
async fn follow(link: &Link) -> Result<(), Error> {
    let mut pages = link.pinned.borrow().clone();
    pages.extend(link.pending.borrow().clone());
    let shown = link.page.borrow().clone();
    for p in pages.iter().filter(|p| **p != shown) {
        visit(link, p, false).await?;
    }
    Ok(())
}

/// Reads `meta` of the active locale and `page` (`primed`: read already), loads what
/// changed, then shows it. A catalog that moves under the read is read again, three times.
async fn show(
    link: &Link,
    page: &str,
    route_it: bool,
    mut primed: Option<Meta>,
) -> Result<(), Error> {
    // ponytail: 3 retries; a catalog that changes faster than a read is a bug elsewhere
    for _ in 0..4 {
        let m = match primed.take() {
            Some(m) => m,
            None => Meta::read(&meta(page, None).await?)?,
        };
        match step(link, &m, page).await {
            Ok(()) => {
                if route_it {
                    route(link, page);
                }
                return Ok(());
            }
            Err(Fail::Stale) => {}
            Err(Fail::Err(e)) => return Err(e),
        }
    }
    Err(Error::Fetch {
        path: "meta".to_owned(),
        message: "the catalog kept changing: 3 new reads of meta".to_owned(),
    })
}

/// The `meta` of the active locale and `page`; a page the catalog does not know reads the
/// root alone.
async fn meta(page: &str, first: Option<&Object>) -> Result<Vec<u8>, Error> {
    let args = |page: Option<&str>| {
        let args = first.map_or_else(Object::new, |f| Object::assign(&Object::new(), f));
        if let Some(p) = page {
            set(&args, "page", &JsValue::from_str(p));
        }
        args
    };
    let named = (page != "index" && !page.is_empty()).then_some(page);
    let v = match call("meta", &args(named), page).await {
        Err(Error::UnknownPage(_)) if named.is_some() => call("meta", &args(None), page).await,
        r => r,
    }?;
    bytes(&v, "meta")
}

/// Brings the instance to `m`: takes the chunks whose tags it does not hold from the last
/// connection's instance or reads them, clears the ones of the view that have no data now,
/// switches to `m.locale`. A newer catalog shown before or during the reads makes `m` stale.
async fn step(link: &Link, m: &Meta, page: &str) -> Result<(), Fail> {
    let stale = || m.version < link.version.get();
    if stale() {
        return Err(Fail::Stale); // a newer catalog is shown already
    }
    let i18n = link.i18n()?;
    let page = if page.is_empty() { "index" } else { page };
    let (wanted, kept, cleared) = {
        let held = link.held.borrow();
        let last = link.last.borrow();
        let mut kept: Vec<Stored> = Vec::new();
        let mut wanted: Vec<(&str, &str)> = Vec::new();
        for (k, t) in m.tags.iter().filter(|(k, t)| held.get(k) != Some(t)) {
            match last.as_ref().and_then(|(old, tags)| reuse(old, tags, k, t)) {
                Some(chunk) => kept.push(chunk),
                None => wanted.push((k.as_str(), t.as_str())),
            }
        }
        let chain = m
            .chains
            .iter()
            .find(|(c, _)| *c == m.locale)
            .map_or(&[][..], |(_, chain)| &chain[..]);
        let mut cleared = Vec::new();
        for c in chain {
            for p in ["index", page]
                .iter()
                .take(if page == "index" { 1 } else { 2 })
            {
                let key = [c.as_str(), "/", p].concat();
                if held.contains_key(&key) && !m.tags.iter().any(|(k, _)| *k == key) {
                    cleared.push(key);
                }
            }
        }
        (wanted, kept, cleared)
    };
    if !wanted.is_empty() || !kept.is_empty() || !cleared.is_empty() || *i18n.locale() != *m.locale
    {
        let mut bodies = Vec::new();
        if !wanted.is_empty() && !link.invoke.get() {
            match read(&wanted).await {
                Ok(got) => bodies = got,
                Err(Read::Network) => link.invoke.set(true),
                Err(Read::Fail(f)) => return Err(f),
            }
            if stale() {
                return Err(Fail::Stale);
            }
        }
        if !wanted.is_empty() && link.invoke.get() {
            let args = Object::new();
            if page != "index"
                && m.tags
                    .iter()
                    .any(|(k, _)| k.ends_with(&["/", page].concat()))
            {
                set(&args, "page", &JsValue::from_str(page));
            }
            let raw = bytes(&call("load", &args, page).await?, "load")?;
            let got = Meta::read(&raw)?;
            if got.version != m.version || got.locale != m.locale || stale() {
                return Err(Fail::Stale);
            }
            payload::apply(i18n, &raw)?;
        }
        if !bodies.is_empty()
            || !kept.is_empty()
            || !cleared.is_empty()
            || *i18n.locale() != *m.locale
        {
            payload::apply_with(i18n, &wrap(&m.locale, &bodies, &cleared)?, kept)?;
        }
    }
    let mut held = link.held.borrow_mut();
    for k in &cleared {
        held.remove(k);
    }
    for (k, t) in &m.tags {
        held.insert(k.clone(), t.clone());
    }
    drop(held);
    link.version.set(m.version);
    let mut fresh = link.fresh.borrow_mut();
    fresh.insert("index".to_owned(), m.version);
    fresh.insert(page.to_owned(), m.version);
    Ok(())
}

/// The chunk `key` with content tag `tag` from the last connection's instance, when it
/// held that very chunk.
fn reuse(old: &I18n, tags: &HashMap<String, String>, key: &str, tag: &str) -> Option<Stored> {
    if tags.get(key).map(String::as_str) != Some(tag) {
        return None;
    }
    let (l, p) = names(key);
    let page = (p != "index").then(|| p.to_owned());
    let tree: Arc<Tree> = old.ready_chunk(l, page.as_deref())?;
    Some((Arc::from(l), page, tree))
}

/// Why the protocol read failed: `fetch` rejected with a `TypeError` (no protocol: raw
/// invoke from now on), or a failure of this read.
enum Read {
    Network,
    Fail(Fail),
}

/// The bodies of `wanted` (`(key, tag)`) over the protocol, all requests started at once.
async fn read(wanted: &[(&str, &str)]) -> Result<Vec<(String, Vec<u8>)>, Read> {
    let tauri = internals().map_err(|e| Read::Fail(e.into()))?;
    let convert = method(&tauri, "convertFileSrc").map_err(|e| Read::Fail(e.into()))?;
    let global = js_sys::global();
    let fetch = method(&global, "fetch").map_err(|e| Read::Fail(e.into()))?;
    let network = |e: &JsValue| {
        if e.is_instance_of::<js_sys::TypeError>() {
            Read::Network
        } else {
            Read::Fail(Fail::Err(js_error("fetch", e, "")))
        }
    };
    let mut started = Vec::with_capacity(wanted.len());
    for (key, tag) in wanted {
        let path = [tag, "/", key, ".json"].concat();
        let url = convert
            .call2(
                &tauri,
                &JsValue::from_str(&path),
                &JsValue::from_str("i18n-next"),
            )
            .map_err(|e| Read::Fail(js_error("convertFileSrc", &e, "").into()))?;
        let res = fetch.call1(&global, &url).map_err(|e| network(&e))?;
        started.push((key, JsFuture::from(Promise::resolve(&res))));
    }
    let mut out = Vec::with_capacity(started.len());
    for (key, res) in started {
        let res = res.await.map_err(|e| network(&e))?;
        let status = get(&res, "status").and_then(|s| s.as_f64()).unwrap_or(0.0);
        if status == 410.0 {
            return Err(Read::Fail(Fail::Stale));
        }
        // an HTTP error is an answer of the protocol: a failed read, not a missing protocol
        if !(200.0..300.0).contains(&status) {
            return Err(Read::Fail(Fail::Err(Error::Fetch {
                path: [key, ".json"].concat(),
                message: ["HTTP ", &(status as u16).to_string()].concat(),
            })));
        }
        let body = method(&res, "arrayBuffer")
            .and_then(|f| f.call0(&res).map_err(|e| js_error("arrayBuffer", &e, "")))
            .map_err(|e| Read::Fail(e.into()))?;
        let body = JsFuture::from(Promise::resolve(&body))
            .await
            .map_err(|e| Read::Fail(js_error("arrayBuffer", &e, "").into()))?;
        let body = bytes(&body, key).map_err(|e| Read::Fail(e.into()))?;
        out.push(((*key).to_owned(), body));
    }
    Ok(out)
}

/// A payload of protocol chunk bodies (`{"data":…,"linked":…}` by `"<locale>/<page>"`) and
/// empty chunks for `cleared`, switching to `locale`: one [`payload::apply`], one snapshot.
fn wrap(locale: &str, bodies: &[(String, Vec<u8>)], cleared: &[String]) -> Result<Vec<u8>, Error> {
    let len: usize = bodies.iter().map(|(k, b)| k.len() + b.len() + 32).sum();
    let mut out = String::with_capacity(64 + len + cleared.len() * 48);
    out.push_str("{\"locale\":");
    write_str(&mut out, locale);
    out.push_str(",\"chunks\":[");
    let mut first = true;
    for (key, body) in bodies {
        let body = std::str::from_utf8(body)
            .ok()
            .and_then(|b| b.trim_start().strip_prefix('{'))
            .ok_or_else(|| Error::NotObject {
                path: ["payload:", key].concat(),
            })?;
        if !first {
            out.push(',');
        }
        first = false;
        let (l, p) = names(key);
        out.push_str("{\"locale\":");
        write_str(&mut out, l);
        out.push_str(",\"page\":");
        write_str(&mut out, p);
        out.push(',');
        out.push_str(body);
    }
    for key in cleared {
        if !first {
            out.push(',');
        }
        first = false;
        let (l, p) = names(key);
        out.push_str("{\"locale\":");
        write_str(&mut out, l);
        out.push_str(",\"page\":");
        write_str(&mut out, p);
        out.push_str(",\"data\":{}}");
    }
    out.push_str("]}");
    Ok(out.into_bytes())
}

/// `"<locale>/<page>"` split at its first `/` (a locale has none, a page may).
fn names(key: &str) -> (&str, &str) {
    key.split_once('/').unwrap_or((key, "index"))
}

/// Follows `changed`: an event of a version shown already is late and dropped; the others
/// queue a [`show`] of the shown page in turn, taken when the event arrives.
async fn listen(link: &Rc<Link>) -> Result<(), Error> {
    let weak = Rc::downgrade(link);
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let Some(link) = weak.upgrade() else { return };
        let v = get(&e, "payload")
            .and_then(|p| get(&p, "version"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u64;
        if link.closed.get() || v <= link.version.get() {
            return;
        }
        let turn = Turn::new(&link);
        spawn_local(async move {
            let _turn = turn.await;
            if link.closed.get() || v <= link.version.get() {
                return;
            }
            let page = link.page.borrow().clone();
            // nobody awaits an event: a failed read leaves the shown state, the next
            // event or call reads again
            if show(&link, &page, true, None).await.is_ok() {
                let _ = follow(&link).await;
            }
        });
    });
    let tauri = internals()?;
    let handler = method(&tauri, "transformCallback")?
        .call1(&tauri, closure.as_ref())
        .map_err(|e| js_error("transformCallback", &e, ""))?;
    let args = Object::new();
    set(&args, "event", &JsValue::from_str(CHANGED));
    let target = Object::new();
    set(&target, "kind", &JsValue::from_str("Any"));
    set(&args, "target", &target);
    set(&args, "handler", &handler);
    let id = invoke("plugin:event|listen", &args)?
        .await
        .map_err(|e| js_error("plugin:event|listen", &e, ""))?;
    *link.listener.borrow_mut() = Some(Listening {
        closure,
        handler,
        id,
    });
    Ok(())
}

/// The `missing` handler of the instance of link `link`: misses while chunks are on their
/// way are not misses, nor are those of an instance whose connection is gone; the rest go to
/// `report_missing` once each, in a batch per microtask.
fn report(link: u64, locale: &str, key: &str, page: &str) {
    let busy = LINK
        .try_with(|l| {
            l.try_borrow().ok().and_then(|l| {
                l.as_ref()
                    .filter(|l| l.id == link)
                    .map(|l| l.queue.busy() || l.closed.get())
            })
        })
        .ok()
        .flatten();
    if busy != Some(false) {
        return;
    }
    let schedule = REPORT.with_borrow_mut(|r| {
        let id = (locale.to_owned(), page.to_owned(), key.to_owned());
        if !r.on || r.link != link || !r.seen.insert(id.clone()) {
            return false;
        }
        r.batch.push(id);
        r.batch.len() == 1
    });
    if schedule {
        spawn_local(async move {
            let batch = REPORT.with_borrow_mut(|r| std::mem::take(&mut r.batch));
            let keys = Array::new();
            for (locale, page, key) in batch {
                let k = Object::new();
                set(&k, "locale", &JsValue::from_str(&locale));
                set(&k, "page", &JsValue::from_str(&page));
                set(&k, "key", &JsValue::from_str(&key));
                keys.push(&k);
            }
            let args = Object::new();
            set(&args, "keys", &keys);
            if call("report_missing", &args, "").await.is_err() {
                // a release build: the command is not registered
                REPORT.with_borrow_mut(|r| {
                    if r.link == link {
                        r.on = false;
                    }
                });
            }
        });
    }
}

/// `__TAURI_INTERNALS__.invoke(cmd, args)`, started.
fn invoke(cmd: &str, args: &Object) -> Result<JsFuture, Error> {
    let tauri = internals()?;
    let p = method(&tauri, "invoke")?
        .call2(&tauri, &JsValue::from_str(cmd), args)
        .map_err(|e| js_error(cmd, &e, ""))?;
    Ok(JsFuture::from(Promise::resolve(&p)))
}

/// A command of the plugin; `name` (the locale or page passed) names what the plugin
/// refused in [`Error::UnknownLocale`] / [`Error::UnknownPage`].
async fn call(cmd: &str, args: &Object, name: &str) -> Result<JsValue, Error> {
    let full = ["plugin:i18n-next|", cmd].concat();
    invoke(&full, args)?
        .await
        .map_err(|e| js_error(&full, &e, name))
}

fn internals() -> Result<JsValue, Error> {
    get(&js_sys::global(), "__TAURI_INTERNALS__")
        .filter(JsValue::is_object)
        .ok_or_else(|| Error::Fetch {
            path: "__TAURI_INTERNALS__".to_owned(),
            message: "not in a Tauri WebView".to_owned(),
        })
}

fn method(o: &JsValue, name: &str) -> Result<Function, Error> {
    get(o, name)
        .and_then(|f| f.dyn_into().ok())
        .ok_or_else(|| Error::Fetch {
            path: name.to_owned(),
            message: "not a function".to_owned(),
        })
}

fn get(o: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(o, &JsValue::from_str(key)).ok()
}

fn set(o: &Object, key: &str, v: &JsValue) {
    let _ = Reflect::set(o, &JsValue::from_str(key), v);
}

/// The bytes of a raw answer of `what`: an `ArrayBuffer`, a view of one (a typed array, a
/// `DataView`), an array of numbers or a string of JSON; anything else is an error.
fn bytes(v: &JsValue, what: &str) -> Result<Vec<u8>, Error> {
    if let Some(s) = v.as_string() {
        return Ok(s.into_bytes());
    }
    if ArrayBuffer::is_view(v) {
        let n = |k| get(v, k).and_then(|n| n.as_f64()).unwrap_or(0.0) as u32;
        let buffer = get(v, "buffer").unwrap_or(JsValue::UNDEFINED);
        if buffer.is_instance_of::<ArrayBuffer>() {
            let view = Uint8Array::new_with_byte_offset_and_length(
                &buffer,
                n("byteOffset"),
                n("byteLength"),
            );
            return Ok(view.to_vec());
        }
    } else if v.is_instance_of::<ArrayBuffer>() || Array::is_array(v) {
        return Ok(Uint8Array::new(v).to_vec());
    }
    Err(Error::Fetch {
        path: what.to_owned(),
        message: "the answer is not bytes or text".to_owned(),
    })
}

/// A rejection as an [`Error`]: the plugin's `{kind, message}`, else its text.
fn js_error(path: &str, e: &JsValue, name: &str) -> Error {
    match get(e, "kind").and_then(|k| k.as_string()).as_deref() {
        Some("UnknownPage") => Error::UnknownPage(name.to_owned()),
        Some("UnknownLocale") => Error::UnknownLocale(name.to_owned()),
        _ => Error::Fetch {
            path: path.to_owned(),
            message: e
                .as_string()
                .or_else(|| get(e, "message").and_then(|m| m.as_string()))
                .unwrap_or_else(|| "rejected".to_owned()),
        },
    }
}

/// The WebView's IANA zone (`Intl.DateTimeFormat().resolvedOptions().timeZone`).
fn web_zone() -> Option<String> {
    let ctor: Function = get(&get(&js_sys::global(), "Intl")?, "DateTimeFormat")?
        .dyn_into()
        .ok()?;
    let fmt = Reflect::construct(&ctor, &Array::new()).ok()?;
    let opts = method(&fmt, "resolvedOptions").ok()?.call0(&fmt).ok()?;
    get(&opts, "timeZone")?.as_string()
}

/// Calls and events in order: each takes a ticket and runs when the ones before it are
/// done (or dropped before their turn).
#[derive(Default)]
struct Queue {
    next: Cell<u64>,
    serving: Cell<u64>,
    /// Tickets dropped before their turn: skipped.
    gone: RefCell<Vec<u64>>,
    wakers: RefCell<Vec<Waker>>,
}

impl Queue {
    fn busy(&self) -> bool {
        self.next.get() != self.serving.get()
    }

    fn advance(&self) {
        let mut s = self.serving.get() + 1;
        {
            let mut gone = self.gone.borrow_mut();
            while let Some(i) = gone.iter().position(|g| *g == s) {
                gone.swap_remove(i);
                s += 1;
            }
        }
        self.serving.set(s);
        for w in self.wakers.take() {
            w.wake();
        }
    }
}

/// A place in the [`Queue`], taken when it is made; resolves to its turn.
struct Turn {
    link: Rc<Link>,
    ticket: u64,
    done: bool,
}

impl Turn {
    fn new(link: &Rc<Link>) -> Turn {
        let ticket = link.queue.next.get();
        link.queue.next.set(ticket + 1);
        Turn {
            link: link.clone(),
            ticket,
            done: false,
        }
    }
}

impl Future for Turn {
    type Output = Held;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Held> {
        if self.link.queue.serving.get() == self.ticket {
            self.done = true;
            return Poll::Ready(Held(self.link.clone()));
        }
        self.link.queue.wakers.borrow_mut().push(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let q = &self.link.queue;
        if q.serving.get() == self.ticket {
            q.advance();
        } else {
            q.gone.borrow_mut().push(self.ticket);
        }
    }
}

/// The turn being run: the next one starts when it is dropped.
struct Held(Rc<Link>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.queue.advance();
    }
}
