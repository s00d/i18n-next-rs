//! `I18n`, its builder and views (DESIGN 3, 4.1, 4.2, 4.5).
//!
//! Reads go through a snapshot: `Inner.version` is one atomic load, and each thread keeps
//! up to four `(instance, version, Arc<Snapshot>)` in a `thread_local!` cache (spike S1),
//! so a hit takes no lock and touches no reference count. A write builds a new snapshot,
//! swaps it under the write lock and bumps the version; a reader holding the old `Arc`
//! never sees a change. A locale switch shares the data and chain maps of the previous
//! snapshot (`Arc` clones): only the active pointer changes.

use crate::args::{Args, Count};
use crate::catalog::{Entry, Msg, Tree, WithCount, interpolate_once};
use crate::change::{Change, Listeners};
use crate::check::{self, CheckReport};
use crate::error::Error;
use crate::hash::FastMap;
use crate::lazy::{Layer, LazyCtl};
use crate::loader::{self, Config};
use crate::plural::{self, Plural};
use crate::source::{self, Pair, Sources};
use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

type Missing = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

/// One locale's chunks: the root (`index`) and its pages, ready or lazy.
#[derive(Debug, Default, Clone)]
pub(crate) struct LocaleData {
    pub(crate) root: Option<Layer>,
    pub(crate) pages: FastMap<Arc<str>, Layer>,
}

impl LocaleData {
    /// Page layer, then root layer.
    fn get(&self, page: Option<&str>, key: &str) -> Option<&Entry> {
        if let Some(p) = page
            && let Some(e) = self
                .pages
                .get(p)
                .and_then(Layer::chunk)
                .and_then(|c| c.get(key))
        {
            return Some(e);
        }
        self.root
            .as_ref()
            .and_then(Layer::chunk)
            .and_then(|c| c.get(key))
    }

    /// The root (`None`) or page layer, as stored.
    pub(crate) fn layer(&self, page: Option<&str>) -> Option<&Layer> {
        match page {
            None => self.root.as_ref(),
            Some(p) => self.pages.get(p),
        }
    }

    /// What `has` reads: page then root, own keys only (no texts `linked.rs` stored).
    fn has(&self, page: Option<&str>, key: &str) -> bool {
        let has = |l: Option<&Layer>| l.and_then(Layer::chunk).is_some_and(|c| c.has(key));
        has(page.and_then(|p| self.pages.get(p))) || has(self.root.as_ref())
    }
}

type Chain = Arc<[Arc<LocaleData>]>;

/// What the builder fixed: it does not change after `build`.
#[derive(Debug, Default)]
pub(crate) struct Meta {
    fallback: Option<Arc<str>>,
    links: FastMap<Arc<str>, Arc<str>>,
    iso: FastMap<Arc<str>, Arc<str>>,
    /// `i18n.json` `locales[]`, in file order.
    configured: Vec<loader::LocaleEntry>,
    /// Locales the app named itself: the start locale, `defaultLocale`, both ends of every
    /// fallback link, the global fallback and `iso` codes. Each is known, with its chain.
    declared: Vec<Arc<str>>,
    aliases: FastMap<Arc<str>, Arc<str>>,
    /// Raw `numberFormats` / `datetimeFormats` of `i18n.json`, for M5 / M6.
    #[allow(dead_code, reason = "read by the formats of M5 / M6")]
    formats: (Option<Box<str>>, Option<Box<str>>),
    /// What `check()` reads besides the chunks (M3).
    check: check::Facts,
    /// `linked_messages(true)`: `build` and `reload` resolve links with it.
    linked: Option<ResolveLinked>,
}

impl Meta {
    /// `linked_messages(true)`.
    pub(crate) fn linked(&self) -> bool {
        self.linked.is_some()
    }

    fn disabled(&self, code: &str) -> bool {
        self.configured.iter().any(|l| l.disabled && l.code == code)
    }

    /// The page chunk a name reads: `""` and `index` are the root, an alias its target.
    pub(crate) fn page<'a>(&'a self, page: Option<&'a str>) -> Option<&'a str> {
        let p = page.filter(|p| !p.is_empty() && *p != "index")?;
        let p = self.aliases.get(p).map_or(p, |t| &**t);
        (p != "index").then_some(p)
    }

    /// `code` and its `fallback_for` links to their end, then the global fallback alone,
    /// never expanded through its own links (`utils` `buildFallbackLocaleChain`). A cycle
    /// (`a → b → a`) stops at the first repeat.
    fn chain(&self, data: &FastMap<Arc<str>, Arc<LocaleData>>, code: &str) -> Chain {
        let mut codes: Vec<&str> = Vec::new();
        let mut next = Some(code);
        while let Some(c) = next {
            if codes.contains(&c) {
                break;
            }
            codes.push(c);
            next = self.links.get(c).map(|s| &**s);
        }
        if let Some(fb) = self.fallback.as_deref()
            && !codes.contains(&fb)
        {
            codes.push(fb);
        }
        codes.iter().filter_map(|c| data.get(*c).cloned()).collect()
    }
}

pub(crate) type DataMap = FastMap<Arc<str>, Arc<LocaleData>>;

pub(crate) struct Snapshot {
    pub(crate) version: u64,
    pub(crate) locale: Arc<str>,
    /// Shared between snapshots that differ only in the active locale.
    pub(crate) data: Arc<DataMap>,
    /// Chains of every known locale, disabled ones too (`with_locale` reads them, as a
    /// configured locale in `buildFallbackLocaleChain`); `set_locale` refuses the disabled.
    chains: Arc<FastMap<Arc<str>, Chain>>,
    active: Chain,
    /// The active locale's own chunks (what `has` reads), `None` when it has no data.
    own: Option<Arc<LocaleData>>,
    /// The global fallback's layer alone, never its links: what a locale that is not known
    /// reads (`buildFallbackLocaleChain` of an unconfigured code is `[fallback]`).
    fallback: Chain,
    pub(crate) meta: Arc<Meta>,
    /// Plural rules of the active locale (its `iso`, else its code); `None`: index rule.
    pub(crate) plural: Option<&'static Plural>,
    /// The same for every known locale, resolved once: a view in another locale reads
    /// them with one hash lookup instead of resolving its tag on every `tc` (~130 ns).
    plurals: Arc<FastMap<Arc<str>, Option<&'static Plural>>>,
}

impl Meta {
    /// Plural rules of `code`: by its `iso` when it has one (`ua` → `uk-UA`), else the code.
    fn plural(&self, code: &str) -> Option<&'static Plural> {
        plural::for_tag(self.iso.get(code).map_or(code, |t| &**t))
    }
}

