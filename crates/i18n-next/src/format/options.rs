//! `Intl.NumberFormat` options of DESIGN 5.3, read from `numberFormats` (the Vue I18n shape
//! of `i18n.json` and [`Builder::formats`](crate::Builder::formats)) and resolved once, the
//! way ECMA-402 `SetNumberFormatDigitOptions` resolves them: currency digits, defaults per
//! style, `minimumFractionDigits` > `maximumFractionDigits` refused. An option outside the
//! subset, or a value `Intl` would throw on, is [`Error::UnsupportedFormat`] at `build`:
//! never a quiet difference from the front end. Unknown keys are ignored, as `Intl` ignores
//! them. Options at their default value (`notation:
//! "standard"`, `signDisplay: "auto"`, `numberingSystem` of the locale…) change nothing and are
//! accepted. Values are read as `Intl` reads them: `ToNumber` for digits (`"0x2"`, `null` → 0,
//! `[2]`), `ToString` for strings, `ToBoolean` first for `useGrouping` (`0`, `""` → off); a key
//! given twice takes its last value.

use super::data::{self, Loc};
use super::number::currency_digits;
use crate::catalog::is_js_space;
use crate::error::Error;
use crate::hash::FastMap;
use crate::json::{Event, Reader, push_js_number};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Decimal,
    Percent,
    Currency,
}

/// `currencyDisplay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Display {
    Symbol,
    Narrow,
    Code,
    /// The plural display name (`{0} {1}` of the locale); feature `currency-names`.
    #[cfg_attr(not(feature = "currency-names"), allow(dead_code))]
    Name,
}

/// `useGrouping`: `"auto"` (the locale's `minimumGroupingDigits`), `"min2"`, `"always"`
/// (`true`), off (`false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grouping {
    Auto,
    Min2,
    Always,
    Off,
}

/// Resolved options: what the formatter reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberOptions {
    pub(crate) style: Style,
    pub(crate) currency: [u8; 3],
    pub(crate) display: Display,
    pub(crate) min_frac: u8,
    pub(crate) max_frac: u8,
    pub(crate) min_int: u8,
    pub(crate) grouping: Grouping,
    pub(crate) compact: bool,
    /// Compact with fraction digits given: rounds by them, else to two significant digits.
    pub(crate) frac_given: bool,
}

impl NumberOptions {
    /// `new Intl.NumberFormat(locale)`.
    pub(crate) const DEFAULT: NumberOptions = NumberOptions {
        style: Style::Decimal,
        currency: *b"XXX",
        display: Display::Symbol,
        min_frac: 0,
        max_frac: 3,
        min_int: 1,
        grouping: Grouping::Auto,
        compact: false,
        frac_given: false,
    };
}

/// One raw option value.
enum Val<'a> {
    Str(Cow<'a, str>),
    Num(f64),
    Bool(bool),
    Null,
    /// An array, as JS `ToString` gives it (`[2]` → `"2"`, `[]` → `""`, `[1, [2]]` → `"1,2"`).
    Arr(String),
    Obj,
}

impl Val<'_> {
    /// ECMAScript `ToString`.
    fn string(&self) -> Cow<'_, str> {
        match self {
            Val::Str(s) => Cow::Borrowed(&**s),
            Val::Num(n) => {
                let mut out = String::new();
                push_js_number(&mut out, *n);
                Cow::Owned(out)
            }
            Val::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Val::Null => Cow::Borrowed("null"),
            Val::Arr(s) => Cow::Borrowed(s),
            Val::Obj => Cow::Borrowed("[object Object]"),
        }
    }

    /// ECMAScript `ToNumber`.
    fn number(&self) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Str(s) => js_number(s),
            Val::Arr(s) => js_number(s),
            Val::Bool(b) => f64::from(u8::from(*b)),
            Val::Null => 0.0,
            Val::Obj => f64::NAN,
        }
    }

    /// ECMAScript `ToBoolean`.
    fn truthy(&self) -> bool {
        match self {
            Val::Str(s) => !s.is_empty(),
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Bool(b) => *b,
            Val::Null => false,
            Val::Arr(_) | Val::Obj => true,
        }
    }
}

