//! `Intl.DateTimeFormat` (DESIGN 5.3, 5.4, spikes S9 / S10): the pattern of every supported
//! option combination was taken from Node's ICU by `scripts/cldr.mjs` and sits in the data
//! (`build.rs` compiled it: `\x01`, the CLDR letter, the width digit per field); this module
//! only picks the pattern and renders it. A date and a time are joined by the locale's glue
//! of the date part's width, unless ICU joins that pair otherwise (an override).
//!
//! The instant is split in the zone (`tz`) as proleptic Gregorian, as V8 sets up ICU; years
//! show as years of the era (2 BC for -1). U+202F of ICU is a space here, as in V8's
//! `format()`.

use super::data::{
    self, DATE_B, DATE_NAMES, DATE_OVER, DATE_OVER_KEYS, DATE_PATS, DATE_TIMES, Loc,
};
use std::fmt;

/// An instant for [`I18n::td`](crate::I18n::td) and [`I18n::tdr`](crate::I18n::tdr), held
/// as a JS `Date` holds it: whole milliseconds since 1970-01-01 UTC, within ±8.64e15
/// (±100 000 000 days). Anything else is an invalid date: it formats as `Invalid Date`.
///
/// From `i64` (milliseconds), `f64` (milliseconds, truncated like `new Date(ms)`; `NaN`
/// is invalid), off `wasm32-unknown-unknown` `SystemTime`; with feature `chrono`
/// `chrono::DateTime<Tz>` and `NaiveDateTime` (as UTC), with feature `time`
/// `OffsetDateTime`, `UtcDateTime` and `PrimitiveDateTime` (as UTC). Sub-millisecond
/// parts are cut toward zero, as for `SystemTime`.
///
/// ```
/// use i18n_next::Timestamp;
///
/// assert_eq!(Timestamp::from(1.9), Timestamp::from(1i64));
/// assert!(!Timestamp::from(f64::NAN).is_valid());
/// assert!(!Timestamp::from(8_640_000_000_000_001i64).is_valid());
/// assert_eq!(Timestamp::from(1_700_000_000_000i64).millis(), Some(1_700_000_000_000));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timestamp(Option<i64>);

/// ECMAScript `TimeClip`: ±8.64e15 ms.
const MAX_MS: i64 = 8_640_000_000_000_000;

impl Timestamp {
    /// The milliseconds, `None` for an invalid date.
    #[must_use]
    pub fn millis(self) -> Option<i64> {
        self.0
    }

    /// Whether the date is valid (JS `!isNaN(date)`).
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.0.is_some()
    }
}

impl From<i64> for Timestamp {
    fn from(ms: i64) -> Self {
        Timestamp((-MAX_MS..=MAX_MS).contains(&ms).then_some(ms))
    }
}

impl From<f64> for Timestamp {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the range is checked first"
    )]
    fn from(ms: f64) -> Self {
        // `TimeClip`: NaN and out of range are invalid, the rest truncated toward zero
        if ms.abs() <= MAX_MS as f64 {
            Timestamp(Some(ms.trunc() as i64))
        } else {
            Timestamp(None)
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl From<std::time::SystemTime> for Timestamp {
    fn from(t: std::time::SystemTime) -> Self {
        let ms = match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).ok(),
            // before 1970: whole milliseconds toward zero, as `Date` reads them
            Err(e) => i64::try_from(e.duration().as_millis()).ok().map(|m| -m),
        };
        ms.map_or(Timestamp(None), Timestamp::from)
    }
}

/// Whole milliseconds toward zero of nanoseconds since 1970 (as `From<SystemTime>`).
#[cfg(any(feature = "chrono", feature = "time"))]
fn from_nanos(ns: i128) -> Timestamp {
    i64::try_from(ns / 1_000_000).map_or(Timestamp(None), Timestamp::from)
}

/// The instant of a `chrono` date-time in any zone (feature `chrono`).
#[cfg(feature = "chrono")]
impl<Tz: chrono::TimeZone> From<chrono::DateTime<Tz>> for Timestamp {
    fn from(t: chrono::DateTime<Tz>) -> Self {
        from_nanos(
            i128::from(t.timestamp()) * 1_000_000_000 + i128::from(t.timestamp_subsec_nanos()),
        )
    }
}

/// A `chrono` date-time without a zone, read as UTC (feature `chrono`).
#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for Timestamp {
    fn from(t: chrono::NaiveDateTime) -> Self {
        Timestamp::from(t.and_utc())
    }
}

