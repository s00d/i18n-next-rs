//! TZif (RFC 8536, versions 1–4) and the Android tzdata container.
//!
//! Only what `offset_at` needs is kept: transition times, the offset after each, the offset
//! before the first (time type 0) and the footer rule. Abbreviations, leap-second records and
//! the std/wall indicators are skipped, as in `jiff`. Every count and index is checked, so
//! malformed input gives `None`, never a panic (fuzz target `tzif`).

use super::posix::{Posix, YearDst};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The last year whose DST the rule is evaluated for ahead of time ([`Table::new`]).
const YEARS_TO: i64 = 2100;

/// Transitions and the rule beyond them: the parsed form of a TZif file or a bundled zone.
#[derive(Debug)]
pub(crate) struct Table {
    /// Strictly ascending unix seconds.
    pub(crate) times: Box<[i64]>,
    /// `offsets[i]`: seconds east of UTC from `times[i]` on.
    pub(crate) offsets: Box<[i32]>,
    /// Before the first transition (and with no transitions and no rule).
    pub(crate) initial: i32,
    /// On and after the last transition (all the time when there are none).
    pub(crate) rule: Option<Posix>,
    /// The rule's DST per year from the year of the last transition (1970 at the earliest)
    /// to 2100, and the end of 2100: a lookup instead of the year arithmetic (benches/tz.rs
    /// `rule_2080`). About 24 bytes a year for a zone with DST, nothing without.
    years: Box<[YearDst]>,
    years_end: i64,
    /// The transition the last lookup landed after: the next one is usually the same
    /// (dates of one page are close). A guess only, so any thread may overwrite it.
    hint: AtomicUsize,
}

impl Clone for Table {
    fn clone(&self) -> Self {
        Table::new(
            self.times.clone(),
            self.offsets.clone(),
            self.initial,
            self.rule.clone(),
        )
    }
}

/// What the zone says; the cached years follow from it and the hint is no part of it.
impl PartialEq for Table {
    fn eq(&self, other: &Self) -> bool {
        (&self.times, &self.offsets, self.initial, &self.rule)
            == (&other.times, &other.offsets, other.initial, &other.rule)
    }
}

impl Eq for Table {}

impl Table {
    pub(crate) fn new(
        times: Box<[i64]>,
        offsets: Box<[i32]>,
        initial: i32,
        rule: Option<Posix>,
    ) -> Table {
        // at most 1970..=2100: a damaged file's transition far in the past costs nothing
        let from = times
            .last()
            .map_or(1970, |&t| super::civil_from_unix(t).year)
            .max(1970);
        let (years, years_end) = match &rule {
            Some(r) if from <= YEARS_TO => r.years(from, YEARS_TO),
            _ => (Box::new([]) as Box<[YearDst]>, 0),
        };
        Table {
            times,
            offsets,
            initial,
            rule,
            years,
            years_end,
            hint: AtomicUsize::new(0),
        }
    }

    /// Binary search over the transitions (after a check of the last hit), the rule past the
    /// last one (RFC 8536 3.2, as jiff: an instant equal to the last transition already
    /// takes the rule).
    pub(crate) fn offset_at(&self, t: i64) -> i32 {
        let n = self.times.len();
        let h = self.hint.load(Ordering::Relaxed);
        let hit = self.times.get(h).is_some_and(|&x| x <= t)
            && self.times.get(h + 1).is_none_or(|&x| t < x);
        let i = if hit {
            h + 1
        } else {
            let i = self.times.partition_point(|&x| x <= t);
            if i > 0 {
                self.hint.store(i - 1, Ordering::Relaxed);
            }
            i
        };
        if i == n
            && let Some(rule) = &self.rule
        {
            return self.rule_at(rule, t);
        }
        match i.checked_sub(1) {
            Some(last) => self.offsets[last],
            None => self.initial,
        }
    }

    /// The rule at `t`: from the years evaluated ahead when `t` is among them.
    fn rule_at(&self, rule: &Posix, t: i64) -> i32 {
        let Some(first) = self.years.first() else {
            return rule.offset_at(t);
        };
        if t < first.start || t >= self.years_end {
            return rule.offset_at(t);
        }
        // the mean year guesses the index, off by one only near Jan 1
        let mut k = usize::try_from((t - first.start) / 31_556_952).unwrap_or(0);
        k = k.min(self.years.len() - 1);
        while k > 0 && self.years[k].start > t {
            k -= 1;
        }
        while self.years.get(k + 1).is_some_and(|y| y.start <= t) {
            k += 1;
        }
        rule.in_year(&self.years[k], t)
    }
}