impl Snapshot {
    pub(crate) fn new(meta: Arc<Meta>, data: DataMap, locale: Arc<str>, version: u64) -> Snapshot {
        let mut chains = FastMap::default();
        let names = meta
            .configured
            .iter()
            .map(|l| &*l.code)
            .chain(data.keys().map(|k| &**k))
            .chain(meta.declared.iter().map(|k| &**k))
            .chain(std::iter::once(&*locale));
        let mut plurals = FastMap::default();
        for code in names {
            if !chains.contains_key(code) {
                let code: Arc<str> = Arc::from(code);
                let chain = meta.chain(&data, &code);
                plurals.insert(code.clone(), meta.plural(&code));
                chains.insert(code, chain);
            }
        }
        let active = chains
            .get(&*locale)
            .cloned()
            .unwrap_or_else(|| meta.chain(&data, &locale));
        let fallback = meta
            .fallback
            .as_deref()
            .and_then(|fb| data.get(fb).cloned())
            .into_iter()
            .collect();
        Snapshot {
            version,
            own: data.get(&*locale).cloned(),
            plural: meta.plural(&locale),
            locale,
            data: Arc::new(data),
            chains: Arc::new(chains),
            plurals: Arc::new(plurals),
            active,
            fallback,
            meta,
        }
    }

    /// Plural rules of `code` (a view's locale): known ones from the table, others resolved.
    pub(crate) fn plural_of(&self, code: &str) -> Option<&'static Plural> {
        match self.plurals.get(code) {
            Some(p) => *p,
            None => self.meta.plural(code),
        }
    }

    /// The layers of `locale` (the active one when `None`). A locale that is not known has
    /// nothing of its own: it reads the global fallback alone.
    pub(crate) fn layers(&self, locale: Option<&str>) -> &[Arc<LocaleData>] {
        match locale {
            None => &self.active,
            Some(code) => self.chains.get(code).unwrap_or(&self.fallback),
        }
    }

    pub(crate) fn lookup(
        &self,
        locale: Option<&str>,
        page: Option<&str>,
        key: &str,
    ) -> Option<&Entry> {
        let page = self.meta.page(page);
        self.layers(locale)
            .iter()
            .find_map(|layer| layer.get(page, key))
    }

    /// Only the locale itself, page then root: no fallback (JS `has`).
    fn has(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> bool {
        let own = match locale {
            None => self.own.as_deref(),
            Some(code) if self.chains.contains_key(code) => self.data.get(code).map(|d| &**d),
            Some(_) => None,
        };
        own.is_some_and(|d| d.has(self.meta.page(page), key))
    }
}

/// One cached snapshot of this thread.
struct Slot {
    id: u64,
    version: u64,
    snap: Arc<Snapshot>,
    /// Cleared when the instance is dropped.
    alive: Arc<AtomicBool>,
}

/// Cached snapshots of this thread, the slot to evict next, and the [`DROPS`] value the
/// slots were last checked against.
struct Cache {
    slots: [Option<Slot>; 4],
    next: usize,
    drops: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = const {
        RefCell::new(Cache { slots: [const { None }; 4], next: 0, drops: 0 })
    };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Bumped by every dropped instance: the next read on a thread that sees a new value drops
/// the cached snapshots of dead instances.
static DROPS: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Inner {
    id: u64,
    version: AtomicU64,
    state: RwLock<Arc<Snapshot>>,
    /// Serializes writers; readers never take it.
    writer: Mutex<()>,
    missing: Option<Missing>,
    alive: Arc<AtomicBool>,
    /// What `build` read: `reload` reads it again.
    pub(crate) sources: Sources,
    /// Lazy loads, eviction and remote fetches of this instance.
    pub(crate) ctl: Arc<LazyCtl>,
    /// `remote` manifest path → its pair (`add_file` accepts only these).
    pub(crate) remote: FastMap<Box<str>, Pair>,
    pub(crate) listeners: Listeners,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        // Release: a reader that sees the new count sees `alive` cleared.
        DROPS.fetch_add(1, Ordering::Release);
    }
}

impl Inner {
    /// The current snapshot (one read lock).
    pub(crate) fn current(&self) -> Arc<Snapshot> {
        self.load()
    }

    fn load(&self) -> Arc<Snapshot> {
        match self.state.read() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// `f` on the current snapshot: the thread cache when it is fresh, else one read lock.
    /// A hit is two atomic loads and a scan of four slots, no reference count.
    pub(crate) fn read<R>(&self, mut f: impl FnMut(&Snapshot) -> R) -> R {
        let version = self.version.load(Ordering::Acquire);
        // Relaxed: only compared here (a plain load on ARM); `refresh` acquires it.
        let drops = DROPS.load(Ordering::Relaxed);
        let hit = CACHE.try_with(|cell| {
            let cache = cell.try_borrow().ok()?;
            if cache.drops != drops {
                return None;
            }
            let slot = cache
                .slots
                .iter()
                .flatten()
                .find(|s| s.id == self.id && s.version == version)?;
            Some(f(&slot.snap))
        });
        if let Ok(Some(r)) = hit {
            return r;
        }
        let (snap, gone) = self.refresh();
        let r = f(&snap);
        // Slots that left the cache are dropped after `f`: freeing an old catalog does
        // not delay the read.
        drop(gone);
        r
    }

    /// The miss path of [`Inner::read`], not generic: loads the snapshot under the read
    /// lock, caches it, and hands back the slots that left the cache (a stale version of
    /// this instance, an evicted one, those of dropped instances).
    #[inline(never)]
    fn refresh(&self) -> (Arc<Snapshot>, [Option<Slot>; 4]) {
        // Acquire: pairs with the release in `Inner::drop`, so `alive` reads below see it.
        let drops = DROPS.load(Ordering::Acquire);
        let snap = self.load();
        let mut gone: [Option<Slot>; 4] = [const { None }; 4];
        // Thread-local storage can be gone (a thread-exit destructor calls in): no caching.
        let _ = CACHE.try_with(|cell| {
            let Ok(mut cache) = cell.try_borrow_mut() else {
                return;
            };
            let cache = &mut *cache;
            if cache.drops != drops {
                for (slot, out) in cache.slots.iter_mut().zip(&mut gone) {
                    if slot
                        .as_ref()
                        .is_some_and(|s| !s.alive.load(Ordering::Relaxed))
                    {
                        *out = slot.take();
                    }
                }
                cache.drops = drops;
            }
            let slots = &mut cache.slots;
            let at = slots
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| s.id == self.id))
                .or_else(|| slots.iter().position(Option::is_none))
                .unwrap_or_else(|| {
                    let at = cache.next;
                    cache.next = (at + 1) % slots.len();
                    at
                });
            let new = Slot {
                id: self.id,
                version: snap.version,
                snap: snap.clone(),
                alive: self.alive.clone(),
            };
            if let Some(old) = slots[at].replace(new) {
                gone[at] = Some(old);
            }
        });
        (snap, gone)
    }

    /// Builds the next snapshot from the current one and publishes it (`None`: no change);
    /// what changed when something did.
    pub(crate) fn write(
        &self,
        f: impl FnOnce(&Snapshot) -> Result<Option<Snapshot>, Error>,
    ) -> Result<Option<Change>, Error> {
        let _writer = match self.writer.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let current = self.load();
        Ok(f(&current)?.map(|next| {
            let locale = next.locale.clone();
            let version = self.publish(next);
            Change { locale, version }
        }))
    }

    fn publish(&self, next: Snapshot) -> u64 {
        let version = next.version;
        let mut state = match self.state.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *state = Arc::new(next);
        self.version.store(version, Ordering::Release);
        version
    }

    /// Evicts the oldest loaded pages past the limit (8b.16), called after a lazy load.
    /// Skipped when a writer is busy, this thread included: a later load evicts then.
    pub(crate) fn try_evict(&self) {
        let Some(limit) = self.ctl.limit else {
            return;
        };
        let _writer = match self.writer.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        let current = self.load();
        if let Some(data) = crate::evict::plan(&current.data, limit, &self.ctl) {
            let next = Snapshot::new(
                current.meta.clone(),
                data,
                current.locale.clone(),
                current.version + 1,
            );
            self.publish(next);
        }
    }
}