/// The instant of a `time` date-time with an offset (feature `time`).
#[cfg(feature = "time")]
impl From<time::OffsetDateTime> for Timestamp {
    fn from(t: time::OffsetDateTime) -> Self {
        from_nanos(t.unix_timestamp_nanos())
    }
}

/// The instant of a `time` UTC date-time (feature `time`).
#[cfg(feature = "time")]
impl From<time::UtcDateTime> for Timestamp {
    fn from(t: time::UtcDateTime) -> Self {
        from_nanos(t.unix_timestamp_nanos())
    }
}

/// A `time` date-time without an offset, read as UTC (feature `time`).
#[cfg(feature = "time")]
impl From<time::PrimitiveDateTime> for Timestamp {
    fn from(t: time::PrimitiveDateTime) -> Self {
        Timestamp::from(t.assume_utc())
    }
}

/// The time zone of a format: with the `tz` feature the crate's own (IANA through the
/// platform, `DESIGN 5.4`); without it only UTC.
#[cfg(feature = "tz")]
pub(crate) use crate::tz::Zone;

#[cfg(feature = "tz")]
use crate::tz::{Civil, civil_from_days, civil_from_unix, days_from_civil};

/// UTC alone (the `tz` feature is off).
#[cfg(not(feature = "tz"))]
#[derive(Debug, Clone)]
pub(crate) struct Zone;

#[cfg(not(feature = "tz"))]
impl Zone {
    pub(crate) fn utc() -> Zone {
        Zone
    }

    pub(crate) fn system() -> Zone {
        Zone
    }

    #[allow(clippy::unused_self, reason = "the `tz` zone's signature")]
    pub(crate) fn offset_at(&self, _: i64) -> i32 {
        0
    }

    pub(crate) fn named(_: &str) -> Option<Zone> {
        None
    }
}

/// H. Hinnant's calendar algorithms, for a build without `tz` (which has its own).
#[cfg(not(feature = "tz"))]
fn days_from_civil(y: i64, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((i64::from(m) + 9) % 12) + 2) / 5 + i64::from(d) - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Calendar fields of unix seconds (a build without `tz`).
#[cfg(not(feature = "tz"))]
#[derive(Debug, Clone, Copy)]
struct Civil {
    year: i64,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    /// 0 = Sunday
    weekday: u8,
}

#[cfg(not(feature = "tz"))]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rem_euclid"
)]
fn civil_from_unix(t: i64) -> Civil {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    Civil {
        year,
        month,
        day,
        hour: (secs / 3600) as u8,
        minute: (secs / 60 % 60) as u8,
        second: (secs % 60) as u8,
        // 1970-01-01 was a Thursday
        weekday: (days + 4).rem_euclid(7) as u8,
    }
}

#[cfg(not(feature = "tz"))]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "month and day"
)]
fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A zone by its IANA name as `Intl` takes it (`UTC`, `Etc/UTC`, `GMT` in any case are UTC),
/// `None` when the platform has no such zone (JS throws a `RangeError`).
pub(crate) fn zone_named(name: &str) -> Option<Zone> {
    const UTC: [&str; 6] = ["UTC", "Etc/UTC", "GMT", "Etc/GMT", "Etc/UCT", "UCT"];
    if UTC.iter().any(|u| u.eq_ignore_ascii_case(name)) {
        return Some(Zone::utc());
    }
    Zone::named(name)
}

/// The ISO week year (`Y`): the year of the Thursday of the date's week. Out of line: only
/// `gd`'s patterns read it, and inlined LLVM hoists its arithmetic into every render.
#[inline(never)]
fn week_year(c: &Civil) -> i64 {
    let days = days_from_civil(c.year, c.month, c.day);
    let monday_based = (i64::from(c.weekday) + 6) % 7;
    civil_from_days(days - monday_based + 3).0
}

/// The hour cycle asked for: none (the locale's, table `j`), one of `h11` `h12` `h23` `h24`
/// (tables `K` `h` `H` `k`), or `hour12` (the locale's 12- or 24-hour cycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cycle {
    Locale,
    H11,
    H12,
    H23,
    H24,
    Hour12(bool),
}

/// What a date format shows: components (a date combination `0..216`, a time combination
/// `0..27`, as `scripts/cldr.mjs` numbers them) or styles (`0..4`: full, long, medium,
/// short; [`NO_STYLE`] for none; a time style is medium or short).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Components { date: u8, time: u8 },
    Styles { date: u8, time: u8 },
}

