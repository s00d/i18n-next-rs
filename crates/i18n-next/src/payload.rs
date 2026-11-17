//! The payload the Tauri plugin sends the front end and a Rust guest applies (DESIGN 2.1,
//! 6.1, 8b.1): one format, one code for both sides. Feature `payload` (`tauri-guest`
//! turns it on).
//!
//! A payload is one JSON object:
//!
//! ```json
//! {"version":3,"locale":"de",
//!  "defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"de","iso":"de-DE"}],
//!  "chains":{"de":["de","en"],"en":["en"]},"numberFormats":{},"datetimeFormats":{},
//!  "timeZone":null,
//!  "chunks":[{"locale":"de","page":"index","data":{"nav":{"home":"Start"}},
//!             "linked":{"title":"Start · Acme"}}]}
//! ```
//!
//! - `version`: the catalog version the chunks come from ([`I18n::version`]);
//! - `locale`: the locale the chunks are for (the active one unless asked otherwise);
//! - `defaultLocale` … `timeZone`: only with `meta` (the first `load`), what
//!   `createI18n` needs (8b.1); `timeZone` is the IANA zone dates show in when it is
//!   known by name (`Builder::time_zone`, `I18n::set_system_time_zone`), else `null`; `fallbackLocale` is `null` when none is set; `chains` is
//!   the fallback chain of every listed locale, strongest first, as the catalog reads it:
//!   the front end premerges a view's chunks in that order (weakest first), since the JS
//!   core reads one locale and one fallback only;
//! - `chunks`: the layers of the locale's fallback chain, strongest first, each the
//!   nested tree JS `addRouteTranslations(locale, page, data)` takes; the root is page
//!   `index`. A pair without data is left out. `linked` (when there is any) holds what
//!   linked messages of weaker layers resolve to in this chunk's view, by flat dotted key:
//!   JS merges it into `data` (own root keys win over the dotted walk), a guest keeps it
//!   apart so `has` does not see it, as on the server.
//!
//! Chunk trees are written from the loaded catalog (merged files, `extend`, `messages`,
//! resolved linked messages), keys sorted: the same catalog gives the same bytes. Each
//! chunk is written once and cached with its content tag ([`chunk`]), which the protocol
//! puts in its URLs: a change that leaves a chunk as it was keeps its URL.

use crate::catalog::{Entry, Msg, Tree, content_tag, path_budget};
use crate::error::Error;
use crate::i18n::{I18n, LocaleData, Snapshot};
use crate::json::{Event, Reader, number_to_js_string, write_str};
use crate::lazy::Layer;
use std::sync::{Arc, OnceLock};

/// The payload of the `locale` fallback chain (the active locale when `None`): its roots
/// and, with `page`, its page chunks; with `meta`, the locale list, the chains,
/// `defaultLocale` and the named formats too. Returns the catalog version and the JSON
/// bytes: what the plugin's `load` sends. Names are not checked here: the plugin accepts
/// only known ones (8b.2); an unknown locale reads the global fallback, as everywhere.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .locale("de")
///     .fallback("en")
///     .messages("de", None, r#"{"a": "A-de"}"#)
///     .messages("en", None, r#"{"a": "A", "b": "B"}"#)
///     .messages("de", Some("settings"), r#"{"t": "Einstellungen"}"#)
///     .build()?;
/// let (version, bytes) = payload::encode(&i18n, None, Some("settings"), false);
/// assert_eq!(version, 0);
/// assert_eq!(
///     String::from_utf8(bytes).unwrap(),
///     r#"{"version":0,"locale":"de","chunks":[{"locale":"de","page":"index","data":{"a":"A-de"}},{"locale":"de","page":"settings","data":{"t":"Einstellungen"}},{"locale":"en","page":"index","data":{"a":"A","b":"B"}}]}"#
/// );
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn encode(i18n: &I18n, locale: Option<&str>, page: Option<&str>, meta: bool) -> (u64, Vec<u8>) {
    i18n.inner.read(|s| {
        let code = locale.unwrap_or(&s.locale);
        let mut out = head(s, code);
        if meta {
            write_meta(&mut out, s);
        }
        write_chunks(&mut out, s, code, page, true);
        (s.public, out.into_bytes())
    })
}