/// What a lookup found, carried out of the snapshot closure.
enum Found {
    Text(String),
    /// A subtree: the key comes back, `missing` is not called.
    Node,
    /// Nothing in any layer: the locale to report.
    Miss(Option<Arc<str>>),
}

/// The translation catalog: `Arc` inside, cheap to clone, `Send + Sync + 'static`.
///
/// Each thread caches up to four snapshots (one per instance it read last), so a read
/// takes no lock. When the last handle is dropped, the next read on every other thread
/// frees that thread's cached snapshot of it; a thread that never reads again keeps at
/// most four snapshots until it reads or exits.
///
/// ```
/// use i18n_next::I18n;
///
/// let i18n = I18n::builder()
///     .locale("de")
///     .fallback("en")
///     .messages("en", None, r#"{"nav": {"home": "Home", "about": "About"}}"#)
///     .messages("de", None, r#"{"nav": {"home": "Startseite"}}"#)
///     .messages("de", Some("settings"), r#"{"title": "Einstellungen"}"#)
///     .build()?;
/// assert_eq!(i18n.t("nav.home"), "Startseite");
/// assert_eq!(i18n.t("nav.about"), "About");          // from the fallback
/// assert_eq!(i18n.page("settings").t("title"), "Einstellungen");
/// assert_eq!(i18n.try_t("nav.nope"), None);
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Clone)]
pub struct I18n {
    pub(crate) inner: Arc<Inner>,
}

/// Public facts about one locale: from `i18n.json`, builder calls and the files found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocaleInfo {
    /// The locale code, as in file names (`de-at`).
    pub code: String,
    /// The BCP 47 tag for plurals and formats (`de-AT`), when set.
    pub iso: Option<String>,
    /// Text direction for the frontend (`rtl`), when set.
    pub dir: Option<String>,
    /// `displayName` from `i18n.json`, when set.
    pub display_name: Option<String>,
}

