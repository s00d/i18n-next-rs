//! Where files go (`@i18n-micro/utils` `parse-path.ts`, `load-dir.ts`) and `i18n.json`
//! (DESIGN 8b.1). Paths are only ever listed and classified here, never built from a name
//! that came from outside (8b.2).

use crate::error::Error;
use crate::json::{Event, JsonError, Reader};

/// What a file is, by its path relative to the locales directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kind<'a> {
    /// `<locale>.json`
    Root(&'a str),
    /// `pages/<a>/<b>/<locale>.json` → page `a-b`
    Page(String, &'a str),
    /// Anything else: not read at all.
    Ignore,
}

/// `parseTranslationRelativePath`: `rel` uses `/`.
pub(crate) fn classify(rel: &str) -> Kind<'_> {
    let Some(stem) = rel.strip_suffix(".json") else {
        return Kind::Ignore;
    };
    // Split, not sliced: no slice panic (and its `fmt` code) in the binary.
    let (dir, locale) = match stem.rsplit_once('/') {
        Some((dir, locale)) => (Some(dir), locale),
        None => (None, stem),
    };
    if locale.is_empty() || locale == "__proto__" {
        return Kind::Ignore;
    }
    match dir.map(|d| d.strip_prefix("pages/")) {
        None => Kind::Root(locale),
        Some(Some(page)) if !page.is_empty() => {
            let page = page.replace('/', "-");
            if page == "__proto__" {
                Kind::Ignore
            } else {
                Kind::Page(page, locale)
            }
        }
        Some(_) => Kind::Ignore,
    }
}

/// JS `<` on strings: UTF-16 code units, so the file order matches the JS loader.
pub(crate) fn js_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Every `*.json` under `dir` as `(relative path with '/', full path)`, symlinks followed,
/// a link back up cut. Names that are not UTF-8 cannot be locale names and are skipped.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn list_dir(dir: &std::path::Path) -> Result<Vec<(String, std::path::PathBuf)>, Error> {
    use std::path::{Path, PathBuf};

    fn io(path: &Path, source: std::io::Error) -> Error {
        Error::Io {
            path: path.to_string_lossy().into_owned(),
            source,
        }
    }

    fn walk(
        dir: &Path,
        rel: &str,
        active: &mut Vec<PathBuf>,
        out: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), Error> {
        let real = std::fs::canonicalize(dir).map_err(|e| io(dir, e))?;
        if active.contains(&real) {
            return Ok(());
        }
        active.push(real);
        for entry in std::fs::read_dir(dir).map_err(|e| io(dir, e))? {
            let path = entry.map_err(|e| io(dir, e))?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let mut child = String::with_capacity(rel.len() + name.len() + 1);
            if !rel.is_empty() {
                child.push_str(rel);
                child.push('/');
            }
            child.push_str(name);
            if path.is_dir() {
                walk(&path, &child, active, out)?;
            } else if path.is_file() && name.ends_with(".json") {
                out.push((child, path));
            }
        }
        active.pop();
        Ok(())
    }

    let mut out = Vec::new();
    walk(dir, "", &mut Vec::new(), &mut out)?;
    Ok(out)
}

/// One `locales[]` entry of `i18n.json` (`Locale` of `@i18n-micro/types`).
#[derive(Debug, Default, Clone)]
pub(crate) struct LocaleEntry {
    pub(crate) code: String,
    pub(crate) iso: Option<String>,
    pub(crate) dir: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) fallback: Option<String>,
    pub(crate) disabled: bool,
}

/// `i18n.json`. Unknown fields are ignored: the same file feeds the Nuxt module.
#[derive(Debug, Default, Clone)]
pub(crate) struct Config {
    pub(crate) default_locale: Option<String>,
    pub(crate) fallback_locale: Option<String>,
    pub(crate) locales: Vec<LocaleEntry>,
    /// Raw JSON of `numberFormats` / `datetimeFormats`, kept for M5 / M6.
    pub(crate) number_formats: Option<Box<str>>,
    pub(crate) datetime_formats: Option<Box<str>>,
}

struct ConfigReader<'a> {
    r: Reader<'a>,
    bytes: &'a [u8],
    path: &'a str,
}

impl<'a> ConfigReader<'a> {
    fn json(&self, e: JsonError) -> Error {
        Error::Json {
            path: self.path.to_owned(),
            offset: e.offset,
            kind: e.kind.as_str(),
        }
    }