/// The page chunks of the active chain alone, for navigation (the plugin's `load_page`):
/// the roots are on the front end already.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"a": "A"}"#)
///     .messages("en", Some("about"), r#"{"t": "About"}"#)
///     .build()?;
/// let (_, bytes) = payload::encode_page(&i18n, "about");
/// assert_eq!(
///     String::from_utf8(bytes).unwrap(),
///     r#"{"version":0,"locale":"en","chunks":[{"locale":"en","page":"about","data":{"t":"About"}}]}"#
/// );
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn encode_page(i18n: &I18n, page: &str) -> (u64, Vec<u8>) {
    i18n.inner.read(|s| {
        let mut out = head(s, &s.locale);
        write_chunks(&mut out, s, &s.locale, Some(page), false);
        (s.public, out.into_bytes())
    })
}

/// What a pair without data sends.
const EMPTY: &str = "{\"data\":{}}";

/// One `(locale, page)` chunk as the front end gets it, `{"data":{…}}` (`{}` when the pair
/// has no data) plus `"linked"` when it holds any (module docs), with its content tag:
/// the protocol's `/{tag}/{locale}/{page}.json`. The JSON is written once per loaded
/// chunk and shared: a call clones an `Arc`. `None`, `""` and `"index"` are the root; an
/// alias reads its target.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"list": ["a", {"b": true}], "n": 1.50, "x": null, "inf": 1e400}"#)
///     .build()?;
/// let (tag, json) = payload::chunk(&i18n, "en", None);
/// assert_eq!(&*json, r#"{"data":{"inf":null,"list":["a",{"b":true}],"n":1.5,"x":null}}"#);
/// assert_eq!(payload::chunk(&i18n, "en", Some("index")).0, tag);
/// assert_eq!(&*payload::chunk(&i18n, "de", Some("x")).1, r#"{"data":{}}"#);
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn chunk(i18n: &I18n, locale: &str, page: Option<&str>) -> (u64, Arc<str>) {
    i18n.inner.read(|s| {
        let page = s.meta.page(page);
        match s.data.get(locale).and_then(|d| tree(d, page)) {
            Some(t) => {
                let (json, tag) = t.payload();
                (*tag, json.clone())
            }
            None => (content_tag(EMPTY.as_bytes()), Arc::from(EMPTY)),
        }
    })
}

/// The protocol's bundle of the `locale` chain (roots and, with `page`, page chunks:
/// `{"locale":…,"chunks":[…]}`, the chunks as [`encode`] writes them) and its tag, a hash
/// of the tags of its chunks: the same chunks give the same tag. The body is built only
/// when `want` is `None` or the current tag, so a stale URL costs no serialization.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"a": "A"}"#)
///     .messages("de", None, r#"{"a": "A-de"}"#)
///     .build()?;
/// let (tag, body) = payload::bundle(&i18n, "en", None, None);
/// assert_eq!(body.unwrap(), br#"{"locale":"en","chunks":[{"locale":"en","page":"index","data":{"a":"A"}}]}"#);
/// i18n.set_locale("de")?; // another version, the same chunks: the same tag
/// assert_eq!(payload::bundle(&i18n, "en", None, Some(tag)), (tag, Some(body_of(&i18n))));
/// assert_eq!(payload::bundle(&i18n, "en", None, Some(tag ^ 1)), (tag, None));
/// # fn body_of(i: &I18n) -> Vec<u8> { payload::bundle(i, "en", None, None).1.unwrap() }
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn bundle(
    i18n: &I18n,
    locale: &str,
    page: Option<&str>,
    want: Option<u64>,
) -> (u64, Option<Vec<u8>>) {
    i18n.inner.read(|s| {
        let tag = bundle_tag(s, locale, page);
        if want.is_some_and(|w| w != tag) {
            return (tag, None);
        }
        let mut out = String::with_capacity(256);
        out.push_str("{\"locale\":");
        write_str(&mut out, locale);
        write_chunks(&mut out, s, locale, page, true);
        (tag, Some(out.into_bytes()))
    })
}