impl fmt::Debug for I18n {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snap = self.inner.load();
        f.debug_struct("I18n")
            .field("locale", &snap.locale)
            .field("version", &snap.version)
            .field("loads", &self.inner.ctl.loads.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// Validates a name passed in code for a chunk: not empty, not `__proto__`.
pub(crate) fn chunk_names(locale: &str, page: Option<&str>) -> Result<(), Error> {
    if locale.is_empty() || locale == "__proto__" {
        return Err(Error::UnknownLocale(locale.to_owned()));
    }
    match page {
        Some("__proto__") => Err(Error::UnknownPage("__proto__".to_owned())),
        _ => Ok(()),
    }
}

/// `messages:<locale>/<page>`: the path of text passed in code, in errors.
pub(crate) fn messages_path(locale: &str, page: Option<&str>) -> String {
    let page = page.filter(|p| !p.is_empty()).unwrap_or("index");
    let mut path = String::with_capacity(10 + locale.len() + page.len());
    path.push_str("messages:");
    path.push_str(locale);
    path.push('/');
    path.push_str(page);
    path
}

impl I18n {
    /// Starts a builder.
    #[must_use]
    pub fn builder() -> Builder {
        Builder::default()
    }

    fn resolve(
        &self,
        locale: Option<&str>,
        page: Option<&str>,
        key: &str,
        args: Option<&Args<'_>>,
    ) -> String {
        if key.is_empty() {
            return String::new();
        }
        let report = self.inner.missing.is_some() || crate::trace::MISSES;
        let found = self.inner.read(|s| match s.lookup(locale, page, key) {
            Some(Entry::Msg(Msg::Text(t))) => Found::Text(match args {
                Some(a) => t.interpolate(a),
                None => String::from(&*t.raw),
            }),
            // `t` reads the whole string, pipes and all (JS `t`)
            Some(Entry::Msg(Msg::Plural(p))) => Found::Text(match args {
                Some(a) => p.text.interpolate(a),
                None => String::from(&*p.text.raw),
            }),
            Some(Entry::Value(v)) => Found::Text(String::from(&**v)),
            Some(Entry::Node) => Found::Node,
            None => Found::Miss((report && locale.is_none()).then(|| s.locale.clone())),
        });
        match found {
            Found::Text(s) => s,
            Found::Node => key.to_owned(),
            Found::Miss(active) => {
                self.report(locale.or(active.as_deref()), page, key);
                match args.and_then(|a| a.default_text().map(|d| (a, d))) {
                    Some((a, d)) => interpolate_once(d, a),
                    None => key.to_owned(),
                }
            }
        }
    }

    /// A miss: the `missing` handler and, with `tracing`, a `warn!`.
    #[cold]
    pub(crate) fn report(&self, locale: Option<&str>, page: Option<&str>, key: &str) {
        let code = locale.unwrap_or("");
        let page = page.filter(|p| !p.is_empty()).unwrap_or("index");
        if let Some(missing) = &self.inner.missing {
            missing(code, key, page);
        }
        crate::trace::missing(code, page, key);
    }

    /// `tc` of `core/src/base.ts` with the default plural rule (`defaultPlural`).
    fn resolve_count(
        &self,
        locale: Option<&str>,
        page: Option<&str>,
        key: &str,
        count: Count,
        args: Option<&Args<'_>>,
    ) -> String {
        let report = self.inner.missing.is_some() || crate::trace::MISSES;
        let params = WithCount { args, count };
        let found = self.inner.read(|s| {
            let plural = match locale {
                None => s.plural,
                Some(code) => s.plural_of(code),
            };
            match s.lookup(locale, page, key) {
                Some(Entry::Msg(Msg::Text(t))) => Found::Text(t.single_form(&params)),
                Some(Entry::Msg(Msg::Plural(p))) => Found::Text(p.pick(count, plural, &params)),
                // A number is its own one form; a boolean or a subtree has no plural.
                Some(Entry::Value(v)) if !matches!(&**v, "true" | "false") => {
                    Found::Text(String::from(&**v))
                }
                Some(_) => Found::Node,
                None => Found::Miss((report && locale.is_none()).then(|| s.locale.clone())),
            }
        });
        match found {
            Found::Text(s) => s,
            Found::Node => count_fallback(key, &params),
            Found::Miss(active) => {
                self.report(locale.or(active.as_deref()), page, key);
                count_fallback(key, &params)
            }
        }
    }

    fn try_resolve(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> Option<String> {
        self.inner.read(|s| {
            s.lookup(locale, page, key)
                .and_then(Entry::text)
                .map(|t| String::from(&**t))
        })
    }

    fn get_in(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> Option<Arc<str>> {
        self.inner
            .read(|s| s.lookup(locale, page, key).and_then(Entry::text).cloned())
    }

    fn has_in(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> bool {
        self.inner.read(|s| s.has(locale, page, key))
    }

    /// The text of `key` in the active locale: page layers, root, then the fallback chain.
    ///
    /// No interpolation (`{name}` stays as written). A number or boolean comes back as JS
    /// prints it. A miss calls `missing(locale, key, "index")` and returns the key; a
    /// subtree returns the key without calling it; `t("")` is `""`.
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        self.resolve(None, None, key, None)
    }

    /// [`I18n::t`] with parameters and an optional default ([`Args::or`]). Strings are
    /// interpolated; numbers and booleans are returned as is. On a miss the default is
    /// interpolated, else the key comes back as written.
    #[must_use]
    pub fn t_with(&self, key: &str, args: &Args<'_>) -> String {
        self.resolve(None, None, key, Some(args))
    }

    /// The text of `key`, `None` on a miss or a subtree. Never calls `missing`.
    #[must_use]
    pub fn try_t(&self, key: &str) -> Option<String> {
        self.try_resolve(None, None, key)
    }

    /// The plural form of `key` for `count`, with `{count}` filled (JS `tc(key, count)`).
    ///
    /// A value `a | b | c` is split once at load; the form goes by the plural rules of the
    /// locale's `iso` (else its code), as `@i18n-micro/core` `pluralForm` picks it:
    /// - `=0 none | one | many`: the first form for 0, the others by CLDR category;
    /// - `none | {count} apple | {count} apples`-like templates with a zero form and one
    ///   form per category the locale's small counts reach (≥ 3 of them, as in `ru`): by
    ///   category;
    /// - anything else by index: form `count`, the last one past the end.
    ///
    /// A negative, fractional or non-finite count takes the last form; an unknown tag the
    /// index rule. Forms are trimmed. A string without `|` is its own one form, a number
    /// is returned as JS prints it. A boolean or a subtree returns the key, a miss calls
    /// `missing` and returns the key. Exactly one allocation: the result.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .locale("ru")
    ///     .messages("ru", None, r#"{"apples": "нет яблок | {count} яблоко | {count} яблока | {count} яблок",
    ///                               "cars": "car | cars"}"#)
    ///     .build()?;
    /// assert_eq!(i18n.tc("apples", 0), "нет яблок");
    /// assert_eq!(i18n.tc("apples", 21), "21 яблоко");
    /// assert_eq!(i18n.tc("apples", 3), "3 яблока");
    /// assert_eq!(i18n.tc("apples", 11), "11 яблок");
    /// assert_eq!(i18n.tc("cars", 1), "cars"); // index rule: form 1
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tc(&self, key: &str, count: impl Into<Count>) -> String {
        self.resolve_count(None, None, key, count.into(), None)
    }

    /// [`I18n::tc`] with parameters and an optional default (JS `tc(key, { count, ...params },
    /// default)`). `{count}` is always `count`. On a miss, a boolean or a subtree the default
    /// is interpolated (an empty one stays empty, unlike `t_with`), else the key comes back.
    ///
    /// ```
    /// use i18n_next::{Args, I18n};
    ///
    /// let i18n = I18n::builder()
    ///     .messages("en", None, r#"{"inbox": "{name}, no mail | {name}, one mail | {name}, {count} mails"}"#)
    ///     .build()?;
    /// assert_eq!(i18n.tc_with("inbox", 5, &Args::new().str("name", "Ann")), "Ann, 5 mails");
    /// assert_eq!(i18n.tc_with("nope", 2, &Args::new().or("{count} items")), "2 items");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tc_with(&self, key: &str, count: impl Into<Count>, args: &Args<'_>) -> String {
        self.resolve_count(None, None, key, count.into(), Some(args))
    }

    /// The raw stored text of `key` (no interpolation), shared: no allocation.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Arc<str>> {
        self.get_in(None, None, key)
    }

    /// Whether the active locale itself has `key` (a subtree counts), not its fallbacks.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.has_in(None, None, key)
    }

    /// A cheap view of one page: page layer first, then the root, down the chain.
    /// `""` and `"index"` are the root; a page with no chunk reads the root only.
    #[must_use]
    pub fn page(&self, name: &str) -> View {
        View {
            i18n: self.clone(),
            locale: None,
            page: Some(Arc::from(name)),
        }
    }

    /// A cheap view in another locale, without switching the active one. A locale that
    /// is not known has nothing of its own: lookups go to the global fallback alone, misses
    /// call `missing` with that code. A disabled locale reads its own chain, as in Nuxt.
    #[must_use]
    pub fn with_locale(&self, code: &str) -> View {
        View {
            i18n: self.clone(),
            locale: Some(Arc::from(code)),
            page: None,
        }
    }

    /// Switches the active locale for every reader (a new snapshot, version + 1; the same
    /// locale is no change).
    ///
    /// # Errors
    /// [`Error::DisabledLocale`] for a locale `i18n.json` disables, [`Error::UnknownLocale`]
    /// for one no file, config or builder call names. Nothing changes then.
    pub fn set_locale(&self, code: &str) -> Result<(), Error> {
        let change = self.inner.write(|s| {
            if s.meta.disabled(code) {
                return Err(Error::DisabledLocale(code.to_owned()));
            }
            let Some((locale, active)) = s.chains.get_key_value(code) else {
                return Err(Error::UnknownLocale(code.to_owned()));
            };
            if &*s.locale == code {
                return Ok(None);
            }
            // Same data, same chains: only the active pointer moves (no rebuild).
            Ok(Some(Snapshot {
                version: s.version + 1,
                locale: locale.clone(),
                data: s.data.clone(),
                chains: s.chains.clone(),
                active: active.clone(),
                own: s.data.get(code).cloned(),
                fallback: s.fallback.clone(),
                plural: s.plural_of(code),
                plurals: s.plurals.clone(),
                meta: s.meta.clone(),
            }))
        })?;
        if let Some(change) = change {
            self.inner.listeners.send(&change);
        }
        Ok(())
    }

    /// The active locale, shared: no allocation.
    #[must_use]
    pub fn locale(&self) -> Arc<str> {
        self.inner.read(|s| s.locale.clone())
    }

    /// Known locales that are not disabled: `i18n.json` order first, then the rest by code.
    #[must_use]
    pub fn locales(&self) -> Vec<LocaleInfo> {
        self.inner.read(|s| {
            let meta = &s.meta;
            let mut rest: Vec<&str> = s
                .data
                .keys()
                .map(|k| &**k)
                .chain(meta.declared.iter().map(|k| &**k))
                .filter(|c| !meta.configured.iter().any(|l| l.code == *c))
                .collect();
            rest.sort_unstable();
            rest.dedup();
            let info = |code: &str, entry: Option<&loader::LocaleEntry>| LocaleInfo {
                code: code.to_owned(),
                iso: meta
                    .iso
                    .get(code)
                    .map(|s| String::from(&**s))
                    .or_else(|| entry.and_then(|e| e.iso.clone())),
                dir: entry.and_then(|e| e.dir.clone()),
                display_name: entry.and_then(|e| e.display_name.clone()),
            };
            meta.configured
                .iter()
                .filter(|l| !l.disabled)
                .map(|l| info(&l.code, Some(l)))
                .chain(rest.into_iter().map(|c| info(c, None)))
                .collect()
        })
    }

    /// Known page names (from files, `messages` and aliases), sorted.
    #[must_use]
    pub fn pages(&self) -> Vec<String> {
        self.inner.read(|s| {
            let mut pages: Vec<String> = s
                .data
                .values()
                .flat_map(|d| d.pages.keys())
                .chain(s.meta.aliases.keys())
                .map(|p| String::from(&**p))
                .collect();
            pages.sort_unstable();
            pages.dedup();
            pages
        })
    }

    /// Deep-merges a JSON object into the `(locale, page)` chunk (`None`, `""` or `"index"`
    /// = the root; an alias writes to its target). Incoming values win, objects merge.
    /// Readers see it in a new snapshot (version + 1); a new locale becomes known.
    ///
    /// A lazy pair is loaded first, so the messages go over its files; a remote pair
    /// that is not in yet keeps them until its files arrive. Messages added here are not
    /// kept by [`I18n::reload`]: it rebuilds from the builder's sources.
    ///
    /// # Errors
    /// [`Error::Json`] / [`Error::NotObject`] with the path `messages:<locale>/<page>`,
    /// [`Error::UnknownLocale`] / [`Error::UnknownPage`] for an empty or `__proto__` name.
    pub fn add_messages(&self, locale: &str, page: Option<&str>, json: &str) -> Result<(), Error> {
        chunk_names(locale, page)?;
        let tree = Tree::parse(json.as_bytes(), &messages_path(locale, page))?;
        let change = self.inner.write(|s| {
            let mut data = DataMap::clone(&s.data);
            let mut slot = data
                .get(locale)
                .map(|d| LocaleData::clone(d))
                .unwrap_or_default();
            merge_into(&mut slot, s.meta.page(page), tree);
            data.insert(Arc::from(locale), Arc::new(slot));
            Ok(Some(Snapshot::new(
                s.meta.clone(),
                data,
                s.locale.clone(),
                s.version + 1,
            )))
        })?;
        if let Some(change) = change {
            self.inner.listeners.send(&change);
        }
        Ok(())
    }

    /// Reads every source of the builder again (`dir` files, `files`, `extend`,
    /// `messages`; lazy pairs start unloaded, remote files are missing again until the
    /// next `ensure_async` / `add_file`) and publishes the result: version + 1,
    /// subscribers notified. Readers keep answering from the old snapshot until the new one
    /// is in. The active locale stays; messages added with `add_messages` are dropped.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().messages("en", None, r#"{"a": "A"}"#).build()?;
    /// i18n.add_messages("en", None, r#"{"a": "patched"}"#)?;
    /// i18n.reload()?;
    /// assert_eq!((i18n.t("a"), i18n.version()), ("A".to_owned(), 2));
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    ///
    /// # Errors
    /// What [`Builder::build`] returns for a source that cannot be read; the catalog stays
    /// as it was.
    pub fn reload(&self) -> Result<(), Error> {
        let timer = crate::trace::Timer::start();
        let inner = &*self.inner;
        let mut broken = Vec::new();
        let change = inner.write(|s| {
            let mut data = source::load(&inner.sources, &s.meta, &inner.ctl)?;
            // linked messages again, over what was read again (M3, linked.rs)
            if let Some(resolve) = s.meta.linked {
                broken = resolve(&mut data, &s.meta);
                check::lock(&s.meta.check.broken).clone_from(&broken);
            }
            let data = data.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
            Ok(Some(Snapshot::new(
                s.meta.clone(),
                data,
                s.locale.clone(),
                s.version + 1,
            )))
        })?;
        timer.done("reload", "", 0);
        // after the writer is released: a handler may write to the catalog
        report_broken(inner.missing.as_ref(), &broken);
        if let Some(change) = change {
            self.inner.listeners.send(&change);
        }
        Ok(())
    }

    /// The snapshot version: 0 after `build`, + 1 per new snapshot (every change, and a
    /// page eviction, which changes no answer). Only grows.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }
}

/// What `tc` returns without a plural: the default interpolated (an empty one stays, JS
/// `defaultValue ? interpolate(defaultValue, values) : defaultValue ?? key`), else the key.
fn count_fallback(key: &str, params: &WithCount<'_, '_>) -> String {
    match params.args.and_then(Args::default_raw) {
        Some("") => String::new(),
        Some(d) => interpolate_once(d, params),
        None => key.to_owned(),
    }
}

/// Merges `tree` into a copy of the root (`None`) or page chunk of `slot`: the old chunk
/// stays as it is for readers of older snapshots. A lazy chunk is loaded first and becomes
/// a ready one (never evicted: nothing would restore the messages); a remote one that is
/// not in yet keeps the tree until its files arrive.
fn merge_into(slot: &mut LocaleData, page: Option<&str>, tree: Tree) {
    let layer = slot.layer(page);
    let chunk = match layer {
        // A remote pair answers its partial view before its files are in: not loaded.
        Some(Layer::Lazy(cell)) => match if cell.remote {
            cell.loaded()
        } else {
            cell.get()
        } {
            Some(loaded) => Some(loaded),
            None => {
                let waiting = Layer::Lazy(Arc::new(cell.with_over(tree)));
                put(slot, page, waiting);
                return;
            }
        },
        Some(Layer::Ready(c)) => Some(&**c),
        None => None,
    };
    let mut merged = chunk.cloned().unwrap_or_default();
    merged.merge(tree);
    put(slot, page, Layer::Ready(Arc::new(merged)));
}

fn put(slot: &mut LocaleData, page: Option<&str>, chunk: Layer) {
    match page {
        None => slot.root = Some(chunk),
        Some(p) => {
            slot.pages.insert(Arc::from(p), chunk);
        }
    }
}

/// A cheap view of an [`I18n`]: one page and/or another locale, with the same lookups.
/// `Clone + Send + Sync + 'static`; it reads the live catalog, so it sees later changes.
///
/// ```
/// use i18n_next::I18n;
///
/// let i18n = I18n::builder()
///     .locale("en")
///     .messages("en", None, r#"{"title": "App", "ok": "OK"}"#)
///     .messages("en", Some("settings"), r#"{"title": "Settings"}"#)
///     .messages("de", Some("settings"), r#"{"title": "Einstellungen"}"#)
///     .build()?;
/// let settings = i18n.page("settings");
/// assert_eq!(settings.t("title"), "Settings");
/// assert_eq!(settings.t("ok"), "OK");                       // page → root
/// assert_eq!(i18n.with_locale("de").page("settings").t("title"), "Einstellungen");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Clone)]
pub struct View {
    pub(crate) i18n: I18n,
    pub(crate) locale: Option<Arc<str>>,
    pub(crate) page: Option<Arc<str>>,
}

impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("locale", &self.locale)
            .field("page", &self.page)
            .finish_non_exhaustive()
    }
}

