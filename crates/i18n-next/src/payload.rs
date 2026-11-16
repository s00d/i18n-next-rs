//! The payload the Tauri plugin sends the front end and a Rust guest applies (DESIGN 2.1,
//! 6.1, 8b.1): one format, one code for both sides. Feature `payload` (`tauri-guest`
//! turns it on).
//!
//! A payload is one JSON object:
//!
//! ```json
//! {"version":3,"locale":"de",
//!  "defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"de","iso":"de-DE"}],
//!  "numberFormats":{},"datetimeFormats":{},
//!  "chunks":[{"locale":"de","page":"index","data":{"nav":{"home":"Start"}}}]}
//! ```
//!
//! - `version`: the snapshot the chunks come from ([`I18n::version`]);
//! - `locale`: the locale the chunks are for (the active one unless asked otherwise);
//! - `defaultLocale` … `datetimeFormats`: only with `meta` (the first `load`), what
//!   `createI18n` needs (8b.1); `fallbackLocale` is `null` when none is set;
//! - `chunks`: the layers of the locale's fallback chain, strongest first, each the
//!   nested tree JS `addRouteTranslations(locale, page, data)` takes; the root is page
//!   `index`. A pair without data is left out.
//!
//! Chunk trees are written from the loaded catalog (merged files, `extend`, `messages`,
//! resolved linked messages), keys sorted: the same catalog gives the same bytes.

use crate::catalog::Tree;
use crate::error::Error;
use crate::i18n::{I18n, LocaleData, Snapshot};
use crate::json::{Event, Reader, write_str};
use crate::lazy::Layer;
use std::sync::Arc;

/// The payload of the `locale` fallback chain (the active locale when `None`): its roots
/// and, with `page`, its page chunks; with `meta`, the locale list, `defaultLocale` and the
/// named formats too. Returns the snapshot version and the JSON bytes. What the plugin's
/// `load` (`meta: true`) and the protocol's bundle (`meta: false`) send. Names are not
/// checked here: the plugin accepts only known ones (8b.2); an unknown locale reads the
/// global fallback, as everywhere.
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
        (s.version, out.into_bytes())
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
        (s.version, out.into_bytes())
    })
}

/// One `(locale, page)` chunk as its nested JSON object (`{}` when it has no data), with
/// the snapshot version: the protocol's `/v{version}/{locale}/{page}.json`. `None`,
/// `""` and `"index"` are the root; an alias reads its target.
///
/// ```
/// use i18n_next::{I18n, payload};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"list": ["a", {"b": true}], "n": 1.50, "x": null}"#)
///     .build()?;
/// let (_, bytes) = payload::chunk(&i18n, "en", None);
/// assert_eq!(bytes, br#"{"list":["a",{"b":true}],"n":1.5,"x":null}"#);
/// assert_eq!(payload::chunk(&i18n, "de", Some("x")).1, b"{}");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn chunk(i18n: &I18n, locale: &str, page: Option<&str>) -> (u64, Vec<u8>) {
    i18n.inner.read(|s| {
        let mut out = String::new();
        let page = s.meta.page(page);
        match s.data.get(locale).and_then(|d| tree(d, page)) {
            Some(t) => t.write_json(&mut out),
            None => out.push_str("{}"),
        }
        (s.version, out.into_bytes())
    })
}

/// Applies a payload to `i18n` (a Rust guest, DESIGN 2.1): every chunk replaces its
/// `(locale, page)` pair, and the payload's locale becomes the active one unless it is
/// disabled; one new snapshot, subscribers notified. Returns the payload's `version` (0
/// when it has none). Meta fields are not applied here: a guest builds its instance from
/// them.
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
/// assert_eq!((&*guest.locale(), guest.t("hi")), ("de", "Hallo".to_owned()));
/// assert!(payload::apply(&guest, b"[]").is_err());
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// # Errors
/// [`Error::Json`] / [`Error::NotObject`] (path `payload`, or `payload:<locale>/<page>`
/// for a chunk) for broken JSON, [`Error::Config`] for a field of the wrong type,
/// [`Error::UnknownLocale`] / [`Error::UnknownPage`] for an empty or `__proto__` name.
/// Nothing changes then.
pub fn apply(i18n: &I18n, bytes: &[u8]) -> Result<u64, Error> {
    let shape = |reason: &'static str| Error::Config {
        path: "payload".to_owned(),
        reason,
    };
    let mut r = Payload {
        r: Reader::new(bytes),
        bytes,
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
                        Event::ObjectStart => chunks.push(r.chunk()?),
                        _ => return Err(shape("chunks[] must hold objects")),
                    }
                }
            }
            _ => r.skip_value()?,
        }
    }
    // trailing bytes are an error too
    r.r.next().map_err(|e| r.json(e))?;
    if let Some(l) = &locale {
        crate::i18n::chunk_names(l, None)?;
    }
    i18n.replace_chunks(chunks, locale.as_deref())?;
    Ok(version)
}