/// What the front end needs to start and to build protocol URLs (the plugin's `meta`
/// command): `version`, `locale` (the active one when `None`), the meta fields of
/// [`encode`] (locales, chains, `defaultLocale`, formats), `tags` (the content tag of
/// every chunk of the locale's chain with data, roots and `page`, by `"<locale>/<page>"`)
/// and `bundle` (the tag of [`bundle`] for that locale and page).
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .config_json(r#"{"fallbackLocale": "en", "locales": [{"code": "de-at", "fallbackLocale": "de"}]}"#)
///     .messages("de", None, r#"{"a": "A-de"}"#)
///     .messages("en", None, r#"{"a": "A"}"#)
///     .locale("de-at")
///     .build()?;
/// let meta = String::from_utf8(payload::meta(&i18n, None, None)).unwrap();
/// assert!(meta.contains(r#""chains":{"de-at":["de-at","de","en"],"de":["de","en"],"en":["en"]}"#));
/// let (tag, _) = payload::chunk(&i18n, "de", None);
/// assert!(meta.contains(&format!(r#""de/index":"{tag:016x}""#)));
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn meta(i18n: &I18n, locale: Option<&str>, page: Option<&str>) -> Vec<u8> {
    i18n.inner.read(|s| {
        let code = locale.unwrap_or(&s.locale);
        let mut out = head(s, code);
        write_meta(&mut out, s);
        out.push_str(",\"tags\":{");
        let mut first = true;
        for_chunks(s, code, page, true, |c, name, t| {
            if !first {
                out.push(',');
            }
            first = false;
            out.push('"');
            push_name(&mut out, c);
            out.push('/');
            push_name(&mut out, name);
            out.push_str("\":");
            push_tag(&mut out, t.payload().1);
        });
        out.push_str("},\"bundle\":");
        push_tag(&mut out, bundle_tag(s, code, page));
        out.push('}');
        out.into_bytes()
    })
}

/// Applies a payload to `i18n` (a Rust guest, DESIGN 2.1): every chunk replaces its
/// `(locale, page)` pair (`linked` texts included), and the payload's locale becomes the
/// active one unless it is disabled; one new snapshot, subscribers notified. Returns the
/// payload's `version` (0 when it has none). Meta fields are not applied here: a guest
/// builds its instance from them. One pass over the bytes.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let server = I18n::builder()
///     .locale("de")
///     .messages("de", None, r#"{"hi": "Hallo"}"#)
///     .build()?;
/// let guest = I18n::builder().build()?;
/// let (_, bytes) = payload::encode(&server, None, None, true);
/// assert_eq!(payload::apply(&guest, &bytes)?, 0);
/// assert_eq!((&*guest.locale(), guest.t("hi", str::to_owned)), ("de", "Hallo".into()));
/// assert!(payload::apply(&guest, b"[]").is_err());
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// # Errors
/// [`Error::Json`] / [`Error::NotObject`] (path `payload`, or `payload:<locale>/<page>`
/// for a chunk whose names came before its `data`) for broken JSON, [`Error::Config`] for
/// a field of the wrong type, [`Error::UnknownLocale`] / [`Error::UnknownPage`] for an
/// empty or `__proto__` name. Nothing changes then.
pub fn apply(i18n: &I18n, bytes: &[u8]) -> Result<u64, Error> {
    apply_with(i18n, bytes, Vec::new())
}

/// A loaded chunk as [`apply`] stores it: `(locale, page, tree)`, `None` for the root.
pub(crate) type Stored = (Arc<str>, Option<String>, Arc<Tree>);

/// [`apply`], plus `extra` chunks a guest had already (by content tag) in the same snapshot.
pub(crate) fn apply_with(i18n: &I18n, bytes: &[u8], mut extra: Vec<Stored>) -> Result<u64, Error> {
    let shape = |reason: &'static str| Error::Config {
        path: "payload".to_owned(),
        reason,
    };
    let mut r = Payload {
        r: Reader::new(bytes),
        budget: path_budget(bytes.len()),
    };
    if !matches!(r.next()?, Event::ObjectStart) {
        return Err(Error::NotObject {
            path: "payload".to_owned(),
        });
    }
    let mut version = 0;
    let mut locale = None;
    let mut chunks = Vec::new();
    while let Event::Key(key) = r.next()? {
        match &*key {
            "version" => match r.next()? {
                Event::Num(n) => version = n.parse().map_err(|_| shape("version"))?,
                _ => return Err(shape("version must be a number")),
            },
            "locale" => match r.next()? {
                Event::Str(s) => locale = Some(s.into_owned()),
                _ => return Err(shape("locale must be a string")),
            },
            "chunks" => {
                if !matches!(r.next()?, Event::ArrayStart) {
                    return Err(shape("chunks must be an array"));
                }
                loop {
                    match r.next()? {
                        Event::ArrayEnd => break,
                        Event::ObjectStart => {
                            let (l, p, t) = r.chunk()?;
                            chunks.push((l, p, Arc::new(t)));
                        }
                        _ => return Err(shape("chunks[] must hold objects")),
                    }
                }
            }
            _ => r.skip_value()?,
        }
    }
    // trailing bytes are an error too
    r.r.next().map_err(json_error)?;
    if let Some(l) = &locale {
        crate::i18n::chunk_names(l, None)?;
    }
    chunks.append(&mut extra);
    i18n.replace_chunks(chunks, locale.as_deref())?;
    Ok(version)
}