impl View {
    fn parts(&self) -> (Option<&str>, Option<&str>) {
        (self.locale.as_deref(), self.page.as_deref())
    }

    /// [`I18n::t`] in this view.
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        let (l, p) = self.parts();
        self.i18n.resolve(l, p, key, None)
    }

    /// [`I18n::t_with`] in this view.
    #[must_use]
    pub fn t_with(&self, key: &str, args: &Args<'_>) -> String {
        let (l, p) = self.parts();
        self.i18n.resolve(l, p, key, Some(args))
    }

    /// [`I18n::try_t`] in this view.
    #[must_use]
    pub fn try_t(&self, key: &str) -> Option<String> {
        let (l, p) = self.parts();
        self.i18n.try_resolve(l, p, key)
    }

    /// [`I18n::tc`] in this view: the plural rules of this view's locale.
    #[must_use]
    pub fn tc(&self, key: &str, count: impl Into<Count>) -> String {
        let (l, p) = self.parts();
        self.i18n.resolve_count(l, p, key, count.into(), None)
    }

    /// [`I18n::tc_with`] in this view.
    #[must_use]
    pub fn tc_with(&self, key: &str, count: impl Into<Count>, args: &Args<'_>) -> String {
        let (l, p) = self.parts();
        self.i18n.resolve_count(l, p, key, count.into(), Some(args))
    }

    /// [`I18n::get`] in this view.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Arc<str>> {
        let (l, p) = self.parts();
        self.i18n.get_in(l, p, key)
    }

    /// [`I18n::has`] in this view: this view's locale only, page then root.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        let (l, p) = self.parts();
        self.i18n.has_in(l, p, key)
    }

    /// The same view on another page.
    #[must_use]
    pub fn page(&self, name: &str) -> View {
        View {
            page: Some(Arc::from(name)),
            ..self.clone()
        }
    }

    /// The same view in another locale (see [`I18n::with_locale`]).
    #[must_use]
    pub fn with_locale(&self, code: &str) -> View {
        View {
            locale: Some(Arc::from(code)),
            ..self.clone()
        }
    }

    /// The locale this view reads: its own, or the active one. Shared: no allocation.
    #[must_use]
    pub fn locale(&self) -> Arc<str> {
        match &self.locale {
            Some(l) => l.clone(),
            None => self.i18n.locale(),
        }
    }
}

