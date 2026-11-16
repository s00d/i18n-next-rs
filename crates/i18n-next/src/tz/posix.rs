//! POSIX `TZ` rules (POSIX.1-2017 8.3 with the IANA v3 extension of RFC 8536 3.3.1: rule
//! times in `-167..=167` hours), as found in TZif footers and in the `TZ` variable:
//! `std offset [dst [offset] ,start[/time],end[/time]]`, names plain (`CET`) or quoted
//! (`<+0330>`), dates `Jn` (1..=365, no Feb 29), `n` (0..=365, with it) or `Mm.w.d`.
//!
//! The evaluation follows `jiff` (the oracle of `tests/tz_*.rs`) to the second: the year is
//! the UTC year of the instant, each transition is clamped into that year, and a rule that
//! starts on Jan 1 at 00:00 and never leaves the year is permanent DST.

use super::days_from_civil;

/// A parsed rule. Offsets are seconds east of UTC (POSIX writes them west: `EST5` is −5 h).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Posix {
    pub(crate) std: i32,
    pub(crate) dst: Option<Dst>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dst {
    pub(crate) offset: i32,
    pub(crate) start: DayTime,
    pub(crate) end: DayTime,
}

/// A transition: a day of the year and a wall-clock time in seconds (may be negative or
/// past 24 h, v3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DayTime {
    pub(crate) day: Day,
    pub(crate) time: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Day {
    /// `Jn`: 1..=365, Feb 29 never counted.
    NoLeap(u16),
    /// `n`: 0..=365, Feb 29 counted.
    Leap(u16),
    /// `Mm.w.d`: month 1..=12, week 1..=5 (5 = last), weekday 0..=6 (0 = Sunday).
    Weekday { month: u8, week: u8, weekday: u8 },
}

/// Instants are clamped to about ±35 million years before the year arithmetic, so no
/// product overflows `i64` (the answer that far out does not matter, only "no panic").
const LIMIT: i64 = 1 << 50;

impl Posix {
    /// Parses a whole string; anything left over is an error.
    pub(crate) fn parse(s: &str) -> Option<Posix> {
        let mut p = Parser {
            s: s.as_bytes(),
            pos: 0,
        };
        let rule = p.rule()?;
        (p.pos == p.s.len()).then_some(rule)
    }

    /// Seconds east of UTC at `t` (unix seconds). The year is guessed from the mean year and
    /// both transitions are derived from its Jan 1: usually one `days_from_civil` per call
    /// (benches/tz.rs `posix_only`: 39 ns with `civil_from_days` + one per date, 22 ns now).
    pub(crate) fn offset_at(&self, t: i64) -> i32 {
        let Some(dst) = &self.dst else {
            return self.std;
        };
        let t = t.clamp(-LIMIT, LIMIT);
        // the mean Gregorian year (365.2425 days) guesses the year, off by one only near Jan 1
        let mut y = Year::of(1970 + t.div_euclid(31_556_952));
        if t < y.start {
            y = Year::of(y.year - 1);
        } else if t >= y.end {
            y = Year::of(y.year + 1);
        }
        self.in_year(&self.year_dst(dst, &y), t)
    }

    /// The DST interval of year `y` (jiff's evaluation, see the module doc).
    fn year_dst(&self, dst: &Dst, y: &Year) -> YearDst {
        let start = dst.start.to_utc(y, self.std);
        let mut end = dst.end.to_utc(y, dst.offset);
        // Permanent DST (jiff `dst_info_utc`): a start at Jan 1 00:00 UTC and an end that,
        // read in standard time, falls into another year leave no room for standard time.
        if start.0 == y.start {
            let e = end.0 - i64::from(end.1) + i64::from(self.std);
            if e < y.start || e >= y.end {
                end = (y.end, true);
            }
        }
        YearDst {
            start: y.start,
            s: start.0,
            e: end.0,
        }
    }

    /// The offset at `t` inside the year of `d`.
    #[inline]
    pub(crate) fn in_year(&self, d: &YearDst, t: i64) -> i32 {
        let in_dst = if d.s <= d.e {
            d.s <= t && t < d.e
        } else {
            !(d.e <= t && t < d.s)
        };
        match &self.dst {
            Some(dst) if in_dst => dst.offset,
            _ => self.std,
        }
    }

    /// The DST interval of every year `from..=to` (none without DST) and the end of `to`.
    pub(crate) fn years(&self, from: i64, to: i64) -> (Box<[YearDst]>, i64) {
        let Some(dst) = &self.dst else {
            return (Box::new([]), 0);
        };
        let years = (from..=to)
            .map(|y| self.year_dst(dst, &Year::of(y)))
            .collect();
        (years, Year::of(to).end)
    }
}

/// The DST of one year: its first second, and DST from `s` to `e` (`s > e`: outside
/// `e..s`, a southern rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct YearDst {
    pub(crate) start: i64,
    s: i64,
    e: i64,
}

/// A calendar year: its Jan 1 as a day number and as unix seconds, its end (Jan 1 of the
/// next year) in seconds, and whether it has Feb 29.
struct Year {
    year: i64,
    jan1: i64,
    leap: bool,
    start: i64,
    end: i64,
}

impl Year {
    fn of(y: i64) -> Year {
        let jan1 = days_from_civil(y, 1, 1);
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let start = jan1 * 86_400;
        Year {
            year: y,
            jan1,
            leap,
            start,
            end: start + (365 + i64::from(leap)) * 86_400,
        }
    }
}

/// Days before the first of each month in a common year.
const MONTH_START: [u16; 13] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365];

