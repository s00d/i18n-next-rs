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

use crate::error::Error;
use crate::i18n::I18n;
use crate::json::write_str;
use crate::payload::{self, Meta};
use js_sys::futures::{JsFuture, spawn_local};
use js_sys::wasm_bindgen::closure::Closure;
use js_sys::wasm_bindgen::{JsCast, JsValue};
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// The event every window gets after a change (DESIGN 6.1).
const CHANGED: &str = "i18n-next://changed";

thread_local! {
    static LINK: RefCell<Option<Rc<Link>>> = const { RefCell::new(None) };
    static REPORT: RefCell<Report> = RefCell::new(Report { on: true, ..Report::default() });
}

/// The connection: one per WebView.
struct Link {
    i18n: I18n,
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
    /// Raw invoke carries the data: the protocol failed once.
    invoke: Cell<bool>,
    closed: Cell<bool>,
    /// The `changed` listener: kept alive while connected, and its event id.
    listener: RefCell<Option<(Listener, JsValue)>>,
}

type Listener = Closure<dyn FnMut(JsValue)>;

/// Misses for `report_missing` (8b.7): deduped, sent per microtask; the first refusal (a
/// release build has no such command) turns reporting off.
#[derive(Default)]
struct Report {
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
/// second call while connected shows `page` on the same instance.
///
/// Install the instance (`i18n_next::install(i18n)`) for `t!` without `=>`; subscribe to
/// it ([`I18n::subscribe`]) to render again after a change.
///
/// # Errors
/// [`Error::Fetch`] when there is no `__TAURI_INTERNALS__` (not in a Tauri WebView), a
/// command fails or a protocol read answers an HTTP error; what [`payload::builder`] and
/// [`payload::apply`] return for a broken answer.
pub async fn connect(page: &str) -> Result<I18n, Error> {
    if let Some(link) = current() {
        visit(&link, page, true).await?;
        return Ok(link.i18n.clone());
    }
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
    let i18n = payload::builder(&raw)?.missing(report).build()?;
    let link = Rc::new(Link {
        i18n: i18n.clone(),
        queue: Queue::default(),
        version: Cell::new(0),
        page: RefCell::new("index".to_owned()),
        pending: RefCell::new(None),
        pinned: RefCell::new(Vec::new()),
        held: RefCell::new(HashMap::new()),
        fresh: RefCell::new(HashMap::new()),
        invoke: Cell::new(false),
        closed: Cell::new(false),
        listener: RefCell::new(None),
    });
    {
        let _turn = Turn::new(&link).await;
        show(&link, page, true, Some(raw)).await?;
    }
    listen(&link).await?;
    REPORT.with_borrow_mut(|r| {
        *r = Report {
            on: true,
            ..Report::default()
        }
    });
    LINK.with_borrow_mut(|l| *l = Some(link));
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
    if let Some((_closure, id)) = link.listener.borrow_mut().take() {
        let args = Object::new();
        set(&args, "event", &JsValue::from_str(CHANGED));
        set(&args, "eventId", &id);
        // nothing to wait for: the handler is gone with the closure either way
        let _ = invoke("plugin:event|unlisten", &args);
    }
    REPORT.with_borrow_mut(|r| *r = Report::default());
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

/// Reads `meta` of the active locale and `page` (`primed`: one read already), loads what
/// changed, then shows it. A catalog that moves under the read is read again, three times.
async fn show(
    link: &Link,
    page: &str,
    route_it: bool,
    mut primed: Option<Vec<u8>>,
) -> Result<(), Error> {
    // ponytail: 3 retries; a catalog that changes faster than a read is a bug elsewhere
    for _ in 0..4 {
        let raw = match primed.take() {
            Some(raw) => raw,
            None => meta(page, None).await?,
        };
        let m = Meta::read(&raw)?;
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
    match call("meta", &args(named), page).await {
        Err(Error::UnknownPage(_)) if named.is_some() => call("meta", &args(None), page).await,
        r => r,
    }
    .map(|v| bytes(&v))
}

/// Brings the instance to `m`: reads the chunks whose tags it does not hold, clears the
/// ones of the view that have no data now, switches to `m.locale`.
async fn step(link: &Link, m: &Meta, page: &str) -> Result<(), Fail> {
    if m.version < link.version.get() {
        return Err(Fail::Stale); // a newer catalog is shown already
    }
    let page = if page.is_empty() { "index" } else { page };
    let (wanted, cleared) = {
        let held = link.held.borrow();
        let wanted: Vec<(&str, &str)> = m
            .tags
            .iter()
            .filter(|(k, t)| held.get(k) != Some(t))
            .map(|(k, t)| (k.as_str(), t.as_str()))
            .collect();
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
        (wanted, cleared)
    };
    if !wanted.is_empty() || !cleared.is_empty() || *link.i18n.locale() != *m.locale {
        let mut bodies = Vec::new();
        if !wanted.is_empty() && !link.invoke.get() {
            match read(&wanted).await {
                Ok(got) => bodies = got,
                Err(Read::Network) => link.invoke.set(true),
                Err(Read::Fail(f)) => return Err(f),
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
            let raw = bytes(&call("load", &args, page).await?);
            let got = Meta::read(&raw)?;
            if got.version != m.version || got.locale != m.locale {
                return Err(Fail::Stale);
            }
            payload::apply(&link.i18n, &raw)?;
        }
        if !bodies.is_empty() || !cleared.is_empty() || *link.i18n.locale() != *m.locale {
            payload::apply(&link.i18n, &wrap(&m.locale, &bodies, &cleared)?)?;
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
        out.push(((*key).to_owned(), bytes(&body)));
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
    *link.listener.borrow_mut() = Some((closure, id));
    Ok(())
}

/// The `missing` handler of the instance: misses while chunks are on their way are not
/// misses; the rest go to `report_missing` once each, in a batch per microtask.
fn report(locale: &str, key: &str, page: &str) {
    let busy = LINK
        .try_with(|l| {
            l.try_borrow()
                .ok()
                .and_then(|l| l.as_ref().map(|l| l.queue.busy() || l.closed.get()))
        })
        .ok()
        .flatten();
    if busy != Some(false) {
        return;
    }
    let schedule = REPORT.with_borrow_mut(|r| {
        let id = (locale.to_owned(), page.to_owned(), key.to_owned());
        if !r.on || !r.seen.insert(id.clone()) {
            return false;
        }
        r.batch.push(id);
        r.batch.len() == 1
    });
    if schedule {
        spawn_local(async {
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
                REPORT.with_borrow_mut(|r| r.on = false);
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

/// The bytes of a raw answer: an `ArrayBuffer`, a typed array or an array of numbers, or a
/// string of JSON.
fn bytes(v: &JsValue) -> Vec<u8> {
    match v.as_string() {
        Some(s) => s.into_bytes(),
        None => Uint8Array::new(v).to_vec(),
    }
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