    fn shape(&self, reason: &'static str) -> Error {
        Error::Config {
            path: self.path.to_owned(),
            reason,
        }
    }

    fn next(&mut self) -> Result<Event<'a>, Error> {
        match self.r.next() {
            Ok(Some(e)) => Ok(e),
            // The reader reports the end of input as an error before running out.
            Ok(None) => Err(self.shape("unexpected end")),
            Err(e) => Err(self.json(e)),
        }
    }

    /// The rest of a value whose first event is `first`.
    fn skip(&mut self, first: &Event<'a>) -> Result<(), Error> {
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

    fn string(&mut self, reason: &'static str) -> Result<String, Error> {
        match self.next()? {
            Event::Str(s) => Ok(s.into_owned()),
            _ => Err(self.shape(reason)),
        }
    }

    /// The raw text of the next value.
    fn raw(&mut self) -> Result<Box<str>, Error> {
        let start = self.r.offset();
        let first = self.next()?;
        self.skip(&first)?;
        let text = &self.bytes[start..self.r.offset()];
        // Both ends sit at ASCII bytes of valid UTF-8.
        let text = std::str::from_utf8(text)
            .unwrap_or("")
            .trim_matches([' ', '\t', '\n', '\r']);
        Ok(text.into())
    }

    fn locale(&mut self) -> Result<LocaleEntry, Error> {
        if !matches!(self.next()?, Event::ObjectStart) {
            return Err(self.shape("locales[] must hold objects"));
        }
        let mut entry = LocaleEntry::default();
        let mut code = None;
        while let Event::Key(key) = self.next()? {
            match &*key {
                "code" => code = Some(self.string("locales[].code must be a string")?),
                "iso" => entry.iso = Some(self.string("locales[].iso must be a string")?),
                "dir" => entry.dir = Some(self.string("locales[].dir must be a string")?),
                "displayName" => {
                    entry.display_name =
                        Some(self.string("locales[].displayName must be a string")?);
                }
                "fallbackLocale" => {
                    entry.fallback =
                        Some(self.string("locales[].fallbackLocale must be a string")?);
                }
                "disabled" => match self.next()? {
                    Event::Bool(b) => entry.disabled = b,
                    _ => return Err(self.shape("locales[].disabled must be a boolean")),
                },
                _ => {
                    let first = self.next()?;
                    self.skip(&first)?;
                }
            }
        }
        match code {
            Some(c) if !c.is_empty() && c != "__proto__" => entry.code = c,
            _ => return Err(self.shape("locales[].code is missing or empty")),
        }
        Ok(entry)
    }
}

impl Config {
    /// Reads `i18n.json`; `path` names it in errors.
    pub(crate) fn parse(bytes: &[u8], path: &str) -> Result<Config, Error> {
        let mut c = ConfigReader {
            r: Reader::new(bytes),
            bytes,
            path,
        };
        if !matches!(c.next()?, Event::ObjectStart) {
            return Err(Error::NotObject {
                path: path.to_owned(),
            });
        }
        let mut config = Config::default();
        while let Event::Key(key) = c.next()? {
            match &*key {
                "defaultLocale" => {
                    config.default_locale = Some(c.string("defaultLocale must be a string")?);
                }
                "fallbackLocale" => {
                    config.fallback_locale = Some(c.string("fallbackLocale must be a string")?);
                }
                "locales" => {
                    if !matches!(c.next()?, Event::ArrayStart) {
                        return Err(c.shape("locales must be an array"));
                    }
                    loop {
                        // Peek by offset: `]` closes, anything else is an entry.
                        let rest = &bytes[c.r.offset()..];
                        let at = rest.iter().position(|b| !b" \t\n\r,".contains(b));
                        if at.is_some_and(|i| rest[i] == b']') {
                            c.next()?;
                            break;
                        }
                        config.locales.push(c.locale()?);
                    }
                }
                "numberFormats" => config.number_formats = Some(c.raw()?),
                "datetimeFormats" => config.datetime_formats = Some(c.raw()?),
                _ => {
                    let first = c.next()?;
                    c.skip(&first)?;
                }
            }
        }
        // Trailing characters are an error too.
        match c.r.next() {
            Ok(_) => Ok(config),
            Err(e) => Err(c.json(e)),
        }
    }
}