/// A [`Builder`] set up as a [`meta`] answer describes the catalog: its locales (`iso`,
/// `dir`, `displayName`), `defaultLocale`, the fallback chain of every locale, the named
/// formats, the zone dates show in (`timeZone`, feature `format`) and the active locale.
/// What a Rust guest builds its instance from before it [`apply`]s chunks (the
/// `tauri-guest` feature does this in `connect`); add a `missing` handler or a clock, then
/// `build`.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let server = I18n::builder()
///     .config_json(r#"{"fallbackLocale": "en", "locales": [{"code": "de-at", "fallbackLocale": "de"}]}"#)
///     .messages("de", None, r#"{"a": "A-de"}"#)
///     .messages("en", None, r#"{"a": "A", "b": "B"}"#)
///     .locale("de-at")
///     .build()?;
/// let guest = payload::builder(&payload::meta(&server, None, None))?.build()?;
/// payload::apply(&guest, &payload::encode(&server, None, None, false).1)?;
/// assert_eq!((&*guest.locale(), guest.t("a", str::to_owned), guest.t("b", str::to_owned)), ("de-at", "A-de".into(), "B".into()));
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// # Errors
/// [`Error::Json`] / [`Error::NotObject`] / [`Error::Config`] for a broken meta (path
/// `payload`); `build` then checks the formats and the zone as for `i18n.json`.
pub fn builder(meta: &[u8]) -> Result<crate::i18n::Builder, Error> {
    let m = Meta::read(meta)?;
    let text = std::str::from_utf8(meta).map_err(|_| Error::Config {
        path: "payload".to_owned(),
        reason: "meta is not UTF-8",
    })?;
    Ok(m.builder(text))
}

/// The parts of a [`meta`] answer a guest acts on.
#[derive(Debug, Default)]
pub(crate) struct Meta {
    pub(crate) version: u64,
    pub(crate) locale: String,
    pub(crate) fallback: Option<String>,
    /// Every locale's chain, strongest first.
    pub(crate) chains: Vec<(String, Vec<String>)>,
    /// `("<locale>/<page>", tag)` of every chunk of the chain with data.
    pub(crate) tags: Vec<(String, String)>,
    pub(crate) time_zone: Option<String>,
}

impl Meta {
    pub(crate) fn read(bytes: &[u8]) -> Result<Meta, Error> {
        let shape = |reason: &'static str| Error::Config {
            path: "payload".to_owned(),
            reason,
        };
        let mut r = Payload {
            r: Reader::new(bytes),
            budget: 0,
        };
        if !matches!(r.next()?, Event::ObjectStart) {
            return Err(Error::NotObject {
                path: "payload".to_owned(),
            });
        }
        let mut m = Meta::default();
        let text_or_null = |r: &mut Payload<'_>, reason| match r.next()? {
            Event::Str(s) => Ok(Some(s.into_owned())),
            Event::Null => Ok(None),
            _ => Err(shape(reason)),
        };
        while let Event::Key(key) = r.next()? {
            match &*key {
                "version" => match r.next()? {
                    Event::Num(n) => m.version = n.parse().map_err(|_| shape("version"))?,
                    _ => return Err(shape("version must be a number")),
                },
                "locale" => {
                    m.locale = text_or_null(&mut r, "locale must be a string")?
                        .ok_or(shape("locale must be a string"))?;
                }
                "fallbackLocale" => {
                    m.fallback = text_or_null(&mut r, "fallbackLocale must be a string")?;
                }
                "timeZone" => m.time_zone = text_or_null(&mut r, "timeZone must be a string")?,
                "chains" => {
                    if !matches!(r.next()?, Event::ObjectStart) {
                        return Err(shape("chains must be an object"));
                    }
                    while let Event::Key(code) = r.next()? {
                        if !matches!(r.next()?, Event::ArrayStart) {
                            return Err(shape("chains{} must hold arrays"));
                        }
                        let mut chain = Vec::new();
                        loop {
                            match r.next()? {
                                Event::ArrayEnd => break,
                                Event::Str(c) => chain.push(c.into_owned()),
                                _ => return Err(shape("chains{}[] must hold strings")),
                            }
                        }
                        m.chains.push((code.into_owned(), chain));
                    }
                }
                "tags" => {
                    if !matches!(r.next()?, Event::ObjectStart) {
                        return Err(shape("tags must be an object"));
                    }
                    while let Event::Key(at) = r.next()? {
                        match r.next()? {
                            Event::Str(t) => m.tags.push((at.into_owned(), t.into_owned())),
                            _ => return Err(shape("tags{} must hold strings")),
                        }
                    }
                }
                _ => r.skip_value()?,
            }
        }
        r.r.next().map_err(json_error)?;
        Ok(m)
    }

    /// `text` (the meta itself) read as `i18n.json` gives the locales, `defaultLocale`, the
    /// global fallback and the formats; the chains become `fallback_for` links.
    pub(crate) fn builder(&self, text: &str) -> crate::i18n::Builder {
        let mut b = crate::i18n::Builder::default()
            .config_json(text)
            .locale(&self.locale);
        for (code, chain) in &self.chains {
            // a chain of `code` and the global fallback alone needs no link: the fallback
            // closes every chain, never expanded through its own links
            if let Some(next) = chain.get(1)
                && !(chain.len() == 2 && self.fallback.as_deref() == Some(next.as_str()))
            {
                b = b.fallback_for(code, next);
            }
        }
        #[cfg(feature = "format")]
        if let Some(zone) = &self.time_zone {
            b = b.time_zone(zone);
        }
        b
    }
}

