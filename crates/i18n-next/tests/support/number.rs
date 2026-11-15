//! The number goldens replayed through the public API (DESIGN 7.2): `number.md` and
//! `number-props.md` (`scripts/compat.mjs`: `Intl.NumberFormat` through `FormatService`,
//! named formats through `BaseI18n.tn`) and `number-locales.md` (`scripts/cldr.mjs`: tag
//! resolution and autonyms). Shared by `tests/number_compat.rs` and `tests/wasm.rs`.
//!
//! - `num(locale, value, options)`: every distinct `options` becomes a named format of every
//!   locale (`Builder::formats`), shown with `with_locale(locale).tn_as(value, name)`;
//! - `num_name(..)`: the same with `currencyDisplay: "name"`, only with `currency-names`;
//! - `number_config()` is an `i18n.json`; `tn(code, v)` / `tn_as(code, name, v)` run on an
//!   instance of it with `code` active;
//! - `locale(tag)`: the formats of the header line, on a locale whose `iso` is `tag`;
//! - `autonym(locale)`: `language_name`.
#![allow(dead_code)]

use i18n_next::I18n;
use std::collections::BTreeMap;

pub const NUMBER: &str = include_str!("../../../../fixtures/compat/__golden__/number.md");
pub const PROPS: &str = include_str!("../../../../fixtures/compat/__golden__/number-props.md");
pub const LOCALES: &str = include_str!("../../../../fixtures/compat/__golden__/number-locales.md");