/// ECMAScript `StringToNumber`: JS white space trimmed, empty → 0, `Infinity`, decimal
/// literals (`.5`, `5.`, `1e0`, signed), `0x` / `0o` / `0b` integers (unsigned); else `NaN`.
fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(is_js_space);
    if t.is_empty() {
        return 0.0;
    }
    let radix = match t.get(..2) {
        Some("0x" | "0X") => 16,
        Some("0o" | "0O") => 8,
        Some("0b" | "0B") => 2,
        _ => 10,
    };
    if radix != 10 {
        let digits = t.get(2..).unwrap_or("");
        if digits.is_empty() {
            return f64::NAN;
        }
        let mut v = 0.0;
        for c in digits.chars() {
            match c.to_digit(radix) {
                Some(d) => v = v * f64::from(radix) + f64::from(d),
                None => return f64::NAN,
            }
        }
        return v;
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    if body == "Infinity" {
        return if t.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // Rust also reads `inf` and `nan`: only digits, `.` and an exponent go to it
    if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || !body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// An array's `ToString` from the reader, after its `[`.
fn array_string<'a>(r: &mut Reader<'a>, path: &str) -> Result<String, Error> {
    let mut out = String::new();
    let mut first = true;
    loop {
        let e = next_event(r, path)?;
        if matches!(e, Event::ArrayEnd) {
            return Ok(out);
        }
        if !first {
            out.push(',');
        }
        first = false;
        match e {
            Event::Str(s) => out.push_str(&s),
            Event::Num(n) => push_js_number(&mut out, n.parse().unwrap_or(f64::NAN)),
            Event::Bool(b) => out.push_str(if b { "true" } else { "false" }),
            Event::ArrayStart => out.push_str(&array_string(r, path)?),
            Event::ObjectStart => {
                skip_object(r, path)?;
                out.push_str("[object Object]");
            }
            // `null` (and `undefined`) join as nothing
            _ => {}
        }
    }
}

/// Skips the rest of an object, after its `{`.
fn skip_object(r: &mut Reader<'_>, path: &str) -> Result<(), Error> {
    let mut depth = 1;
    while depth > 0 {
        match next_event(r, path)? {
            Event::ObjectStart | Event::ArrayStart => depth += 1,
            Event::ObjectEnd | Event::ArrayEnd => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}

/// Named formats by locale code, then name.
pub(crate) type Formats = FastMap<Box<str>, FastMap<Box<str>, NumberOptions>>;

/// A `numberingSystem` as given (lowercase, zero-padded), by locale code and format name:
/// accepted only when it is the locale's default, checked once the locales' `iso` are known
/// ([`check_numbering`]). Kept out of [`NumberOptions`], which every format call reads.
pub(crate) type Numbering = Vec<(Box<str>, Box<str>, [u8; 8])>;

/// Reads `{ "<locale>": { "<name>": { <options> } } }` into `into` (a later name wins), the
/// `numberingSystem`s given into `numbering`. `path` names the source in a JSON or shape error.
pub(crate) fn parse_formats<'a>(
    json: &'a str,
    path: &str,
    into: &mut Formats,
    numbering: &mut Numbering,
) -> Result<(), Error> {
    let mut r = Reader::new(json.as_bytes());
    let shape = |reason| Error::Config {
        path: path.to_owned(),
        reason,
    };
    let next = |r: &mut Reader<'a>| next_event(r, path);
    if !matches!(next(&mut r)?, Event::ObjectStart) {
        return Err(shape("numberFormats must be an object"));
    }
    while let Event::Key(locale) = next(&mut r)? {
        if !matches!(next(&mut r)?, Event::ObjectStart) {
            return Err(shape("numberFormats.<locale> must be an object"));
        }
        while let Event::Key(name) = next(&mut r)? {
            if !matches!(next(&mut r)?, Event::ObjectStart) {
                return Err(shape("numberFormats.<locale>.<name> must be an object"));
            }
            let mut raw: Vec<(Box<str>, Val<'_>)> = Vec::new();
            while let Event::Key(key) = next(&mut r)? {
                let v = match next(&mut r)? {
                    Event::Str(s) => Val::Str(s),
                    Event::Num(n) => Val::Num(n.parse().unwrap_or(f64::NAN)),
                    Event::Bool(b) => Val::Bool(b),
                    Event::ArrayStart => Val::Arr(array_string(&mut r, path)?),
                    Event::ObjectStart => {
                        skip_object(&mut r, path)?;
                        Val::Obj
                    }
                    _ => Val::Null,
                };
                // a key given again: its last value, as a JS object has it
                raw.retain(|(k, _)| **k != *key);
                raw.push((Box::from(&*key), v));
            }
            let (options, nu) = resolve(&raw).map_err(|option| Error::UnsupportedFormat {
                locale: locale.to_string(),
                name: name.to_string(),
                option: option.to_owned(),
            })?;
            if &*name != "__proto__" && &*locale != "__proto__" {
                numbering.retain(|(l, n, _)| (&**l, &**n) != (&*locale, &*name));
                if let Some(nu) = nu {
                    numbering.push((Box::from(&*locale), Box::from(&*name), nu));
                }
                into.entry(Box::from(&*locale))
                    .or_default()
                    .insert(Box::from(&*name), options);
            }
        }
    }
    match r.next() {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::Json {
            path: path.to_owned(),
            offset: e.offset,
            kind: e.kind.as_str(),
        }),
    }
}

/// Refuses a `numberingSystem` other than the default of its format's locale (`loc`: the
/// data locale of a locale code, by its `iso`): ICU would format with it, this crate formats
/// with the default digits only. The first refused format by locale, then name.
pub(crate) fn check_numbering(
    numbering: &Numbering,
    loc: impl Fn(&str) -> &'static Loc,
) -> Result<(), Error> {
    let mut refused: Option<(&str, &str)> = None;
    for (locale, name, nu) in numbering {
        let given = nu.split(|&b| b == 0).next().unwrap_or(&[]);
        if given == data::numbering(loc(locale).digits).as_bytes() {
            continue;
        }
        if refused.is_none_or(|r| (&**locale, &**name) < r) {
            refused = Some((locale, name));
        }
    }
    match refused {
        Some((locale, name)) => Err(Error::UnsupportedFormat {
            locale: locale.to_owned(),
            name: name.to_owned(),
            option: "numberingSystem".to_owned(),
        }),
        None => Ok(()),
    }
}

/// The next event; the end of input or a syntax error is an error naming `path`.
fn next_event<'a>(r: &mut Reader<'a>, path: &str) -> Result<Event<'a>, Error> {
    match r.next() {
        Ok(Some(e)) => Ok(e),
        Ok(None) => Err(Error::Config {
            path: path.to_owned(),
            reason: "unexpected end",
        }),
        Err(e) => Err(Error::Json {
            path: path.to_owned(),
            offset: e.offset,
            kind: e.kind.as_str(),
        }),
    }
}

