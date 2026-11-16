//! TZif (RFC 8536, versions 1–4) and the Android tzdata container.
//!
//! Only what `offset_at` needs is kept: transition times, the offset after each, the offset
//! before the first (time type 0) and the footer rule. Abbreviations, leap-second records and
//! the std/wall indicators are skipped, as in `jiff`. Every count and index is checked, so
//! malformed input gives `None`, never a panic (fuzz target `tzif`).

use super::posix::Posix;

/// Transitions and the rule beyond them: the parsed form of a TZif file or a bundled zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Table {
    /// Strictly ascending unix seconds.
    pub(crate) times: Box<[i64]>,
    /// `offsets[i]`: seconds east of UTC from `times[i]` on.
    pub(crate) offsets: Box<[i32]>,
    /// Before the first transition (and with no transitions and no rule).
    pub(crate) initial: i32,
    /// On and after the last transition (all the time when there are none).
    pub(crate) rule: Option<Posix>,
}

impl Table {
    /// Binary search over the transitions, the rule past the last one (RFC 8536 3.2, as
    /// jiff: an instant equal to the last transition already takes the rule).
    pub(crate) fn offset_at(&self, t: i64) -> i32 {
        let i = self.times.partition_point(|&x| x <= t);
        if i == self.times.len()
            && let Some(rule) = &self.rule
        {
            return rule.offset_at(t);
        }
        match i.checked_sub(1) {
            Some(last) => self.offsets[last],
            None => self.initial,
        }
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
    Some(Table {
        times: times.into(),
        offsets: offsets.into(),
        initial,
        rule,
    })
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