/// Named formats of `i18n.json` and then of the builder's calls, merged by name: the
/// `numberFormats` / `datetimeFormats` objects of the payload.
pub(crate) fn formats_json(
    b: &crate::i18n::Builder,
    config: &crate::loader::Config,
) -> Result<(Box<str>, Box<str>), Error> {
    let merged = |from_config: Option<&str>, calls: &[String], name: &str| {
        let mut acc = Tree::default();
        let texts = from_config
            .map(|t| ("i18n.json", t))
            .into_iter()
            .chain(calls.iter().map(|t| (name, t.as_str())));
        for (path, text) in texts {
            acc.merge_replacing(Tree::parse(text.as_bytes(), path)?, 2);
        }
        let mut out = String::new();
        acc.write_json(&mut out);
        Ok::<Box<str>, Error>(out.into())
    };
    #[cfg(feature = "format")]
    let (numbers, dates) = (&b.formats[..], &b.datetime_formats[..]);
    #[cfg(not(feature = "format"))]
    let (numbers, dates): (&[String], &[String]) = {
        let _ = b;
        (&[], &[])
    };
    Ok((
        merged(config.number_formats.as_deref(), numbers, "formats")?,
        merged(
            config.datetime_formats.as_deref(),
            dates,
            "datetime_formats",
        )?,
    ))
}

/// The loaded root (`None`) or page tree of one locale.
fn tree<'a>(d: &'a LocaleData, page: Option<&str>) -> Option<&'a Tree> {
    d.layer(page).and_then(Layer::chunk)
}

/// `{"version":N,"locale":"<code>"`, open.
fn head(s: &Snapshot, code: &str) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("{\"version\":");
    crate::json::push_u64(&mut out, s.version);
    out.push_str(",\"locale\":");
    write_str(&mut out, code);
    out
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
    out.push_str(",\"locales\":[");
    for (i, l) in s.locale_infos().iter().enumerate() {
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
    out.push_str("],\"numberFormats\":");
    out.push_str(&meta.formats_json.0);
    out.push_str(",\"datetimeFormats\":");
    out.push_str(&meta.formats_json.1);
}

/// `,"chunks":[…]}`: per locale of the chain, its root (with `roots`) and its `page`.
fn write_chunks(out: &mut String, s: &Snapshot, code: &str, page: Option<&str>, roots: bool) {
    out.push_str(",\"chunks\":[");
    let resolved = s.meta.page(page);
    // `index` (or `""`) is the root: once, also when the roots are not asked for
    let page = page
        .map(|p| (resolved, if resolved.is_some() { p } else { "index" }))
        .filter(|(at, _)| at.is_some() || !roots);
    let mut first = true;
    for c in s.meta.chain_codes(code) {
        let Some(d) = s.data.get(c) else { continue };
        let pairs = [roots.then_some((None, "index")), page];
        for (at, name) in pairs.into_iter().flatten() {
            let Some(t) = tree(d, at) else { continue };
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("{\"locale\":");
            write_str(out, c);
            out.push_str(",\"page\":");
            write_str(out, name);
            out.push_str(",\"data\":");
            t.write_json(out);
            out.push('}');
        }
    }
    out.push_str("]}");
}

/// A payload being read.
struct Payload<'a> {
    r: Reader<'a>,
    bytes: &'a [u8],
}

impl<'a> Payload<'a> {
    fn json(&self, e: crate::json::JsonError) -> Error {
        Error::Json {
            path: "payload".to_owned(),
            offset: e.offset,
            kind: e.kind.as_str(),
        }
    }

    fn next(&mut self) -> Result<Event<'a>, Error> {
        match self.r.next() {
            Ok(Some(e)) => Ok(e),
            // the reader reports the end of input as an error before running out
            Ok(None) => Err(Error::Config {
                path: "payload".to_owned(),
                reason: "unexpected end",
            }),
            Err(e) => Err(self.json(e)),
        }
    }

    fn skip_value(&mut self) -> Result<(), Error> {
        let first = self.next()?;
        let mut depth = usize::from(matches!(first, Event::ObjectStart | Event::ArrayStart));
        while depth > 0 {
            match self.next()? {
                Event::ObjectStart | Event::ArrayStart => depth += 1,
                Event::ObjectEnd | Event::ArrayEnd => depth -= 1,
                _ => {}
            }
        }
        Ok(())
    }

    /// One `{"locale", "page", "data"}` after its `{`.
    fn chunk(&mut self) -> Result<(Arc<str>, Option<String>, Tree), Error> {
        let shape = |reason: &'static str| Error::Config {
            path: "payload".to_owned(),
            reason,
        };
        let (mut locale, mut page, mut data) = (None, None, None);
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
                    let start = self.r.offset();
                    self.skip_value()?;
                    data = self.bytes.get(start..self.r.offset());
                }
                _ => self.skip_value()?,
            }
        }
        let (Some(locale), Some(data)) = (locale, data) else {
            return Err(shape("chunks[] needs locale and data"));
        };
        let page = page.filter(|p| !p.is_empty() && p != "index");
        crate::i18n::chunk_names(&locale, page.as_deref())?;
        let path = crate::i18n::messages_path(&locale, page.as_deref());
        let path = path.replacen("messages", "payload", 1);
        let tree = Tree::parse(data, &path)?;
        Ok((Arc::from(locale), page, tree))
    }
}
