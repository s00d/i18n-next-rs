//! Proleptic Gregorian calendar arithmetic (H. Hinnant). Shared by `tz` and, through
//! `#[path]`, by the date formatter of a build without `tz` (`format/date.rs`).

/// A UTC instant split into calendar fields (proleptic Gregorian, astronomical years:
/// 0 = 1 BC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Civil {
    pub(crate) year: i64,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) second: u8,
    /// 0 = Sunday.
    pub(crate) weekday: u8,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant, `days_from_civil`).
/// `m` in 1..=12, `d` in 1..=31; any `i64` year whose result fits.
pub(crate) fn days_from_civil(y: i64, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month, day)` of a day count since 1970-01-01 (H. Hinnant, `civil_from_days`).
pub(crate) fn civil_from_days(z: i64) -> (i64, u8, u8) {
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

/// Calendar fields of unix seconds in UTC; every `i64` works.
pub(crate) fn civil_from_unix(t: i64) -> Civil {
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