impl Day {
    /// Days since 1970-01-01 of this day in `y`; `None` for day 365 of `n` in a common
    /// year (jiff then takes the end of the year).
    fn to_days(self, y: &Year) -> Option<i64> {
        let leap = i64::from(y.leap);
        match self {
            Day::NoLeap(n) => {
                let n = i64::from(n);
                Some(y.jan1 + n - 1 + leap * i64::from(n >= 60))
            }
            Day::Leap(n) => (i64::from(n) < 365 + leap).then(|| y.jan1 + i64::from(n)),
            Day::Weekday {
                month,
                week,
                weekday,
            } => {
                let m = usize::from(month);
                let feb = leap * i64::from(m > 2);
                let first = y.jan1 + i64::from(MONTH_START[m - 1]) + feb;
                let len = i64::from(MONTH_START[m] - MONTH_START[m - 1]) + leap * i64::from(m == 2);
                // 1970-01-01 was a Thursday (4)
                let first_wd = (first + 4).rem_euclid(7);
                let mut day =
                    (i64::from(weekday) - first_wd).rem_euclid(7) + 7 * (i64::from(week) - 1);
                while day >= len {
                    day -= 7;
                }
                Some(first + day)
            }
        }
    }
}

impl DayTime {
    /// The transition in `y` as unix seconds, the wall time read at `offset`, clamped into
    /// the year as jiff does: before Jan 1 → Jan 1 00:00; after Dec 31 → the end of the year,
    /// returned as Jan 1 of the next year with `true` (jiff's `23:59:59.999999999`: equal for
    /// `<` / `<=` on whole seconds, one second less when added to).
    fn to_utc(self, y: &Year, offset: i32) -> (i64, bool) {
        let Some(days) = self.day.to_days(y) else {
            return (y.end, true);
        };
        let t = days * 86_400 + i64::from(self.time) - i64::from(offset);
        if t < y.start {
            (y.start, false)
        } else if t >= y.end {
            (y.end, true)
        } else {
            (t, false)
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        let hit = self.peek() == Some(b);
        self.pos += usize::from(hit);
        hit
    }

    fn rule(&mut self) -> Option<Posix> {
        self.name()?;
        let std = -self.hms(24)?;
        if !matches!(self.peek(), Some(b) if b == b'<' || b.is_ascii_alphabetic()) {
            return Some(Posix { std, dst: None });
        }
        self.name()?;
        let offset = if self.peek() == Some(b',') {
            std + 3600
        } else {
            -self.hms(24)?
        };
        if !self.eat(b',') {
            return None;
        }
        let start = self.day_time()?;
        if !self.eat(b',') {
            return None;
        }
        let end = self.day_time()?;
        Some(Posix {
            std,
            dst: Some(Dst { offset, start, end }),
        })
    }

    /// `<[A-Za-z0-9+-]{3,}>` or `[A-Za-z]{3,}`.
    fn name(&mut self) -> Option<()> {
        let quoted = self.eat(b'<');
        let start = self.pos;
        while let Some(b) = self.peek() {
            let ok = b.is_ascii_alphabetic()
                || (quoted && (b.is_ascii_digit() || b == b'+' || b == b'-'));
            if !ok {
                break;
            }
            self.pos += 1;
        }
        let len = self.pos - start;
        if quoted && !self.eat(b'>') {
            return None;
        }
        (len >= 3).then_some(())
    }

    /// `[+-]hh[:mm[:ss]]` in seconds, `hh` in `0..=max_hour`.
    fn hms(&mut self, max_hour: i32) -> Option<i32> {
        let sign = if self.eat(b'-') {
            -1
        } else {
            self.eat(b'+');
            1
        };
        let h = self.num(1, if max_hour > 99 { 3 } else { 2 })?;
        if h > max_hour {
            return None;
        }
        let mut secs = h * 3600;
        if self.eat(b':') {
            let m = self.num(2, 2)?;
            if m > 59 {
                return None;
            }
            secs += m * 60;
            if self.eat(b':') {
                let s = self.num(2, 2)?;
                if s > 59 {
                    return None;
                }
                secs += s;
            }
        }
        Some(sign * secs)
    }

    fn num(&mut self, min: usize, max: usize) -> Option<i32> {
        let start = self.pos;
        let mut n = 0i32;
        while self.pos - start < max
            && let Some(b @ b'0'..=b'9') = self.peek()
        {
            n = n * 10 + i32::from(b - b'0');
            self.pos += 1;
        }
        (self.pos - start >= min).then_some(n)
    }

    fn day_time(&mut self) -> Option<DayTime> {
        let day = if self.eat(b'J') {
            let n = self.num(1, 3)?;
            if !(1..=365).contains(&n) {
                return None;
            }
            Day::NoLeap(n as u16)
        } else if self.eat(b'M') {
            let month = self.num(1, 2)?;
            if !(1..=12).contains(&month) || !self.eat(b'.') {
                return None;
            }
            let week = self.num(1, 1)?;
            if !(1..=5).contains(&week) || !self.eat(b'.') {
                return None;
            }
            let weekday = self.num(1, 1)?;
            if weekday > 6 {
                return None;
            }
            Day::Weekday {
                month: month as u8,
                week: week as u8,
                weekday: weekday as u8,
            }
        } else {
            let n = self.num(1, 3)?;
            if n > 365 {
                return None;
            }
            Day::Leap(n as u16)
        };
        let time = if self.eat(b'/') { self.hms(167)? } else { 7200 };
        Some(DayTime { day, time })
    }
}