/// Named formats of `i18n.json` and then of the builder's calls, kept as given and merged
/// by name on first use: the `numberFormats` / `datetimeFormats` objects of the payload.
/// Lazy, so an app that never encodes a payload links none of the merge code.
#[derive(Debug, Default)]
pub(crate) struct Formats {
    numbers: Vec<Box<str>>,
    dates: Vec<Box<str>>,
    json: OnceLock<(Box<str>, Box<str>)>,
}

impl Formats {
    pub(crate) fn new(b: &crate::i18n::Builder, config: &crate::loader::Config) -> Formats {
        #[cfg(feature = "format")]
        let (numbers, dates) = (&b.formats[..], &b.datetime_formats[..]);
        #[cfg(not(feature = "format"))]
        let (numbers, dates): (&[String], &[String]) = {
            let _ = b;
            (&[], &[])
        };
        let texts = |first: Option<&str>, rest: &[String]| -> Vec<Box<str>> {
            first
                .into_iter()
                .chain(rest.iter().map(String::as_str))
                .map(Box::from)
                .collect()
        };
        Formats {
            numbers: texts(config.number_formats.as_deref(), numbers),
            dates: texts(config.datetime_formats.as_deref(), dates),
            json: OnceLock::new(),
        }
    }

    /// The merged objects. The texts were checked at `build` (`format`) or come from a
    /// parsed `i18n.json`; one that does not parse is left out.
    fn json(&self) -> &(Box<str>, Box<str>) {
        self.json.get_or_init(|| {
            let merged = |texts: &[Box<str>]| {
                let mut acc = Tree::default();
                for text in texts {
                    if let Ok(t) = Tree::parse(text.as_bytes(), "formats") {
                        acc.merge_replacing(t, 2);
                    }
                }
                let mut out = String::new();
                acc.write_json(&mut out);
                out.into_boxed_str()
            };
            (merged(&self.numbers), merged(&self.dates))
        })
    }
}

/// The loaded root (`None`) or page tree of one locale.
fn tree<'a>(d: &'a LocaleData, page: Option<&str>) -> Option<&'a Tree> {
    d.layer(page).and_then(Layer::chunk)
}

/// `{"version":N,"locale":"<code>"`, open.
fn head(s: &Snapshot, code: &str) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("{\"version\":");
    crate::json::push_u64(&mut out, s.public);
    out.push_str(",\"locale\":");
    write_str(&mut out, code);
    out
}

/// `"<16 hex digits>"`.
fn push_tag(out: &mut String, tag: u64) {
    out.push('"');
    for i in (0..16).rev() {
        let d = ((tag >> (i * 4)) & 0xf) as u32;
        out.push(char::from_digit(d, 16).unwrap_or('0'));
    }
    out.push('"');
}