/// Offsets beyond ±26 h are not a time zone (RFC 8536 limits them to ±25:59:59).
const MAX_OFFSET: u32 = 26 * 3600 - 1;

fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `[isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt]` of the header at `at`.
fn header(d: &[u8], at: usize) -> Option<(u8, [usize; 6])> {
    if d.get(at..at.checked_add(4)?)? != b"TZif" {
        return None;
    }
    let version = *d.get(at + 4)?;
    let mut counts = [0usize; 6];
    for (i, c) in counts.iter_mut().enumerate() {
        *c = usize::try_from(be32(d, at + 20 + 4 * i)?).ok()?;
    }
    Some((version, counts))
}

/// Bytes of the data block after a header, `tsize` = 4 (v1) or 8 (v2+).
fn block_len(c: &[usize; 6], tsize: usize) -> Option<usize> {
    let [isut, isstd, leap, time, typ, chars] = *c;
    time.checked_mul(tsize + 1)?
        .checked_add(typ.checked_mul(6)?)?
        .checked_add(chars)?
        .checked_add(leap.checked_mul(tsize + 4)?)?
        .checked_add(isstd)?
        .checked_add(isut)
}

/// Parses a whole TZif file; `None` when it is not one or is inconsistent.
pub(crate) fn parse(d: &[u8]) -> Option<Table> {
    let (version, v1) = header(d, 0)?;
    let (tsize, at, c) = if version >= b'2' {
        let at = 44usize.checked_add(block_len(&v1, 4)?)?;
        let (_, c) = header(d, at)?;
        (8, at + 44, c)
    } else {
        (4, 44, v1)
    };
    let [_, _, _, timecnt, typecnt, _] = c;
    let end = at.checked_add(block_len(&c, tsize)?)?;
    if typecnt == 0 || d.len() < end {
        return None;
    }
    let idx_at = at + timecnt * tsize;
    let types_at = idx_at + timecnt;
    let utoff = |i: usize| -> Option<i32> {
        let raw = be32(d, types_at + 6 * i)?;
        let off = raw as i32;
        (off.unsigned_abs() <= MAX_OFFSET).then_some(off)
    };
    let mut times = Vec::with_capacity(timecnt);
    let mut offsets = Vec::with_capacity(timecnt);
    for i in 0..timecnt {
        let p = at + i * tsize;
        let t = if tsize == 8 {
            let b = &d[p..p + 8];
            i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        } else {
            i64::from(be32(d, p)? as i32)
        };
        if times.last().is_some_and(|&prev| prev >= t) {
            return None;
        }
        let ty = usize::from(d[idx_at + i]);
        if ty >= typecnt {
            return None;
        }
        times.push(t);
        offsets.push(utoff(ty)?);
    }
    let initial = utoff(0)?;
    let rule = if version >= b'2' {
        footer(d.get(end..)?)?
    } else {
        None
    };
    Some(Table::new(times.into(), offsets.into(), initial, rule))
}

/// `\n<TZ string>\n` after the v2+ data: `Some(None)` for an empty string or a missing
/// footer, `None` when the string does not parse.
fn footer(rest: &[u8]) -> Option<Option<Posix>> {
    let Some(body) = rest.strip_prefix(b"\n") else {
        return Some(None);
    };
    let len = body.iter().position(|&b| b == b'\n')?;
    if len == 0 {
        return Some(None);
    }
    Posix::parse(std::str::from_utf8(&body[..len]).ok()?).map(Some)
}

/// The TZif bytes of `name` in an Android tzdata container (`bionic/libc/tzcode`
/// `ZoneInfoDB`): `tzdata` + version + NUL (12 bytes), big-endian u32 offsets of the index,
/// the data and the zone tab, then index entries of 52 bytes: name NUL-padded to 40, start
/// (from the data offset), length, raw offset.
pub(crate) fn android_find<'a>(d: &'a [u8], name: &str) -> Option<&'a [u8]> {
    if !d.starts_with(b"tzdata") {
        return None;
    }
    let index = usize::try_from(be32(d, 12)?).ok()?;
    let data = usize::try_from(be32(d, 16)?).ok()?;
    let entries = d.get(index..data)?;
    for e in entries.as_chunks::<52>().0 {
        let n = &e[..40];
        let len = n.iter().position(|&b| b == 0).unwrap_or(40);
        if &n[..len] == name.as_bytes() {
            let start = data.checked_add(usize::try_from(be32(e, 40)?).ok()?)?;
            let size = usize::try_from(be32(e, 44)?).ok()?;
            return d.get(start..start.checked_add(size)?);
        }
    }
    None
}