enum ConfigSource {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    Path(std::path::PathBuf),
    Json(String),
}

/// Builds an [`I18n`]. Every call is optional; later calls of the same kind add to or
/// override earlier ones, builder calls override `i18n.json`.
///
/// ```
/// use i18n_next::I18n;
///
/// let i18n = I18n::builder()
///     .config_json(r#"{"defaultLocale": "de-at", "fallbackLocale": "en",
///                      "locales": [{"code": "de-at", "fallbackLocale": "de"}]}"#)
///     .files([
///         ("en.json", br#"{"a": "en", "b": "en", "c": "en"}"#.as_slice()),
///         ("de.json", br#"{"a": "de", "b": "de"}"#),
///         ("de-at.json", br#"{"a": "at"}"#),
///     ])
///     .build()?;
/// assert_eq!([i18n.t("a"), i18n.t("b"), i18n.t("c")], ["at", "de", "en"]);
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Default)]
pub struct Builder {
    locale: Option<String>,
    fallback: Option<String>,
    links: Vec<(String, String)>,
    iso: Vec<(String, String)>,
    sources: Sources,
    aliases: Vec<(String, String)>,
    config: Option<ConfigSource>,
    missing: Option<Missing>,
    /// `linked_messages(true)`: the resolver, as a pointer so an app that never turns links
    /// on links none of their code (no case tables in its `.wasm`).
    linked: Option<ResolveLinked>,
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("locale", &self.locale)
            .field("fallback", &self.fallback)
            .field("files", &self.sources.files.len())
            .finish_non_exhaustive()
    }
}

impl Builder {
    /// The active locale at start (else `defaultLocale` of `i18n.json`, else `en`). It is a
    /// known locale even with no file of its own: it reads its chain.
    #[must_use]
    pub fn locale(mut self, code: &str) -> Self {
        self.locale = Some(code.to_owned());
        self
    }

    /// The global fallback: the weakest layer of every chain. The code becomes a known
    /// locale (listed by `locales`, accepted by `set_locale`).
    #[must_use]
    pub fn fallback(mut self, code: &str) -> Self {
        self.fallback = Some(code.to_owned());
        self
    }

    /// A chain step: `locale` falls back to `fallback` before the global fallback
    /// (`de-at → de → en`). Both codes become known locales, so a regional locale with no
    /// file of its own (`de-ch → de`) can be the active one. A cycle (`a → b → a`) stops
    /// at the first repeat.
    #[must_use]
    pub fn fallback_for(mut self, locale: &str, fallback: &str) -> Self {
        self.links.push((locale.to_owned(), fallback.to_owned()));
        self
    }

    /// The BCP 47 tag of a locale code (`ua` → `uk-UA`) for plurals and formats: `tc` goes
    /// by its rules. Without one, the code itself is the tag; a tag `Intl.PluralRules`
    /// rejects or does not know (`en_US`, `xx`) gets the index rule. The code becomes a
    /// known locale.
    #[must_use]
    pub fn iso(mut self, code: &str, tag: &str) -> Self {
        self.iso.push((code.to_owned(), tag.to_owned()));
        self
    }

    /// Reads every root and page file under `path` at `build` (not on
    /// `wasm32-unknown-unknown`: use [`Builder::files`] there).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[must_use]
    pub fn dir(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.sources.dirs.push(path.as_ref().to_path_buf());
        self
    }

    /// Embedded files with the same layout as a directory (`en.json`,
    /// `pages/settings/en.json`): the way to ship locales on Android and in the browser.
    #[must_use]
    pub fn files<I, P, B>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = (P, B)>,
        P: AsRef<str>,
        B: AsRef<[u8]>,
    {
        self.sources.files.extend(embedded(files));
        self
    }

    /// Lists the root and page files under `path` at `build` without reading them: each
    /// `(locale, page)` file is read on the first lookup that reaches its layer, once (not
    /// on `wasm32-unknown-unknown`). A file that cannot be read then is skipped and
    /// reported (stderr in debug builds, or `tracing`): its layer stays empty.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[must_use]
    pub fn lazy_dir(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.sources.lazy_dirs.push(path.as_ref().to_path_buf());
        self.sources.lazy = Some(source::LAZY);
        self
    }

    /// Embedded files like [`Builder::files`], parsed on first use of their
    /// `(locale, page)` pair instead of at `build`, once.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .lazy_files([("en.json", br#"{"a": "A"}"#.as_slice()), ("pages/big/en.json", br#"{"b": "B"}"#)])
    ///     .build()?;
    /// assert_eq!(i18n.pages(), ["big"]);              // known from the name, not read yet
    /// assert_eq!(i18n.page("big").t("b"), "B");       // read now
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn lazy_files<I, P, B>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = (P, B)>,
        P: AsRef<str>,
        B: AsRef<[u8]>,
    {
        self.sources.lazy_files.extend(embedded(files));
        self.sources.lazy = Some(source::LAZY);
        self
    }

    /// The files the app will bring itself, as paths in the locales layout (`de.json`,
    /// `pages/settings/de.json`): known pairs, nothing loaded. A lookup answers from what
    /// is in; [`I18n::ensure_async`] fetches what a locale and page lack,
    /// [`I18n::add_file`] adds bytes fetched by hand. Paths outside the layout are ignored.
    #[must_use]
    pub fn remote<I>(mut self, manifest: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.sources
            .remote
            .extend(manifest.into_iter().map(|p| source::normalize(p.as_ref())));
        self.sources.lazy = Some(source::LAZY);
        self
    }

