//! `I18n`, its builder and views (DESIGN 3, 4.1, 4.2, 4.5).
//!
//! Reads go through a snapshot: `Inner.version` is one atomic load, and each thread keeps
//! up to four `(instance, version, Arc<Snapshot>)` in a `thread_local!` cache (spike S1),
//! so a hit takes no lock and touches no reference count. A write builds a new snapshot,
//! swaps it under the write lock and bumps the version; a reader holding the old `Arc`
//! never sees a change.

use crate::args::Args;
use crate::catalog::{Chunk, Entry, Msg, Template, Tree};
use crate::error::Error;
use crate::hash::FastMap;
use crate::loader::{self, Config, Kind};
use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

type Missing = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

/// One locale's chunks: the root (`index`) and its pages.
#[derive(Debug, Default, Clone)]
struct LocaleData {
    root: Option<Arc<Chunk>>,
    pages: FastMap<Arc<str>, Arc<Chunk>>,
}

impl LocaleData {
    /// Page layer, then root layer.
    fn get(&self, page: Option<&str>, key: &str) -> Option<&Entry> {
        if let Some(p) = page
            && let Some(e) = self.pages.get(p).and_then(|c| c.get(key))
        {
            return Some(e);
        }
        self.root.as_ref().and_then(|c| c.get(key))
    }
}

type Chain = Arc<[Arc<LocaleData>]>;

/// What the builder fixed: it does not change after `build`.
#[derive(Debug, Default)]
struct Meta {
    fallback: Option<Arc<str>>,
    links: FastMap<Arc<str>, Arc<str>>,
    iso: FastMap<Arc<str>, Arc<str>>,
    /// `i18n.json` `locales[]`, in file order.
    configured: Vec<loader::LocaleEntry>,
    /// Locales the app named itself (`.locale`, `defaultLocale`).
    declared: Vec<Arc<str>>,
    aliases: FastMap<Arc<str>, Arc<str>>,
    /// Raw `numberFormats` / `datetimeFormats` of `i18n.json`, for M5 / M6.
    #[allow(dead_code, reason = "read by the formats of M5 / M6")]
    formats: (Option<Box<str>>, Option<Box<str>>),
}

impl Meta {
    fn disabled(&self, code: &str) -> bool {
        self.configured.iter().any(|l| l.disabled && l.code == code)
    }

    /// The page chunk a name reads: `""` and `index` are the root, an alias its target.
    fn page<'a>(&'a self, page: Option<&'a str>) -> Option<&'a str> {
        let p = page.filter(|p| !p.is_empty() && *p != "index")?;
        let p = self.aliases.get(p).map_or(p, |t| &**t);
        (p != "index").then_some(p)
    }

    /// `code`, its `fallback_for` links, then the global fallback and its links.
    fn chain(&self, data: &FastMap<Arc<str>, Arc<LocaleData>>, code: &str) -> Chain {
        fn follow<'a>(
            links: &'a FastMap<Arc<str>, Arc<str>>,
            start: &'a str,
            codes: &mut Vec<&'a str>,
        ) {
            let mut next = Some(start);
            while let Some(c) = next {
                if codes.contains(&c) {
                    break;
                }
                codes.push(c);
                next = links.get(c).map(|s| &**s);
            }
        }
        let mut codes: Vec<&str> = Vec::new();
        follow(&self.links, code, &mut codes);
        if let Some(fb) = &self.fallback {
            follow(&self.links, fb, &mut codes);
        }
        codes.iter().filter_map(|c| data.get(*c).cloned()).collect()
    }
}

struct Snapshot {
    version: u64,
    locale: Arc<str>,
    data: FastMap<Arc<str>, Arc<LocaleData>>,
    /// Chains of the active locale and every enabled known locale.
    chains: FastMap<Arc<str>, Chain>,
    active: Chain,
    /// The global fallback's chain: what a locale that is not known reads.
    fallback: Chain,
    meta: Arc<Meta>,
}

impl Snapshot {
    fn new(
        meta: Arc<Meta>,
        data: FastMap<Arc<str>, Arc<LocaleData>>,
        locale: Arc<str>,
        version: u64,
    ) -> Snapshot {
        let mut chains = FastMap::default();
        let names = meta
            .configured
            .iter()
            .map(|l| &*l.code)
            .chain(data.keys().map(|k| &**k))
            .chain(meta.declared.iter().map(|k| &**k))
            .chain(std::iter::once(&*locale));
        for code in names {
            if !meta.disabled(code) && !chains.contains_key(code) {
                chains.insert(Arc::from(code), meta.chain(&data, code));
            }
        }
        let active = chains
            .get(&*locale)
            .cloned()
            .unwrap_or_else(|| meta.chain(&data, &locale));
        let fallback = match &meta.fallback {
            Some(fb) => meta.chain(&data, fb),
            None => Arc::from([]),
        };
        Snapshot {
            version,
            locale,
            data,
            chains,
            active,
            fallback,
            meta,
        }
    }