/// A locale or page name inside a JSON string: escaped as [`write_str`] escapes it.
fn push_name(out: &mut String, name: &str) {
    let mut quoted = String::new();
    write_str(&mut quoted, name);
    out.push_str(quoted.get(1..quoted.len() - 1).unwrap_or(""));
}

fn write_meta(out: &mut String, s: &Snapshot) {
    let meta = &s.meta;
    out.push_str(",\"defaultLocale\":");
    write_str(out, meta.check.reference.as_deref().unwrap_or(&s.locale));
    out.push_str(",\"fallbackLocale\":");
    match meta.fallback() {
        Some(f) => write_str(out, f),
        None => out.push_str("null"),
    }
    let infos = s.locale_infos();
    out.push_str(",\"locales\":[");
    for (i, l) in infos.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"code\":");
        write_str(out, &l.code);
        for (name, value) in [
            ("iso", &l.iso),
            ("dir", &l.dir),
            ("displayName", &l.display_name),
        ] {
            if let Some(v) = value {
                out.push_str(",\"");
                out.push_str(name);
                out.push_str("\":");
                write_str(out, v);
            }
        }
        out.push('}');
    }
    out.push_str("],\"chains\":{");
    for (i, l) in infos.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_str(out, &l.code);
        out.push_str(":[");
        for (j, c) in meta.chain_codes(&l.code).into_iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write_str(out, c);
        }
        out.push(']');
    }
    let (numbers, dates) = meta.formats.json();
    out.push_str("},\"numberFormats\":");
    out.push_str(numbers);
    out.push_str(",\"datetimeFormats\":");
    out.push_str(dates);
    // the zone dates show in, when known by name: the front end formats in it too
    out.push_str(",\"timeZone\":");
    #[cfg(feature = "format")]
    let zone = meta.zone_name();
    #[cfg(not(feature = "format"))]
    let zone: Option<&str> = None;
    match zone {
        Some(z) => write_str(out, z),
        None => out.push_str("null"),
    }
}

/// `f(locale, page name, tree)` per locale of the chain of `code` and pair with data: its
/// root (with `roots`) and its `page`.
fn for_chunks<'s>(
    s: &'s Snapshot,
    code: &'s str,
    page: Option<&'s str>,
    roots: bool,
    mut f: impl FnMut(&'s str, &'s str, &'s Tree),
) {
    let resolved = s.meta.page(page);
    // `index` (or `""`) is the root: once, also when the roots are not asked for
    let page = page
        .map(|p| (resolved, if resolved.is_some() { p } else { "index" }))
        .filter(|(at, _)| at.is_some() || !roots);
    for c in s.meta.chain_codes(code) {
        let Some(d) = s.data.get(c) else { continue };
        let pairs = [roots.then_some((None, "index")), page];
        for (at, name) in pairs.into_iter().flatten() {
            if let Some(t) = tree(d, at) {
                f(c, name, t);
            }
        }
    }
}

/// The bundle's tag: a hash of the names and tags of its chunks.
fn bundle_tag(s: &Snapshot, code: &str, page: Option<&str>) -> u64 {
    let mut parts = Vec::with_capacity(64);
    for_chunks(s, code, page, true, |c, name, t| {
        parts.extend_from_slice(c.as_bytes());
        parts.push(0);
        parts.extend_from_slice(name.as_bytes());
        parts.push(0);
        parts.extend_from_slice(&t.payload().1.to_le_bytes());
    });
    content_tag(&parts)
}

/// `,"chunks":[…]}`: per locale of the chain, its root (with `roots`) and its `page`.
fn write_chunks(out: &mut String, s: &Snapshot, code: &str, page: Option<&str>, roots: bool) {
    out.push_str(",\"chunks\":[");
    let mut first = true;
    for_chunks(s, code, page, roots, |c, name, t| {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str("{\"locale\":");
        write_str(out, c);
        out.push_str(",\"page\":");
        write_str(out, name);
        out.push(',');
        // the cached `{"data":…}` without its `{`
        let json = &t.payload().0;
        out.push_str(json.get(1..).unwrap_or("}"));
    });
    out.push_str("]}");
}

fn json_error(e: crate::json::JsonError) -> Error {
    Error::Json {
        path: "payload".to_owned(),
        offset: e.offset,
        kind: e.kind.as_str(),
    }
}