pub(crate) const NO_STYLE: u8 = u8::MAX;

/// Resolved `Intl.DateTimeFormat` options.
#[derive(Debug, Clone)]
pub(crate) struct DateOptions {
    pub(crate) shape: Shape,
    pub(crate) cycle: Cycle,
    /// The format's own `timeZone`; else the instance's.
    pub(crate) zone: Option<Zone>,
}

impl DateOptions {
    /// `new Intl.DateTimeFormat(locale)`: year, month and day, numeric.
    pub(crate) const DEFAULT: DateOptions = DateOptions {
        shape: Shape::Components {
            date: DATE_NUMERIC,
            time: 0,
        },
        cycle: Cycle::Locale,
        zone: None,
    };
}

/// Year, month and day numeric: `((1 * 6 + 1) * 3 + 1) * 4`.
const DATE_NUMERIC: u8 = 88;

// Slots of the date row and the time row (`build.rs` `pattern_slots`).
const DATE_STYLES: usize = 216;
const GLUE: usize = 220;
const STYLE_GLUE: usize = 224;
const HOUR_TABLES: usize = 9;
const TIME_STYLES: usize = 99;

/// The glue of a date combination (ICU's pattern generator): a long month with a weekday is
/// full, a long month long, a short one medium, the rest short.
fn glue_width(date: u8) -> usize {
    match date / 12 % 6 {
        3 if !date.is_multiple_of(4) => 0,
        3 => 1,
        4 => 2,
        _ => 3,
    }
}

/// The hour table of a cycle: 0 `j`, 1 `K`, 2 `h`, 3 `H`, 4 `k`.
fn table(cycle: Cycle, loc: &Loc) -> usize {
    let letter = match cycle {
        Cycle::Locale => return 0,
        Cycle::H11 => b'K',
        Cycle::H12 => b'h',
        Cycle::H23 => b'H',
        Cycle::H24 => b'k',
        Cycle::Hour12(true) => loc.hc12,
        Cycle::Hour12(false) => loc.hc24,
    };
    match letter {
        b'K' => 1,
        b'h' => 2,
        b'H' => 3,
        _ => 4,
    }
}

/// The override of `(kind, a, b, table)` among the locale's, as `build.rs` keys them.
fn override_of(loc: &Loc, kind: u32, a: u8, b: u8, mode: usize) -> Option<&'static str> {
    let key = (kind << 20) | (u32::from(a) << 12) | (u32::from(b) << 4) | mode as u32;
    let start = loc.dover as usize;
    let keys = DATE_OVER_KEYS.get(start..start + usize::from(loc.dover_len))?;
    let i = keys.binary_search(&key).ok()?;
    DATE_OVER.get(start + i).map(|&p| data::dpat(p))
}

/// What to render: one pattern, or a glue with its date and time parts.
struct Picked {
    pattern: &'static str,
    date: &'static str,
    time: &'static str,
}

fn pick(o: &DateOptions, loc: &Loc) -> Picked {
    let row = DATE_PATS.get(usize::from(loc.dpats));
    let times = DATE_TIMES.get(usize::from(loc.dtimes));
    let p = |slot: usize| data::dpat(row.and_then(|r| r.get(slot)).copied().unwrap_or(0));
    let t = |slot: usize| data::dpat(times.and_then(|r| r.get(slot)).copied().unwrap_or(0));
    let mode = table(o.cycle, loc);
    let one = |pattern| Picked {
        pattern,
        date: "",
        time: "",
    };
    match o.shape {
        Shape::Components { date, time } => {
            let time_pattern = || match time {
                1..9 => t(usize::from(time)),
                _ => t(HOUR_TABLES + mode * 18 + usize::from(time) - 9),
            };
            match (date, time) {
                (_, 0) => one(p(usize::from(date))),
                (0, _) => one(time_pattern()),
                _ => {
                    let m = if time < 9 { 0 } else { mode };
                    match override_of(loc, 0, date, time, m) {
                        Some(full) => one(full),
                        None => Picked {
                            pattern: p(GLUE + glue_width(date)),
                            date: p(usize::from(date)),
                            time: time_pattern(),
                        },
                    }
                }
            }
        }
        Shape::Styles { date, time } => {
            let time_pattern = || t(TIME_STYLES + mode * 2 + usize::from(time.saturating_sub(2)));
            match (date, time) {
                (_, NO_STYLE) => one(p(DATE_STYLES + usize::from(date))),
                (NO_STYLE, _) => one(time_pattern()),
                _ => match override_of(loc, 1, date, time, mode) {
                    Some(full) => one(full),
                    None => Picked {
                        pattern: p(STYLE_GLUE + usize::from(date)),
                        date: p(DATE_STYLES + usize::from(date)),
                        time: time_pattern(),
                    },
                },
            }
        }
    }
}

