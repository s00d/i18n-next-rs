//! `Intl.NumberFormat` options of DESIGN 5.3, read from `numberFormats` (the Vue I18n shape
//! of `i18n.json` and [`Builder::formats`](crate::Builder::formats)) and resolved once, the
//! way ECMA-402 `SetNumberFormatDigitOptions` resolves them: currency digits, defaults per
//! style, `minimumFractionDigits` > `maximumFractionDigits` refused. An option outside the
//! subset, or a value `Intl` would throw on, is [`Error::UnsupportedFormat`] at `build`:
//! never a quiet difference from the front end. Unknown keys are ignored, as `Intl` ignores
//! them. Options at their default value (`notation:
//! "standard"`, `signDisplay: "auto"`…) change nothing and are accepted.

use super::number::currency_digits;
use crate::error::Error;
use crate::hash::FastMap;
use crate::json::{Event, Reader};

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
    Str(std::borrow::Cow<'a, str>),
    Num(f64),
    Bool(bool),
    Other,
}

/// Named formats by locale code, then name.
pub(crate) type Formats = FastMap<Box<str>, FastMap<Box<str>, NumberOptions>>;

/// Reads `{ "<locale>": { "<name>": { <options> } } }` into `into` (a later name wins).
/// `path` names the source in a JSON or shape error.
pub(crate) fn parse_formats<'a>(
    json: &'a str,
    path: &str,
    into: &mut Formats,
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
                    Event::ObjectStart | Event::ArrayStart => {
                        let mut depth = 1;
                        while depth > 0 {
                            match next(&mut r)? {
                                Event::ObjectStart | Event::ArrayStart => depth += 1,
                                Event::ObjectEnd | Event::ArrayEnd => depth -= 1,
                                _ => {}
                            }
                        }
                        Val::Other
                    }
                    _ => Val::Other,
                };
                raw.push((Box::from(&*key), v));
            }
            let options = resolve(&raw).map_err(|option| Error::UnsupportedFormat {
                locale: locale.to_string(),
                name: name.to_string(),
                option: option.to_owned(),
            })?;
            if &*name != "__proto__" && &*locale != "__proto__" {
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

/// ECMA-402 `GetNumberOption`: a number (or numeric string) in `lo..=hi`, floored.
fn number(v: &Val<'_>, lo: u8, hi: u8) -> Option<u8> {
    let n = match v {
        Val::Num(n) => *n,
        Val::Str(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    let n = n.floor();
    (n >= f64::from(lo) && n <= f64::from(hi)).then_some(n as u8)
}

fn string<'v>(v: &'v Val<'_>) -> Option<&'v str> {
    match v {
        Val::Str(s) => Some(s),
        _ => None,
    }
}

/// The options of one named format; `Err(option)` for the first one that is not supported.
fn resolve(raw: &[(Box<str>, Val<'_>)]) -> Result<NumberOptions, &'static str> {
    let get = |k: &str| {
        raw.iter()
            .rev()
            .find(|(key, _)| &**key == k)
            .map(|(_, v)| v)
    };
    // Options that change nothing at these values are fine; any other value is not supported.
    const DEFAULTS: [(&str, &[&str]); 8] = [
        ("signDisplay", &["auto"]),
        ("currencySign", &["standard"]),
        ("compactDisplay", &["short"]),
        ("localeMatcher", &["best fit", "lookup"]),
        ("roundingMode", &["halfExpand"]),
        ("roundingPriority", &["auto"]),
        ("trailingZeroDisplay", &["auto"]),
        ("notation", &["standard", "compact"]),
    ];
    for (key, v) in raw {
        let key = &**key;
        if let Some((name, ok)) = DEFAULTS.iter().find(|(k, _)| *k == key) {
            if !string(v).is_some_and(|s| ok.contains(&s)) {
                return Err(name);
            }
            continue;
        }
        match key {
            "style"
            | "currency"
            | "currencyDisplay"
            | "minimumFractionDigits"
            | "maximumFractionDigits"
            | "minimumIntegerDigits"
            | "useGrouping" => {}
            "roundingIncrement" if matches!(v, Val::Num(n) if *n == 1.0) => {}
            "roundingIncrement" => return Err("roundingIncrement"),
            "unit" => return Err("unit"),
            "unitDisplay" => return Err("unitDisplay"),
            "minimumSignificantDigits" => return Err("minimumSignificantDigits"),
            "maximumSignificantDigits" => return Err("maximumSignificantDigits"),
            "numberingSystem" => return Err("numberingSystem"),
            // `Intl` reads only the options it knows
            _ => {}
        }
    }
    let mut o = NumberOptions::DEFAULT;
    o.style = match get("style").map(string) {
        None => Style::Decimal,
        Some(Some("decimal")) => Style::Decimal,
        Some(Some("percent")) => Style::Percent,
        Some(Some("currency")) => Style::Currency,
        Some(_) => return Err("style"),
    };
    o.compact = get("notation").and_then(string) == Some("compact");
    if o.compact && o.style != Style::Decimal {
        return Err("notation");
    }
    if let Some(c) = get("currency") {
        let c = string(c).ok_or("currency")?;
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
    o.display = match get("currencyDisplay").map(string) {
        None | Some(Some("symbol")) => Display::Symbol,
        Some(Some("narrowSymbol")) => Display::Narrow,
        Some(Some("code")) => Display::Code,
        #[cfg(feature = "currency-names")]
        Some(Some("name")) => Display::Name,
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
    o.grouping = match get("useGrouping") {
        None => {
            if o.compact {
                Grouping::Min2
            } else {
                Grouping::Auto
            }
        }
        Some(Val::Bool(true)) => Grouping::Always,
        Some(Val::Bool(false)) => Grouping::Off,
        Some(Val::Str(s)) => match &**s {
            "always" => Grouping::Always,
            "auto" => Grouping::Auto,
            "min2" => Grouping::Min2,
            // `GetBooleanOrStringNumberFormatOption`: these strings are the default
            "true" | "false" if o.compact => Grouping::Min2,
            "true" | "false" => Grouping::Auto,
            _ => return Err("useGrouping"),
        },
        Some(_) => return Err("useGrouping"),
    };
    // ICU groups compact numbers by `min2` when asked for `auto`
    if o.compact && o.grouping == Grouping::Auto {
        o.grouping = Grouping::Min2;
    }
    Ok(o)
}