/// A payload being read.
struct Payload<'a> {
    r: Reader<'a>,
    /// Stored path bytes left for the whole payload (`Tree::parse`'s bound).
    budget: usize,
}

impl<'a> Payload<'a> {
    fn next(&mut self) -> Result<Event<'a>, Error> {
        match self.r.next() {
            Ok(Some(e)) => Ok(e),
            // the reader reports the end of input as an error before running out
            Ok(None) => Err(Error::Config {
                path: "payload".to_owned(),
                reason: "unexpected end",
            }),
            Err(e) => Err(json_error(e)),
        }
    }

    /// Skips the rest of a container whose start was read.
    fn skip_rest(&mut self) -> Result<(), Error> {
        let mut depth = 1usize;
        while depth > 0 {
            match self.next()? {
                Event::ObjectStart | Event::ArrayStart => depth += 1,
                Event::ObjectEnd | Event::ArrayEnd => depth -= 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn skip_value(&mut self) -> Result<(), Error> {
        match self.next()? {
            Event::ObjectStart | Event::ArrayStart => self.skip_rest(),
            _ => Ok(()),
        }
    }

    /// One `{"locale", "page", "data", "linked"}` after its `{`; `data` is read into its
    /// tree in the same pass.
    fn chunk(&mut self) -> Result<(Arc<str>, Option<String>, Tree), Error> {
        let shape = |reason: &'static str| Error::Config {
            path: "payload".to_owned(),
            reason,
        };
        let (mut locale, mut page, mut data, mut linked) = (None, None, None, Vec::new());
        while let Event::Key(key) = self.next()? {
            match &*key {
                "locale" => match self.next()? {
                    Event::Str(s) => locale = Some(s.into_owned()),
                    _ => return Err(shape("chunks[].locale must be a string")),
                },
                "page" => match self.next()? {
                    Event::Str(s) => page = Some(s.into_owned()),
                    _ => return Err(shape("chunks[].page must be a string")),
                },
                "data" => {
                    let path = match &locale {
                        Some(l) => crate::i18n::messages_path(l, page.as_deref())
                            .replacen("messages", "payload", 1),
                        None => "payload".to_owned(),
                    };
                    match self.next()? {
                        Event::ObjectStart => {
                            data = Some(Tree::read_object(&mut self.r, &path, &mut self.budget)?);
                        }
                        Event::ArrayStart => {
                            self.skip_rest()?;
                            return Err(Error::NotObject { path });
                        }
                        _ => return Err(Error::NotObject { path }),
                    }
                }
                "linked" => {
                    if !matches!(self.next()?, Event::ObjectStart) {
                        return Err(shape("chunks[].linked must be an object"));
                    }
                    self.linked(&mut linked)?;
                }
                _ => self.skip_value()?,
            }
        }
        let (Some(locale), Some(mut data)) = (locale, data) else {
            return Err(shape("chunks[] needs locale and data"));
        };
        let page = page.filter(|p| !p.is_empty() && p != "index");
        crate::i18n::chunk_names(&locale, page.as_deref())?;
        if !linked.is_empty() {
            data.add_linked(linked);
        }
        Ok((Arc::from(locale), page, data))
    }

    /// The flat `{"key": text}` of a chunk's linked texts, after its `{`: a string is a
    /// message, a number or boolean a value, a container a subtree; `null` is nothing.
    fn linked(&mut self, out: &mut Vec<(Arc<str>, Entry)>) -> Result<(), Error> {
        while let Event::Key(key) = self.next()? {
            let entry = match self.next()? {
                Event::Str(s) => Some(Entry::Msg(Msg::new(&s))),
                Event::Num(n) => Some(Entry::Value(number_to_js_string(n).into())),
                Event::Bool(b) => Some(Entry::Value(Arc::from(if b { "true" } else { "false" }))),
                Event::ObjectStart | Event::ArrayStart => {
                    self.skip_rest()?;
                    Some(Entry::Node)
                }
                _ => None,
            };
            let Some(entry) = entry else { continue };
            // the same bound as a document's paths
            self.budget = self.budget.checked_sub(key.len()).ok_or(Error::Config {
                path: "payload".to_owned(),
                reason: "linked keys too long for the payload size",
            })?;
            if key.len() > 4096 {
                return Err(Error::Config {
                    path: "payload".to_owned(),
                    reason: "linked key longer than 4096 bytes",
                });
            }
            out.push((Arc::from(&*key), entry));
        }
        Ok(())
    }
}