    /// Locale files of a library (8b.4), merged under the app's own: every `extend` call is
    /// one layer, later calls over earlier ones, and the app's files and messages over all
    /// of them, deep-merged per `(locale, page)`. The app can override any library string.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .extend([("en.json", br#"{"ui": {"ok": "OK", "cancel": "Cancel"}}"#.as_slice())])
    ///     .files([("en.json", br#"{"ui": {"ok": "Fine"}}"#.as_slice())])
    ///     .build()?;
    /// assert_eq!((i18n.t("ui.ok"), i18n.t("ui.cancel")), ("Fine".into(), "Cancel".into()));
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn extend<I, P, B>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = (P, B)>,
        P: AsRef<str>,
        B: AsRef<[u8]>,
    {
        self.sources.extend.push(embedded(files).collect());
        self
    }

    /// Keeps at most `n` lazily loaded pages (at least 1) and evicts the least recently
    /// used beyond that (8b.16). Never evicted: root chunks, pairs with `messages` /
    /// `add_messages`, ready (eager) chunks and pinned pages. An evicted local page loads
    /// again on its next use, with the same answers; a remote one is missing until the next
    /// `ensure_async`. Off by default.
    #[must_use]
    pub fn evict_pages(mut self, n: usize) -> Self {
        self.sources.evict = Some(n.max(1));
        self
    }

    /// Never evicts page `name` (Rust-only pages of the native UI: `tray`, `menu`).
    #[must_use]
    pub fn pin_page(mut self, name: &str) -> Self {
        self.sources.pins.push(name.to_owned());
        self
    }

    /// A JSON object for the `(locale, page)` chunk (`None` = root), merged after files.
    #[must_use]
    pub fn messages(mut self, locale: &str, page: Option<&str>, json: &str) -> Self {
        self.sources
            .messages
            .push((locale.to_owned(), page.map(str::to_owned), json.to_owned()));
        self
    }

    /// `false` folds page files into the root chunk (`disablePageLocales`): files merge in
    /// path order (JS `<`), and a page file wins over the root file on the same key in
    /// either order. A [`Builder::page_alias`] then names nothing: it reads the root.
    #[must_use]
    pub fn pages(mut self, enabled: bool) -> Self {
        self.sources.no_pages = !enabled;
        self
    }

    /// Page `from` reads the chunk of page `to` (`routesLocaleLinks`).
    #[must_use]
    pub fn page_alias(mut self, from: &str, to: &str) -> Self {
        self.aliases.push((from.to_owned(), to.to_owned()));
        self
    }

    /// Reads `i18n.json` from `path` at `build` (not on `wasm32-unknown-unknown`).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[must_use]
    pub fn config(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.config = Some(ConfigSource::Path(path.as_ref().to_path_buf()));
        self
    }

    /// `i18n.json` as text: `defaultLocale`, `fallbackLocale`, `locales[]` (`code`, `iso`,
    /// `dir`, `displayName`, `fallbackLocale`, `disabled`); formats are kept for later.
    #[must_use]
    pub fn config_json(mut self, json: &str) -> Self {
        self.config = Some(ConfigSource::Json(json.to_owned()));
        self
    }

    /// Called on every miss of `t` / `t_with` / `tc` / `tc_with` with `(locale, key, page)`;
    /// the root page is `"index"`. Not called for `try_t`, `get`, `has`, a subtree, a
    /// boolean under `tc` or `t("")`. It runs with no lock held, so it may call the
    /// catalog, writes included (`add_messages`).
    ///
    /// The instance owns the handler: a handler that captures a clone of the same [`I18n`]
    /// makes a reference cycle and the catalog is never freed. Keep the instance in an
    /// `Arc<I18n>` and capture `Arc::downgrade` of it instead.
    #[must_use]
    pub fn missing(mut self, handler: impl Fn(&str, &str, &str) + Send + Sync + 'static) -> Self {
        self.missing = Some(Box::new(handler));
        self
    }

    /// Resolves linked messages at `build` (`@:key`, `@.upper:key`, `@.lower:key`,
    /// `@.capitalize:key`, `@:{'key'}`), as the Nuxt module's `linkedMessages: true`: each
    /// `(locale, page)` view is resolved over everything it reads (its page, root and fallback
    /// locales), the case modifiers by the locale's `iso`. Lookups then read plain text: no
    /// cost at `t`. A link to a missing key, a subtree or a cycle leaves the whole message as
    /// written and calls `missing(locale, key, page)` once; [`I18n::check`] lists it. Off by
    /// default: `@:` may be plain text. Text added later with `add_messages` is not resolved.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .linked_messages(true)
    ///     .messages("en", None, r#"{"brand": "acme", "title": "Welcome to @:brand",
    ///                              "shout": "@.upper:brand", "intro": "@.capitalize:{'brand'}."}"#)
    ///     .build()?;
    /// assert_eq!(i18n.t("title"), "Welcome to acme");
    /// assert_eq!(i18n.t("shout"), "ACME");
    /// assert_eq!(i18n.t("intro"), "Acme.");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn linked_messages(mut self, enabled: bool) -> Self {
        self.linked = enabled.then_some(resolve_linked as ResolveLinked);
        self
    }

    fn read_config(&self) -> Result<Config, Error> {
        match &self.config {
            None => Ok(Config::default()),
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            Some(ConfigSource::Path(path)) => {
                let shown = path.to_string_lossy();
                let bytes = std::fs::read(path).map_err(|source| Error::Io {
                    path: shown.clone().into_owned(),
                    source,
                })?;
                Config::parse(&bytes, &shown)
            }
            Some(ConfigSource::Json(json)) => Config::parse(json.as_bytes(), "i18n.json"),
        }
    }

    /// [`Builder::build`], then [`install`](crate::install) as the global instance.
    ///
    /// # Errors
    /// What `build` returns, or [`Error::AlreadyInstalled`].
    #[cfg(feature = "global")]
    pub fn install(self) -> Result<&'static I18n, Error> {
        crate::scope::install(self.build()?)?;
        crate::scope::global().ok_or(Error::NotInstalled)
    }

    /// Reads everything and builds the catalog.
    ///
    /// # Errors
    /// [`Error::Io`], [`Error::Json`], [`Error::NotObject`] with the file path (a broken
    /// file is never skipped), [`Error::Config`] for a malformed `i18n.json`,
    /// [`Error::DisabledLocale`] when the start locale is disabled, [`Error::UnknownPage`]
    /// for an alias to a page nothing defines, [`Error::UnknownLocale`] for an empty or
    /// `__proto__` locale in `messages`.
    pub fn build(mut self) -> Result<I18n, Error> {
        let config = self.read_config()?;
        let mut meta = Meta {
            fallback: self
                .fallback
                .as_deref()
                .or(config.fallback_locale.as_deref())
                .map(Arc::from),
            formats: (config.number_formats, config.datetime_formats),
            ..Meta::default()
        };
        for l in &config.locales {
            if let Some(fb) = &l.fallback {
                meta.links.insert(Arc::from(&*l.code), Arc::from(&**fb));
            }
            if let Some(iso) = &l.iso {
                meta.iso.insert(Arc::from(&*l.code), Arc::from(&**iso));
            }
        }
        for (l, fb) in &self.links {
            meta.links.insert(Arc::from(&**l), Arc::from(&**fb));
        }
        for (code, tag) in &self.iso {
            meta.iso.insert(Arc::from(&**code), Arc::from(&**tag));
        }
        meta.configured = config.locales;
        let locale: Arc<str> = Arc::from(
            self.locale
                .as_deref()
                .or(config.default_locale.as_deref())
                .unwrap_or("en"),
        );
        let mut declared = Vec::new();
        let mut declare = |code: &str| {
            if !declared.iter().any(|d: &Arc<str>| &**d == code) {
                declared.push(Arc::from(code));
            }
        };
        for code in [self.locale.as_deref(), config.default_locale.as_deref()]
            .into_iter()
            .flatten()
        {
            declare(code);
        }
        declare(&locale);
        for (code, fallback) in &meta.links {
            declare(code);
            declare(fallback);
        }
        for code in meta.fallback.iter().chain(meta.iso.keys()) {
            declare(code);
        }
        meta.declared = declared;
        if meta.disabled(&locale) {
            return Err(Error::DisabledLocale(String::from(&*locale)));
        }

        for (from, to) in &self.aliases {
            meta.aliases.insert(Arc::from(&**from), Arc::from(&**to));
        }
        let ctl = Arc::new(LazyCtl::new(self.sources.evict));
        self.sources.parse_files()?;
        let mut data = source::load(&self.sources, &meta, &ctl)?;
        // Folded pages: an alias resolves to a page with no chunk, which reads the root.
        if !self.sources.no_pages {
            for to in meta.aliases.values() {
                let defined = &**to == "index" || data.values().any(|d| d.pages.contains_key(to));
                if !defined {
                    return Err(Error::UnknownPage(String::from(&**to)));
                }
            }
        }

        // ---- linked messages and check() (M3, linked.rs / check.rs) ----
        meta.check.reference = Some(
            config
                .default_locale
                .as_deref()
                .map_or_else(|| locale.clone(), Arc::from),
        );
        meta.linked = self.linked;
        if let Some(resolve) = self.linked {
            let broken = resolve(&mut data, &meta);
            report_broken(self.missing.as_ref(), &broken);
            *check::lock(&meta.check.broken) = broken;
        }

        let data = data.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        let snapshot = Snapshot::new(Arc::new(meta), data, locale, 0);
        let inner = Arc::new(Inner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            version: AtomicU64::new(0),
            state: RwLock::new(Arc::new(snapshot)),
            writer: Mutex::new(()),
            missing: self.missing,
            alive: Arc::new(AtomicBool::new(true)),
            remote: source::remote_index(&self.sources),
            sources: self.sources,
            ctl: ctl.clone(),
            listeners: Listeners::default(),
        });
        let _ = ctl.owner.set(Arc::downgrade(&inner));
        Ok(I18n { inner })
    }
}

