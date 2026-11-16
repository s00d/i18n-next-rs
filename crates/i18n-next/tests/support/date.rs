//! The date goldens replayed through the public API (DESIGN 7.2): `date.md`, `relative.md`
//! and `date-props.md` (`scripts/compat.mjs`: `Intl.DateTimeFormat` through
//! `FormatService.formatDate`, `Intl.RelativeTimeFormat` through `formatRelativeTime` with a
//! fixed `now`, named formats through `BaseI18n.td` / `tdr`). Shared by
//! `tests/date_compat.rs` and `tests/wasm.rs`.
//!
//! - `date(locale, ms, options)`: each distinct `options` of a locale becomes a named format
//!   of it (`Builder::datetime_formats`), shown with `with_locale(locale).td_as(ms, name)`;
//! - `date_zone()` / `date_config()`: the zone and the `i18n.json` of `td(code, ms)` /
//!   `td_as(code, name, ms)`, run on an instance of them with `code` active;
//! - `rel(locale, at, now, options)`: `tdr_with` on an instance whose clock says `now`;
//!   `tdr(code, at, now)` the same on the `date_config()` instance.
#![allow(dead_code)]

use i18n_next::{I18n, RelativeOptions};
use std::collections::BTreeMap;

pub const DATE: &str = include_str!("../../../../fixtures/compat/__golden__/date.md");
pub const RELATIVE: &str = include_str!("../../../../fixtures/compat/__golden__/relative.md");
pub const PROPS: &str = include_str!("../../../../fixtures/compat/__golden__/date-props.md");

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

/// Milliseconds as `compat.mjs` spells them (`String(v)`).
fn value(s: &str) -> f64 {
    match s {
        "NaN" => f64::NAN,
        s => s.parse().unwrap_or_else(|_| panic!("number {s}")),
    }
}

/// `"locale", ms, {options}` → the three parts (the options with their braces).
fn date_args(args: &str) -> (String, f64, &str) {
    let (locale, rest) = unspell(args);
    let rest = rest.strip_prefix(", ").unwrap();
    let (v, options) = rest.split_once(", ").unwrap();
    (locale, value(v), options)
}

/// `RelativeOptions` of `{"numeric":…,"style":…}`.
fn relative_options(json: &str) -> RelativeOptions {
    let mut o = RelativeOptions::default();
    if json.contains(r#""numeric":"auto""#) {
        o = o.auto();
    }
    if json.contains(r#""style":"short""#) {
        o = o.short();
    }
    if json.contains(r#""style":"narrow""#) {
        o = o.narrow();
    }
    o
}

/// The call lines of a golden: `(call, result)`.
fn lines(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|l| {
            !(l.is_empty() || l.starts_with('#') || l.starts_with('>') || l.starts_with("```"))
        })
        .map(|l| l.split_once(" → ").unwrap())
        .collect()
}