    fn known(&self, code: &str) -> bool {
        self.data.contains_key(code)
            || self.meta.configured.iter().any(|l| l.code == code)
            || self.meta.declared.iter().any(|l| &**l == code)
    }

    /// The layers of `locale` (the active one when `None`). A locale that is not known,
    /// or is disabled, has nothing of its own: it reads the global fallback chain.
    fn layers(&self, locale: Option<&str>) -> &[Arc<LocaleData>] {
        match locale {
            None => &self.active,
            Some(code) => self.chains.get(code).unwrap_or(&self.fallback),
        }
    }

    fn lookup(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> Option<&Entry> {
        let page = self.meta.page(page);
        self.layers(locale)
            .iter()
            .find_map(|layer| layer.get(page, key))
    }

    /// Only the locale itself, page then root: no fallback (JS `has`).
    fn has(&self, locale: Option<&str>, page: Option<&str>, key: &str) -> bool {
        let code = locale.unwrap_or(&self.locale);
        if !self.chains.contains_key(code) {
            return false;
        }
        let page = self.meta.page(page);
        self.data
            .get(code)
            .is_some_and(|d| d.get(page, key).is_some())
    }
}

/// Cached snapshots of this thread: `(instance id, version, snapshot)`, and the slot to
/// evict next.
type CacheSlot = Option<(u64, u64, Arc<Snapshot>)>;

thread_local! {
    static CACHE: RefCell<([CacheSlot; 4], usize)> =
        const { RefCell::new(([const { None }; 4], 0)) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Inner {
    id: u64,
    version: AtomicU64,
    state: RwLock<Arc<Snapshot>>,
    /// Serializes writers; readers never take it.
    writer: Mutex<()>,
    missing: Option<Missing>,
}

impl Inner {
    fn load(&self) -> Arc<Snapshot> {
        match self.state.read() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// `f` on the current snapshot: the thread cache when it is fresh, else one read lock.
    /// ponytail: a cached snapshot of a dropped instance lives until its slot is reused
    /// (at most 4 per thread).
    fn read<R>(&self, mut f: impl FnMut(&Snapshot) -> R) -> R {
        let version = self.version.load(Ordering::Acquire);
        let cached = CACHE.try_with(|cell| {
            if let Ok(cache) = cell.try_borrow()
                && let Some((_, _, snap)) = cache
                    .0
                    .iter()
                    .flatten()
                    .find(|(id, v, _)| *id == self.id && *v == version)
            {
                return f(snap);
            }
            let snap = self.load();
            if let Ok(mut cache) = cell.try_borrow_mut() {
                let (slots, next) = &mut *cache;
                let at = slots
                    .iter()
                    .position(|s| s.as_ref().is_some_and(|(id, _, _)| *id == self.id))
                    .or_else(|| slots.iter().position(Option::is_none))
                    .unwrap_or_else(|| {
                        let at = *next;
                        *next = (at + 1) % slots.len();
                        at
                    });
                slots[at] = Some((self.id, snap.version, snap.clone()));
            }
            f(&snap)
        });
        match cached {
            Ok(r) => r,
            // Thread-local storage is gone (a thread-exit destructor calls in).
            Err(_) => f(&self.load()),
        }
    }

    /// Builds the next snapshot from the current one and publishes it; `None` = no change.
    fn write(
        &self,
        f: impl FnOnce(&Snapshot) -> Result<Option<Snapshot>, Error>,
    ) -> Result<(), Error> {
        let _writer = match self.writer.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let current = self.load();
        if let Some(next) = f(&current)? {
            let version = next.version;
            let mut state = match self.state.write() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            *state = Arc::new(next);
            self.version.store(version, Ordering::Release);
        }
        Ok(())
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
    inner: Arc<Inner>,
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
            .finish_non_exhaustive()
    }
}

/// Validates a name passed in code for a chunk: not empty, not `__proto__`.
fn chunk_names(locale: &str, page: Option<&str>) -> Result<(), Error> {
    if locale.is_empty() || locale == "__proto__" {
        return Err(Error::UnknownLocale(locale.to_owned()));
    }
    match page {
        Some("__proto__") => Err(Error::UnknownPage("__proto__".to_owned())),
        _ => Ok(()),
    }
}

/// `messages:<locale>/<page>`: the path of text passed in code, in errors.
fn messages_path(locale: &str, page: Option<&str>) -> String {
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
        let report = self.inner.missing.is_some();
        let found = self.inner.read(|s| match s.lookup(locale, page, key) {
            Some(Entry::Msg(Msg::Text(t))) => Found::Text(match args {
                Some(a) => t.interpolate(a),
                None => String::from(&*t.raw),
            }),
            Some(Entry::Value(v)) => Found::Text(String::from(&**v)),
            Some(Entry::Node) => Found::Node,
            None => Found::Miss((report && locale.is_none()).then(|| s.locale.clone())),
        });
        match found {
            Found::Text(s) => s,
            Found::Node => key.to_owned(),
            Found::Miss(active) => {
                if let Some(missing) = &self.inner.missing {
                    let code = locale.or(active.as_deref()).unwrap_or("");
                    let page = page.filter(|p| !p.is_empty()).unwrap_or("index");
                    missing(code, key, page);
                }
                match args.and_then(|a| a.default_text().map(|d| (a, d))) {
                    Some((a, d)) => Template::new(d).interpolate(a),
                    None => key.to_owned(),
                }
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
    /// is not known (or is disabled) has nothing of its own: lookups go to the global
    /// fallback chain, misses call `missing` with that code.
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
        self.inner.write(|s| {
            if s.meta.disabled(code) {
                return Err(Error::DisabledLocale(code.to_owned()));
            }
            if !s.known(code) {
                return Err(Error::UnknownLocale(code.to_owned()));
            }
            if &*s.locale == code {
                return Ok(None);
            }
            Ok(Some(Snapshot::new(
                s.meta.clone(),
                s.data.clone(),
                Arc::from(code),
                s.version + 1,
            )))
        })
    }

    /// The active locale.
    #[must_use]
    pub fn locale(&self) -> String {
        self.inner.read(|s| String::from(&*s.locale))
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
    /// # Errors
    /// [`Error::Json`] / [`Error::NotObject`] with the path `messages:<locale>/<page>`,
    /// [`Error::UnknownLocale`] / [`Error::UnknownPage`] for an empty or `__proto__` name.
    pub fn add_messages(&self, locale: &str, page: Option<&str>, json: &str) -> Result<(), Error> {
        chunk_names(locale, page)?;
        let tree = Tree::parse(json.as_bytes(), &messages_path(locale, page))?;
        self.inner.write(|s| {
            let mut data = s.data.clone();
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
        })
    }

    /// The snapshot version: 0 after `build`, + 1 per change. Only grows.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }
}

/// Merges `tree` into the root (`None`) or page chunk of `slot`.
fn merge_into(slot: &mut LocaleData, page: Option<&str>, tree: Tree) {
    let chunk = match page {
        None => slot.root.as_ref(),
        Some(p) => slot.pages.get(p),
    };
    let mut merged = chunk.map(|c| Tree::clone(c)).unwrap_or_default();
    merged.merge(tree);
    let chunk = Arc::new(merged);
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
    i18n: I18n,
    locale: Option<Arc<str>>,
    page: Option<Arc<str>>,
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

    /// The locale this view reads: its own, or the active one.
    #[must_use]
    pub fn locale(&self) -> String {
        match &self.locale {
            Some(l) => String::from(&**l),
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
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    dirs: Vec<std::path::PathBuf>,
    files: Vec<(String, Vec<u8>)>,
    messages: Vec<(String, Option<String>, String)>,
    no_pages: bool,
    aliases: Vec<(String, String)>,
    config: Option<ConfigSource>,
    missing: Option<Missing>,
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("locale", &self.locale)
            .field("fallback", &self.fallback)
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl Builder {
    /// The active locale at start (else `defaultLocale` of `i18n.json`, else `en`).
    #[must_use]
    pub fn locale(mut self, code: &str) -> Self {
        self.locale = Some(code.to_owned());
        self
    }

    /// The global fallback: the weakest layer of every chain.
    #[must_use]
    pub fn fallback(mut self, code: &str) -> Self {
        self.fallback = Some(code.to_owned());
        self
    }

    /// A chain step: `locale` falls back to `fallback` before the global fallback
    /// (`de-at → de → en`).
    #[must_use]
    pub fn fallback_for(mut self, locale: &str, fallback: &str) -> Self {
        self.links.push((locale.to_owned(), fallback.to_owned()));
        self
    }

    /// The BCP 47 tag of a locale code (`ua` → `uk-UA`) for plurals and formats.
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
        self.dirs.push(path.as_ref().to_path_buf());
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
        for (path, bytes) in files {
            let path = path.as_ref().replace('\\', "/");
            self.files.push((path, bytes.as_ref().to_vec()));
        }
        self
    }

    /// A JSON object for the `(locale, page)` chunk (`None` = root), merged after files.
    #[must_use]
    pub fn messages(mut self, locale: &str, page: Option<&str>, json: &str) -> Self {
        self.messages
            .push((locale.to_owned(), page.map(str::to_owned), json.to_owned()));
        self
    }

    /// `false` folds page files into the root chunk (`disablePageLocales`); a page file
    /// wins over the root file on the same key.
    #[must_use]
    pub fn pages(mut self, enabled: bool) -> Self {
        self.no_pages = !enabled;
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

    /// Called on every miss of `t` / `t_with` with `(locale, key, page)`; the root page is
    /// `"index"`. Not called for `try_t`, `get`, `has`, a subtree or `t("")`.
    #[must_use]
    pub fn missing(mut self, handler: impl Fn(&str, &str, &str) + Send + Sync + 'static) -> Self {
        self.missing = Some(Box::new(handler));
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

    /// Reads everything and builds the catalog.
    ///
    /// # Errors
    /// [`Error::Io`], [`Error::Json`], [`Error::NotObject`] with the file path (a broken
    /// file is never skipped), [`Error::Config`] for a malformed `i18n.json`,
    /// [`Error::DisabledLocale`] when the start locale is disabled, [`Error::UnknownPage`]
    /// for an alias to a page nothing defines, [`Error::UnknownLocale`] for an empty or
    /// `__proto__` locale in `messages`.
    pub fn build(self) -> Result<I18n, Error> {
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
        for code in [self.locale.as_deref(), config.default_locale.as_deref()]
            .into_iter()
            .flatten()
        {
            meta.declared.push(Arc::from(code));
        }
        if meta.disabled(&locale) {
            return Err(Error::DisabledLocale(String::from(&*locale)));
        }

        // Files: (relative path, bytes, path for errors).
        let mut files: Vec<(String, Vec<u8>, String)> = Vec::new();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        for dir in &self.dirs {
            for (rel, full) in loader::list_dir(dir)? {
                if loader::classify(&rel) == Kind::Ignore {
                    continue;
                }
                let shown = full.to_string_lossy().into_owned();
                let bytes = std::fs::read(&full).map_err(|source| Error::Io {
                    path: shown.clone(),
                    source,
                })?;
                files.push((rel, bytes, shown));
            }
        }
        for (rel, bytes) in self.files {
            let shown = rel.clone();
            files.push((rel, bytes, shown));
        }
        // Path order (JS `<`), page files after root files: a page file folded into the
        // root wins. The index makes the order total, so an unstable sort is stable.
        let rank: Vec<bool> = files
            .iter()
            .map(|f| matches!(loader::classify(&f.0), Kind::Page(..)))
            .collect();
        let mut order: Vec<usize> = (0..files.len()).collect();
        order.sort_unstable_by(|&a, &b| {
            rank[a]
                .cmp(&rank[b])
                .then_with(|| loader::js_order(&files[a].0, &files[b].0))
                .then(a.cmp(&b))
        });

        let mut chunks: Vec<((String, Option<String>), Tree)> = Vec::new();
        let mut add = |locale: &str, page: Option<&str>, tree: Tree| {
            let page = page.filter(|p| *p != "index");
            match chunks
                .iter_mut()
                .find(|((l, p), _)| l == locale && p.as_deref() == page)
            {
                Some((_, t)) => t.merge(tree),
                None => chunks.push(((locale.to_owned(), page.map(str::to_owned)), tree)),
            }
        };
        for &i in &order {
            let (rel, bytes, shown) = &files[i];
            let (locale, page) = match loader::classify(rel) {
                Kind::Ignore => continue,
                Kind::Root(l) => (l, None),
                Kind::Page(_, l) if self.no_pages => (l, None),
                Kind::Page(p, l) => (l, Some(p)),
            };
            add(locale, page.as_deref(), Tree::parse(bytes, shown)?);
        }
        for (from, to) in &self.aliases {
            meta.aliases.insert(Arc::from(&**from), Arc::from(&**to));
        }
        for (locale, page, json) in &self.messages {
            let page = page.as_deref();
            chunk_names(locale, page)?;
            let tree = Tree::parse(json.as_bytes(), &messages_path(locale, page))?;
            add(locale, meta.page(page), tree);
        }
        for to in meta.aliases.values() {
            let defined =
                &**to == "index" || chunks.iter().any(|((_, p), _)| p.as_deref() == Some(&**to));
            if !defined {
                return Err(Error::UnknownPage(String::from(&**to)));
            }
        }

        let mut data: FastMap<Arc<str>, LocaleData> = FastMap::default();
        for ((locale, page), tree) in chunks {
            let slot = data.entry(Arc::from(locale)).or_default();
            let chunk = Arc::new(tree);
            match page {
                None => slot.root = Some(chunk),
                Some(p) => {
                    slot.pages.insert(Arc::from(p), chunk);
                }
            }
        }
        let data = data.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        let snapshot = Snapshot::new(Arc::new(meta), data, locale, 0);
        Ok(I18n {
            inner: Arc::new(Inner {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                version: AtomicU64::new(0),
                state: RwLock::new(Arc::new(snapshot)),
                writer: Mutex::new(()),
                missing: self.missing,
            }),
        })
    }
}