/// What fields read besides the instant.
struct Cx {
    names: &'static [u16],
    digits: [&'static str; 10],
    /// The digits are `0`–`9` (`digits` holds them too).
    latin: bool,
    loc: &'static Loc,
}

/// Formats `t` (in `zone`, unless the options have their own) into `out`; an invalid date
/// writes `Invalid Date`, as `FormatService.formatDate`.
pub(crate) fn write(
    t: Timestamp,
    o: &DateOptions,
    loc: &'static Loc,
    zone: &Zone,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    let Some(ms) = t.0 else {
        return out.write_str("Invalid Date");
    };
    let zone = o.zone.as_ref().unwrap_or(zone);
    let secs = ms.div_euclid(1000);
    let c = civil_from_unix(secs + i64::from(zone.offset_at(secs)));
    let cx = Cx {
        names: DATE_NAMES
            .get(usize::from(loc.dnames))
            .map_or(&[][..], |r| &r[..]),
        digits: super::number::digit_table(loc),
        latin: data::s(loc.digits).is_empty(),
        loc,
    };
    let picked = pick(o, loc);
    render(picked.pattern, &c, &cx, (picked.date, picked.time), out)
}

/// Writes a compiled pattern; a glue's `{1}` is `parts.0`, `{0}` `parts.1`.
fn render(
    p: &str,
    c: &Civil,
    cx: &Cx,
    parts: (&str, &str),
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    let b = p.as_bytes();
    let mut from = 0;
    let mut i = 0;
    while i < b.len() {
        if b.get(i) != Some(&1) {
            i += 1;
            continue;
        }
        if from < i {
            out.write_str(p.get(from..i).unwrap_or(""))?;
        }
        let letter = b.get(i + 1).copied().unwrap_or(0);
        let width = b.get(i + 2).map_or(1, |w| w.wrapping_sub(b'0'));
        i += 3;
        from = i;
        match (letter, width) {
            (b'{', 1) => render(parts.0, c, cx, ("", ""), out)?,
            (b'{', _) => render(parts.1, c, cx, ("", ""), out)?,
            _ => field(letter, width, p, c, cx, out)?,
        }
    }
    match p.get(from..) {
        Some(rest) if !rest.is_empty() => out.write_str(rest),
        _ => Ok(()),
    }
}

/// `00`, `01`, … `99`: two Latin digits per write.
static PAIRS: &str = match std::str::from_utf8(&PAIR_BYTES) {
    Ok(s) => s,
    Err(_) => "",
};
const PAIR_BYTES: [u8; 200] = {
    let mut a = [0u8; 200];
    let mut i = 0;
    while i < 100 {
        a[2 * i] = b'0' + (i / 10) as u8;
        a[2 * i + 1] = b'0' + (i % 10) as u8;
        i += 1;
    }
    a
};

/// Digits of `v`, at least `width` of them, in the locale's digits.
fn number(v: u64, width: u8, cx: &Cx, out: &mut dyn fmt::Write) -> fmt::Result {
    if cx.latin && v < 10_000 && width <= 4 {
        // every calendar field but a long year: at most two writes of `PAIRS`
        let v = v as usize;
        let len = match v {
            0..10 => 1,
            10..100 => 2,
            100..1000 => 3,
            _ => 4,
        }
        .max(usize::from(width));
        let pair = |n: usize| PAIRS.get(2 * n..2 * n + 2).unwrap_or("00");
        let (hi, lo) = (pair(v / 100), pair(v % 100));
        if len > 2 {
            out.write_str(if len == 3 {
                hi.get(1..).unwrap_or("")
            } else {
                hi
            })?;
        }
        return out.write_str(if len == 1 {
            lo.get(1..).unwrap_or("")
        } else {
            lo
        });
    }
    let mut buf = [0u8; 20];
    let mut n = v;
    let mut len = 0;
    while (n > 0 || len < usize::from(width.max(1))) && len < buf.len() {
        if let Some(slot) = buf.get_mut(len) {
            *slot = (n % 10) as u8;
        }
        n /= 10;
        len += 1;
    }
    for d in buf.get(..len).unwrap_or(&[]).iter().rev() {
        out.write_str(cx.digits.get(usize::from(*d)).copied().unwrap_or("0"))?;
    }
    Ok(())
}

/// A name of the row: list `set` (`build.rs` `NAME_SETS` order), entry `i`.
fn name(cx: &Cx, set: usize, i: usize) -> &'static str {
    // months: 6 lists of 12; weekdays: 8 of 7 from 72; AM / PM and eras: 6 of 2 from 128
    let at = match set {
        0..6 => set * 12 + i,
        6..14 => 72 + (set - 6) * 7 + i,
        _ => 128 + (set - 14) * 2 + i,
    };
    data::dname(cx.names.get(at).copied().unwrap_or(0))
}