/// `JSON.stringify(s)` plus the README's invisible code units as `\uXXXX`.
fn spell(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        let u = c as u32;
        let invisible = matches!(u,
            0x7f..=0xa0 | 0xad | 0x1680 | 0x180e | 0x2000..=0x200f | 0x2028..=0x202f
            | 0x205f..=0x206f | 0x3000 | 0xfe00..=0xfe0f | 0xfeff | 0xfff9..=0xfffb);
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ if u < 0x20 || invisible => out.push_str(&format!("\\u{u:04x}")),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON string literal at the start of `s`: the text and the rest after it.
fn unspell(s: &str) -> (String, &str) {
    let b = s.as_bytes();
    assert_eq!(b[0], b'"', "string expected in {s}");
    let mut units: Vec<u16> = Vec::new();
    let mut i = 1;
    loop {
        match b[i] {
            b'"' => break,
            b'\\' => {
                i += 1;
                match b[i] {
                    b'u' => {
                        units.push(u16::from_str_radix(&s[i + 1..i + 5], 16).unwrap());
                        i += 4;
                    }
                    b'n' => units.push(10),
                    b't' => units.push(9),
                    b'r' => units.push(13),
                    c => units.push(u16::from(c)),
                }
                i += 1;
            }
            _ => {
                let c = s[i..].chars().next().unwrap();
                let mut buf = [0u16; 2];
                units.extend_from_slice(c.encode_utf16(&mut buf));
                i += c.len_utf8();
            }
        }
    }
    (String::from_utf16(&units).unwrap(), &s[i + 1..])
}

/// A number as `compat.mjs` spells it (`String(v)`, `-0`).
fn value(s: &str) -> f64 {
    match s {
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        "-0" => -0.0,
        s => s.parse().unwrap_or_else(|_| panic!("number {s}")),
    }
}

/// `"locale", value, {options}` → the three parts.
fn num_args(args: &str) -> (String, f64, &str) {
    let (locale, rest) = unspell(args);
    let rest = rest.strip_prefix(", ").unwrap();
    let (v, options) = rest.split_once(", {").unwrap();
    // the options without their braces
    (locale, value(v), &options[..options.len() - 1])
}

/// The formats of a `locale(tag)` line, in the order of the result.
const SIG: [(&str, f64); 9] = [
    ("{}", -1234567.891),
    (r#"{"style":"percent"}"#, 0.256),
    (r#"{"style":"currency","currency":"EUR"}"#, 1234.5),
    (r#"{"style":"currency","currency":"USD"}"#, -1234.5),
    (
        r#"{"style":"currency","currency":"JPY","currencyDisplay":"code"}"#,
        1234.0,
    ),
    (
        r#"{"style":"currency","currency":"USD","currencyDisplay":"narrowSymbol"}"#,
        12.0,
    ),
    (r#"{"notation":"compact"}"#, 1234567.0),
    (r#"{"notation":"compact"}"#, 1234.0),
    ("{}", f64::NAN),
];

/// The call lines of a golden: `(call, result)`.
fn lines(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|l| {
            !(l.is_empty() || l.starts_with('#') || l.starts_with('>') || l.starts_with("```"))
        })
        .map(|l| l.split_once(" → ").unwrap())
        .collect()
}

/// Replays `text`; `Err` holds every differing line (`-` golden, `+` Rust).
pub fn run(text: &str) -> Result<usize, String> {
    let lines = lines(text);
    let names = cfg!(feature = "currency-names");
    // every distinct option set, a named format of every locale of the `num` lines
    let mut options: BTreeMap<&str, usize> = BTreeMap::new();
    let mut locales: BTreeMap<String, ()> = BTreeMap::new();
    for (call, _) in &lines {
        let args = match call.split_once('(') {
            Some(("num", a)) => a,
            Some(("num_name", a)) if names => a,
            _ => continue,
        };
        let args = &args[..args.len() - 1];
        let (locale, _, o) = num_args(args);
        let next = options.len();
        options.entry(o).or_insert(next);
        locales.insert(locale, ());
    }
    let mut json = String::from("{");
    for (k, locale) in locales.keys().enumerate() {
        if k > 0 {
            json.push(',');
        }
        json.push_str(&spell(locale));
        json.push_str(":{");
        for (j, (o, n)) in options.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            json.push_str(&format!("\"o{n}\":{{{o}}}"));
        }
        json.push('}');
    }
    json.push('}');
    let all = I18n::builder()
        .formats(&json)
        .build()
        .map_err(|e| e.to_string())?;
    let mut config = None;
    let mut diff = String::new();
    let mut count = 0;
    for (call, expected) in &lines {
        let (name, args) = call.split_once('(').unwrap();
        let args = &args[..args.len() - 1];
        let got = match name {
            "num" | "num_name" => {
                if name == "num_name" && !names {
                    continue;
                }
                let (locale, v, o) = num_args(args);
                all.with_locale(&locale)
                    .tn_as(v, &format!("o{}", options[o]))
            }
            "number_config" => {
                config = Some(unspell(expected).0);
                count += 1;
                continue;
            }
            "tn" | "tn_as" => {
                let (code, rest) = unspell(args);
                let rest = rest.strip_prefix(", ").unwrap();
                let i18n = I18n::builder()
                    .config_json(config.as_deref().expect("number_config() sorts first"))
                    .locale(&code)
                    .build()
                    .map_err(|e| e.to_string())?;
                if name == "tn" {
                    i18n.tn(value(rest))
                } else {
                    let (format, rest) = unspell(rest);
                    i18n.tn_as(value(rest.strip_prefix(", ").unwrap()), &format)
                }
            }
            "locale" => {
                let (tag, _) = unspell(args);
                let mut formats = String::from(r#"{"t":{"#);
                for (k, (o, _)) in SIG.iter().enumerate() {
                    if k > 0 {
                        formats.push(',');
                    }
                    formats.push_str(&format!("\"s{k}\":{o}"));
                }
                formats.push_str("}}");
                let i18n = I18n::builder()
                    .iso("t", &tag)
                    .locale("t")
                    .formats(&formats)
                    .build()
                    .map_err(|e| e.to_string())?;
                SIG.iter()
                    .enumerate()
                    .map(|(k, (_, v))| i18n.tn_as(*v, &format!("s{k}")))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            "autonym" => {
                let (l, _) = unspell(args);
                let i18n = I18n::builder().build().map_err(|e| e.to_string())?;
                count += 1;
                let got = i18n.language_name(&l).map_or("<none>".to_owned(), spell);
                if got != *expected {
                    diff.push_str(&format!("-{call} → {expected}\n+{call} → {got}\n"));
                }
                continue;
            }
            other => panic!("unknown call {other}"),
        };
        count += 1;
        let got = spell(&got);
        if got != *expected {
            diff.push_str(&format!("-{call} → {expected}\n+{call} → {got}\n"));
        }
    }
    if diff.is_empty() {
        Ok(count)
    } else {
        Err(diff)
    }
}