/// `$TZDIR` (searched before the system zoneinfo, and on Windows before the bundled tzdb, which
/// has no history before 1970: src/tz/mod.rs `read`) for the goldens:
/// TZif files of jiff-tzdb, the IANA release of `src/tz/tzdb.bin` (tests/tz_tzdb.rs), written
/// under `CARGO_TARGET_TMPDIR`. The goldens hold Node's zone data; the system's is whatever
/// the OS ships: macOS runners had a tzdata from before Morocco's 2026 change (+01 with a
/// Ramadan +00, precomputed to 2087) where the goldens say +00, so Africa/Casablanca after
/// 2026 was an hour off there. The crate forbids `unsafe`, so no `set_var`: the test runs
/// again in a child process with the variable set. `true` in that child; in the parent the
/// child has run `test` and passed (else this panics with its output), and the test returns.
#[cfg(not(target_arch = "wasm32"))]
pub fn zones_pinned(test: &str) -> bool {
    use std::path::Path;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("zoneinfo-golden");
    if std::env::var_os("TZDIR").is_some_and(|d| d == dir.as_os_str()) {
        return true;
    }
    let tmp = format!("tmp.{}.{test}", std::process::id());
    for name in jiff_tzdb::available() {
        let (_, bytes) = jiff_tzdb::get(name).unwrap();
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // other tests (threads here, other processes under nextest) write and read these
        // files too: each writes its own temporary file and replaces in one rename
        let part = path.with_extension(&tmp);
        std::fs::write(&part, bytes).unwrap();
        std::fs::rename(&part, &path).unwrap();
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env("TZDIR", &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test} with TZDIR={}:\n{stdout}\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// Replays `text`; `Err` holds every differing line (`-` golden, `+` Rust).
pub fn run(text: &str) -> Result<usize, String> {
    let lines = lines(text);
    // every distinct option set of a locale, a named format of it
    let mut named: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for (call, _) in &lines {
        if let Some(args) = call.strip_prefix("date(") {
            let (locale, _, o) = date_args(&args[..args.len() - 1]);
            let list = named.entry(locale).or_default();
            let next = list.len();
            list.entry(o).or_insert(next);
        }
    }
    let mut json = String::from("{");
    for (k, (locale, list)) in named.iter().enumerate() {
        if k > 0 {
            json.push(',');
        }
        json.push_str(&spell(locale));
        json.push_str(":{");
        for (j, (o, n)) in list.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            json.push_str(&format!("\"o{n}\":{o}"));
        }
        json.push('}');
    }
    json.push('}');
    let all = I18n::builder()
        .datetime_formats(&json)
        .build()
        .map_err(|e| e.to_string())?;
    let (mut zone, mut config) = (None, None);
    let mut clocks: BTreeMap<i64, I18n> = BTreeMap::new();
    let mut diff = String::new();
    let mut count = 0;
    for (call, expected) in &lines {
        let (name, args) = call.split_once('(').unwrap();
        let args = &args[..args.len() - 1];
        let got = match name {
            "date" => {
                let (locale, v, o) = date_args(args);
                all.with_locale(&locale)
                    .td_as(v, &format!("o{}", named[&locale][o]))
            }
            "date_zone" => {
                zone = Some(unspell(expected).0);
                count += 1;
                continue;
            }
            "date_config" => {
                config = Some(unspell(expected).0);
                count += 1;
                continue;
            }
            "td" | "td_as" => {
                let (code, rest) = unspell(args);
                let rest = rest.strip_prefix(", ").unwrap();
                let i18n = I18n::builder()
                    .config_json(config.as_deref().expect("date_config() sorts first"))
                    .time_zone(zone.as_deref().expect("date_zone() sorts first"))
                    .locale(&code)
                    .build()
                    .map_err(|e| e.to_string())?;
                if name == "td" {
                    i18n.td(value(rest))
                } else {
                    let (format, rest) = unspell(rest);
                    i18n.td_as(value(rest.strip_prefix(", ").unwrap()), &format)
                }
            }
            "rel" => {
                let (locale, rest) = unspell(args);
                let mut parts = rest.strip_prefix(", ").unwrap().splitn(3, ", ");
                let at = value(parts.next().unwrap());
                let now: i64 = parts.next().unwrap().parse().unwrap();
                let o = relative_options(parts.next().unwrap());
                let i18n = clocks
                    .entry(now)
                    .or_insert_with(|| I18n::builder().clock(move || now).build().unwrap());
                i18n.with_locale(&locale).tdr_with(at, o)
            }
            "tdr" => {
                let (code, rest) = unspell(args);
                let mut parts = rest.strip_prefix(", ").unwrap().split(", ");
                let at = value(parts.next().unwrap());
                let now: i64 = parts.next().unwrap().parse().unwrap();
                let i18n = I18n::builder()
                    .config_json(config.as_deref().expect("date_config() sorts first"))
                    .clock(move || now)
                    .locale(&code)
                    .build()
                    .map_err(|e| e.to_string())?;
                i18n.tdr(at)
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