/// ECMA-402 `DefaultNumberOption`: `ToNumber` in `lo..=hi` (`NaN` and out of range: `None`),
/// floored.
fn number(v: &Val<'_>, lo: u8, hi: u8) -> Option<u8> {
    let n = v.number();
    (n >= f64::from(lo) && n <= f64::from(hi)).then(|| n.floor() as u8)
}

/// The options of one named format; `Err(option)` for the first one that is not supported.
/// With the `numberingSystem` given, for [`check_numbering`].
fn resolve(raw: &[(Box<str>, Val<'_>)]) -> Result<(NumberOptions, Option<[u8; 8]>), &'static str> {
    let get = |k: &str| raw.iter().find(|(key, _)| &**key == k).map(|(_, v)| v);
    let string = |k: &str| get(k).map(Val::string);
    // Options that change nothing at these values are fine; any other value is not supported.
    const DEFAULTS: [(&str, &[&str]); 7] = [
        ("signDisplay", &["auto"]),
        ("currencySign", &["standard"]),
        ("localeMatcher", &["best fit", "lookup"]),
        ("roundingMode", &["halfExpand"]),
        ("roundingPriority", &["auto"]),
        ("trailingZeroDisplay", &["auto"]),
        ("notation", &["standard", "compact"]),
    ];
    for (key, v) in raw {
        let key = &**key;
        if let Some((name, ok)) = DEFAULTS.iter().find(|(k, _)| *k == key) {
            if !ok.contains(&&*v.string()) {
                return Err(name);
            }
            continue;
        }
        match key {
            // `GetNumberOption(…, 1, 5000)`, floored: only 1 rounds as without it
            "roundingIncrement" if (1.0..2.0).contains(&v.number()) => {}
            "roundingIncrement" => return Err("roundingIncrement"),
            "unit" => return Err("unit"),
            "unitDisplay" => return Err("unitDisplay"),
            "minimumSignificantDigits" => return Err("minimumSignificantDigits"),
            "maximumSignificantDigits" => return Err("maximumSignificantDigits"),
            // `Intl` reads only the options it knows; the others are below
            _ => {}
        }
    }
    let mut o = NumberOptions::DEFAULT;
    o.style = match string("style").as_deref() {
        None | Some("decimal") => Style::Decimal,
        Some("percent") => Style::Percent,
        Some("currency") => Style::Currency,
        Some(_) => return Err("style"),
    };
    o.compact = string("notation").as_deref() == Some("compact");
    if o.compact && o.style != Style::Decimal {
        return Err("notation");
    }
    // read whatever the notation (a bad value throws), used only by compact
    match string("compactDisplay").as_deref() {
        None | Some("short") => {}
        Some("long") if !o.compact => {}
        Some(_) => return Err("compactDisplay"),
    }
    let mut numbering = None;
    if let Some(nu) = string("numberingSystem") {
        // the `type` subtag of a Unicode locale id (3..8 alphanumerics), else `Intl` throws
        let b = nu.as_bytes();
        if !(3..=8).contains(&b.len()) || !b.iter().all(u8::is_ascii_alphanumeric) {
            return Err("numberingSystem");
        }
        let mut given = [0u8; 8];
        for (slot, c) in given.iter_mut().zip(b) {
            *slot = c.to_ascii_lowercase();
        }
        numbering = Some(given);
    }
    if let Some(c) = string("currency") {
        let b = c.as_bytes();
        if b.len() != 3 || !b.iter().all(u8::is_ascii_alphabetic) {
            return Err("currency");
        }
        o.currency = [
            b[0].to_ascii_uppercase(),
            b[1].to_ascii_uppercase(),
            b[2].to_ascii_uppercase(),
        ];
    } else if o.style == Style::Currency {
        return Err("currency");
    }
    o.display = match string("currencyDisplay").as_deref() {
        None | Some("symbol") => Display::Symbol,
        Some("narrowSymbol") => Display::Narrow,
        Some("code") => Display::Code,
        #[cfg(feature = "currency-names")]
        Some("name") => Display::Name,
        Some(_) => return Err("currencyDisplay"),
    };
    if let Some(v) = get("minimumIntegerDigits") {
        o.min_int = number(v, 1, 21).ok_or("minimumIntegerDigits")?;
    }
    let (min_default, max_default) = match o.style {
        Style::Decimal => (0, 3),
        Style::Percent => (0, 0),
        Style::Currency => {
            let d = currency_digits(o.currency);
            (d, d)
        }
    };
    let min = get("minimumFractionDigits")
        .map(|v| number(v, 0, 100).ok_or("minimumFractionDigits"))
        .transpose()?;
    let max = get("maximumFractionDigits")
        .map(|v| number(v, 0, 100).ok_or("maximumFractionDigits"))
        .transpose()?;
    o.frac_given = min.is_some() || max.is_some();
    (o.min_frac, o.max_frac) = match (min, max) {
        (None, None) => (min_default, max_default),
        (None, Some(max)) => (min_default.min(max), max),
        (Some(min), None) => (min, max_default.max(min)),
        (Some(min), Some(max)) if min > max => return Err("maximumFractionDigits"),
        (Some(min), Some(max)) => (min, max),
    };
    // `GetBooleanOrStringNumberFormatOption`: `true` → "always", falsy → off, else the
    // string, where "true" / "false" mean the default
    let fallback = if o.compact {
        Grouping::Min2
    } else {
        Grouping::Auto
    };
    o.grouping = match get("useGrouping") {
        None => fallback,
        Some(Val::Bool(true)) => Grouping::Always,
        Some(v) if !v.truthy() => Grouping::Off,
        Some(v) => match &*v.string() {
            "always" => Grouping::Always,
            "auto" => Grouping::Auto,
            "min2" => Grouping::Min2,
            "true" | "false" => fallback,
            _ => return Err("useGrouping"),
        },
    };
    // ICU groups compact numbers by `min2` when asked for `auto`
    if o.compact && o.grouping == Grouping::Auto {
        o.grouping = Grouping::Min2;
    }
    Ok((o, numbering))
}