/// Width letters to a list: 3 abbreviated, 4 wide, 5 narrow, 6 short.
fn width_index(width: u8) -> usize {
    match width {
        4 => 1,
        5 => 2,
        6 => 3,
        _ => 0,
    }
}

fn field(
    letter: u8,
    width: u8,
    p: &str,
    c: &Civil,
    cx: &Cx,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    match letter {
        b'y' => {
            // the year of the era: 2 BC for -1
            let y = if c.year > 0 { c.year } else { 1 - c.year }.unsigned_abs();
            if width == 2 {
                number(y % 100, 2, cx, out)
            } else {
                number(y, width, cx, out)
            }
        }
        b'Y' => {
            let y = week_year(c);
            if y < 0 {
                out.write_str(data::s(
                    data::PATTERNS
                        .get(usize::from(cx.loc.decimal_pat))
                        .map_or(0, |pat| pat.np),
                ))?;
            }
            let a = y.unsigned_abs();
            number(if width == 2 { a % 100 } else { a }, width, cx, out)
        }
        b'M' | b'L' if width <= 2 => number(u64::from(c.month), width, cx, out),
        b'M' | b'L' => {
            let set = usize::from(letter == b'L') * 3 + width_index(width).min(2);
            out.write_str(name(cx, set, usize::from(c.month).saturating_sub(1)))
        }
        b'r' => {
            // not CLDR's `r`: a numeric month in lowercase Roman numerals (`haw`)
            const ROMAN: [&str; 12] = [
                "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii",
            ];
            out.write_str(
                ROMAN
                    .get(usize::from(c.month).saturating_sub(1))
                    .copied()
                    .unwrap_or(""),
            )
        }
        b'd' => number(u64::from(c.day), width, cx, out),
        b'E' | b'c' => {
            let set = 6 + usize::from(letter == b'c') * 4 + width_index(width);
            out.write_str(name(cx, set, usize::from(c.weekday)))
        }
        b'h' => number(
            u64::from(match c.hour % 12 {
                0 => 12,
                h => h,
            }),
            width,
            cx,
            out,
        ),
        b'H' => number(u64::from(c.hour), width, cx, out),
        b'K' => number(u64::from(c.hour % 12), width, cx, out),
        b'k' => number(
            u64::from(if c.hour == 0 { 24 } else { c.hour }),
            width,
            cx,
            out,
        ),
        b'm' => number(u64::from(c.minute), width, cx, out),
        b's' => number(u64::from(c.second), width, cx, out),
        b'a' => out.write_str(name(
            cx,
            14 + width_index(width).min(2),
            usize::from(c.hour >= 12),
        )),
        b'B' => out.write_str(flexible(p, c, cx, width)),
        b'G' => out.write_str(name(
            cx,
            17 + width_index(width).min(2),
            usize::from(c.year > 0),
        )),
        _ => Ok(()),
    }
}

/// A flexible day period (`B`, ICU): `noon` at 12:00 exactly (minutes and seconds the pattern
/// shows are zero) when the locale has one, else the period of the hour.
fn flexible(p: &str, c: &Civil, cx: &Cx, width: u8) -> &'static str {
    let Some(row) = DATE_B.get(usize::from(cx.loc.db)) else {
        return name(cx, 14, usize::from(c.hour >= 12));
    };
    let base = width_index(width).min(2) * 25;
    let shows = |l: u8| p.as_bytes().windows(2).any(|w| w == [1, l]);
    let noon = row.get(base + 24).copied().unwrap_or(0);
    let exact = (!shows(b'm') || c.minute == 0) && (!shows(b's') || c.second == 0);
    let i = if c.hour == 12 && noon != 0 && exact {
        noon
    } else {
        row.get(base + usize::from(c.hour)).copied().unwrap_or(0)
    };
    data::dname(i)
}