// ---- linked messages and check() (M3, linked.rs / check.rs) ----

/// [`resolve_linked`] as the builder keeps it.
type ResolveLinked = fn(&mut FastMap<Arc<str>, LocaleData>, &Meta) -> Vec<crate::linked::Broken>;

/// Reports each message `broken` left as written once through `missing`. Called with no
/// lock held: the handler may write to the catalog.
fn report_broken(missing: Option<&Missing>, broken: &[crate::linked::Broken]) {
    let Some(missing) = missing else { return };
    let mut last = None;
    for b in broken {
        let at = (&b.locale, &b.page, &b.key);
        if last != Some(at) {
            missing(&b.locale, &b.key, b.page.as_deref().unwrap_or("index"));
            last = Some(at);
        }
    }
}

/// Resolves the linked messages of `data` (`linked.rs`): the ready chunks go into one ordered
/// map and back; each lazy or remote pair gets what it needs to resolve its own messages when
/// it loads ([`crate::lazy::LinkCtx`]). Returns the links that left a message as written.
fn resolve_linked(
    data: &mut FastMap<Arc<str>, LocaleData>,
    meta: &Meta,
) -> Vec<crate::linked::Broken> {
    let mut chunks = crate::linked::Chunks::new();
    for (code, d) in data.iter_mut() {
        if let Some(Layer::Ready(c)) = d.root.take_if(|l| matches!(l, Layer::Ready(_))) {
            chunks.push(((code.clone(), None), c));
        }
        let ready: Vec<Arc<str>> = d
            .pages
            .iter()
            .filter(|(_, l)| matches!(l, Layer::Ready(_)))
            .map(|(p, _)| p.clone())
            .collect();
        for p in ready {
            if let Some(Layer::Ready(c)) = d.pages.remove(&p) {
                chunks.push(((code.clone(), Some(p)), c));
            }
        }
    }
    chunks.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let setup = crate::linked::Setup {
        links: &meta.links,
        fallback: meta.fallback.as_deref(),
        iso: &meta.iso,
    };
    let mut broken = crate::linked::resolve(&mut chunks, &setup);
    // a lazy pair keeps its cell: what was stored for its view is dropped, it resolves its
    // own messages when it loads
    for ((code, page), chunk) in chunks {
        let d = data.entry(code).or_default();
        match page {
            None if matches!(d.root, Some(Layer::Lazy(_))) => {}
            None => d.root = Some(Layer::Ready(chunk)),
            Some(p) if matches!(d.pages.get(&p), Some(Layer::Lazy(_))) => {}
            Some(p) => {
                d.pages.insert(p, Layer::Ready(chunk));
            }
        }
    }
    crate::linked::lazy_contexts(data, &setup);
    crate::linked::mark_lazy(&mut broken, data, &setup);
    broken
}

impl I18n {
    /// Checks the catalog (DESIGN 8b.8): pages with a file in only some locales, keys of
    /// the default locale missing elsewhere, `{placeholders}` that differ from the default
    /// locale's, plural form counts that do not fit the locale's CLDR categories, links that
    /// left a linked message as written. Disabled locales are skipped. Meant for tests:
    /// `assert!(i18n.check().is_clean(), "{}", i18n.check())`.
    #[must_use]
    pub fn check(&self) -> CheckReport {
        self.inner.read(|s| {
            let meta = &s.meta;
            let chunks = s
                .data
                .iter()
                .filter(|(code, _)| !meta.disabled(code))
                .flat_map(|(code, d)| {
                    let root = d
                        .root
                        .iter()
                        .filter_map(move |l| Some((code, None, l.chunk()?)));
                    let pages = d.pages.iter();
                    root.chain(pages.filter_map(move |(p, l)| Some((code, Some(p), l.chunk()?))))
                });
            check::run(chunks, &meta.check, |code| s.plural_of(code))
        })
    }
}

/// `(path with '/', shared bytes)` of embedded files.
fn embedded<I, P, B>(files: I) -> impl Iterator<Item = (String, source::Bytes)>
where
    I: IntoIterator<Item = (P, B)>,
    P: AsRef<str>,
    B: AsRef<[u8]>,
{
    files
        .into_iter()
        .map(|(p, b)| (source::normalize(p.as_ref()), Arc::from(b.as_ref())))
}
