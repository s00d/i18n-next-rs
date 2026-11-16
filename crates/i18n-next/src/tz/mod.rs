//! Time zones without dependencies (DESIGN 5.4): civil calendar arithmetic, TZif readers,
//! POSIX TZ rules, the Android tzdata container, a browser backend and a compact bundled tzdb.
//! The date formatter only needs `offset_at(unix seconds) -> seconds east of UTC`.
//!
//! Where a named zone comes from, first hit wins:
//! - browser (`wasm32-unknown-unknown`): `Intl.DateTimeFormat({ timeZone })`, `browser.rs`;
//! - Android: the tzdata container (`/apex/com.android.tzdata/etc/tz/tzdata`, then
//!   `/system/usr/share/zoneinfo/tzdata`);
//! - other non-Windows targets (unix, macOS, iOS, WASI): TZif files under `$TZDIR`,
//!   `/usr/share/zoneinfo`, `/var/db/timezone/zoneinfo`, `/usr/share/lib/zoneinfo`,
//!   `/etc/zoneinfo`;
//! - the bundled tzdb (`tzdb.rs`): with feature `tzdb`, and always on Windows when `tz` is on
//!   (a Cargo feature cannot default to on for one target, so `cfg(windows)` includes it).
//!
//! Parsed zones are cached per name for the process; `offset_at` is a binary search plus the
//! POSIX rule past the last transition, without allocation. The module uses nothing else of
//! the crate, so tests, benches and the fuzz target include it with `#[path]`.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod browser;
pub(crate) mod posix;
#[cfg_attr(
    not(any(feature = "tzdb", windows)),
    allow(dead_code, reason = "used by tests and the tzdb feature")
)]
pub(crate) mod tzdb;
pub(crate) mod tzif;

use posix::Posix;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
pub(crate) use tzif::Table;

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

/// A time zone: UTC, a fixed offset, or rules from TZif / the bundle / the browser. Cloning
/// copies an `Arc`.
#[derive(Clone, Debug)]
pub(crate) struct Zone(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    name: Box<str>,
    rules: Rules,
}

#[derive(Debug)]
enum Rules {
    Fixed(i32),
    Table(Table),
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    Js(browser::JsZone),
}

/// Name of a zone read from a file without a known name (as ICU calls it).
const UNKNOWN: &str = "Etc/Unknown";

impl Zone {
    fn new(name: &str, rules: Rules) -> Zone {
        Zone(Arc::new(Inner {
            name: name.into(),
            rules,
        }))
    }

    pub(crate) fn utc() -> Zone {
        static UTC: OnceLock<Zone> = OnceLock::new();
        UTC.get_or_init(|| Zone::new("UTC", Rules::Fixed(0)))
            .clone()
    }

    /// A constant offset, named `+HH:MM` (`+HH:MM:SS` when the seconds are not zero); 0 is
    /// [`Zone::utc`].
    pub(crate) fn fixed(offset_seconds: i32) -> Zone {
        if offset_seconds == 0 {
            return Zone::utc();
        }
        let a = offset_seconds.unsigned_abs();
        let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
        let mut name = String::with_capacity(9);
        name.push(if offset_seconds < 0 { '-' } else { '+' });
        for (i, v) in [h, m, s].into_iter().enumerate() {
            if i == 2 && v == 0 {
                break;
            }
            if i > 0 {
                name.push(':');
            }
            push_2(&mut name, v);
        }
        Zone::new(&name, Rules::Fixed(offset_seconds))
    }

    /// A zone built from parsed rules (TZif or the bundle), named `name`.
    pub(crate) fn from_table(name: &str, table: Table) -> Zone {
        Zone::new(name, Rules::Table(table))
    }

    /// The IANA zone `iana` from the platform (module doc), `None` if it has no such zone.
    /// Found zones are cached for the process; misses are not (a name from outside cannot
    /// grow the cache).
    pub(crate) fn named(iana: &str) -> Option<Zone> {
        static CACHE: OnceLock<RwLock<HashMap<Box<str>, Zone>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        if let Some(z) = cache.read().unwrap_or_else(|e| e.into_inner()).get(iana) {
            return Some(z.clone());
        }
        let zone = load(iana)?;
        let mut map = cache.write().unwrap_or_else(|e| e.into_inner());
        Some(map.entry(iana.into()).or_insert(zone).clone())
    }

    /// The zone of the process: `TZ`, then `/etc/localtime`; in the browser the zone of
    /// `Intl`; UTC when nothing says otherwise (also on Windows without `TZ`).
    pub(crate) fn system() -> Zone {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            browser::system()
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let tz = std::env::var("TZ").ok();
            detect(&Env {
                tz: tz.as_deref(),
                read: &|p| std::fs::read(p).ok(),
                read_link: &|p| std::fs::read_link(p).ok()?.to_str().map(str::to_owned),
                named: &Zone::named,
            })
        }
    }

    /// Seconds east of UTC at `unix_seconds`.
    #[inline]
    pub(crate) fn offset_at(&self, unix_seconds: i64) -> i32 {
        match &self.0.rules {
            Rules::Fixed(o) => *o,
            Rules::Table(t) => t.offset_at(unix_seconds),
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Rules::Js(z) => z.offset_at(&self.0.name, unix_seconds),
        }
    }

    /// The IANA name, `UTC`, `+HH:MM`, the `TZ` string for a POSIX `TZ`, or `Etc/Unknown`
    /// for an unnamed `/etc/localtime`.
    pub(crate) fn name(&self) -> &str {
        &self.0.name
    }
}

fn push_2(s: &mut String, v: u32) {
    s.push(char::from(b'0' + (v / 10 % 10) as u8));
    s.push(char::from(b'0' + (v % 10) as u8));
}

/// IANA names only: ASCII letters, digits, `_+-`, components separated by `/`, none empty
/// or starting with `.`; so a name can never leave the zoneinfo directory.
pub(crate) fn valid_name(name: &str) -> bool {
    name.len() <= 255
        && name.split('/').all(|c| {
            !c.is_empty()
                && !c.starts_with('.')
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-' | b'.'))
        })
}

/// The platform lookup behind [`Zone::named`], uncached.
pub(crate) fn load(name: &str) -> Option<Zone> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if let Some(z) = browser::named(name) {
        return Some(z);
    }
    if !valid_name(name) {
        return None;
    }
    #[cfg(target_os = "android")]
    for path in [
        "/apex/com.android.tzdata/etc/tz/tzdata",
        "/system/usr/share/zoneinfo/tzdata",
    ] {
        // ponytail: reads the whole container (~500 KB) per cold name; seek + read the index
        // if apps look up many zones
        if let Ok(d) = std::fs::read(path)
            && let Some(t) = tzif::android_find(&d, name).and_then(tzif::parse)
        {
            return Some(Zone::from_table(name, t));
        }
    }
    #[cfg(not(any(
        windows,
        target_os = "android",
        all(target_arch = "wasm32", target_os = "unknown")
    )))]
    {
        let tzdir = std::env::var("TZDIR").ok();
        let dirs = tzdir.iter().map(String::as_str).chain(ZONEINFO);
        for dir in dirs {
            if let Ok(d) = std::fs::read(format!("{dir}/{name}"))
                && let Some(t) = tzif::parse(&d)
            {
                return Some(Zone::from_table(name, t));
            }
        }
    }
    #[cfg(any(feature = "tzdb", windows))]
    if let Some(t) = tzdb::find(tzdb::DATA, name) {
        return Some(Zone::from_table(name, t));
    }
    None
}

/// System zoneinfo directories, after `$TZDIR`.
#[cfg_attr(
    any(
        windows,
        target_os = "android",
        all(target_arch = "wasm32", target_os = "unknown")
    ),
    allow(dead_code, reason = "no zoneinfo files there")
)]
pub(crate) const ZONEINFO: [&str; 4] = [
    "/usr/share/zoneinfo",
    "/var/db/timezone/zoneinfo",
    "/usr/share/lib/zoneinfo",
    "/etc/zoneinfo",
];

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    allow(dead_code, reason = "no TZ or files in the browser")
)]
/// What [`detect`] may look at: the `TZ` value and the file system, injected so tests run
/// on fakes instead of mutating the process environment.
pub(crate) struct Env<'a> {
    pub(crate) tz: Option<&'a str>,
    /// Contents of an absolute path.
    pub(crate) read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    /// Target of a symbolic link.
    pub(crate) read_link: &'a dyn Fn(&str) -> Option<String>,
    /// A zone by IANA name ([`Zone::named`] for the real system).
    pub(crate) named: &'a dyn Fn(&str) -> Option<Zone>,
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    allow(dead_code, reason = "no TZ or files in the browser")
)]
/// The system zone as glibc and tzcode pick it:
/// - `TZ` set and empty: UTC; a leading `:` is dropped; an absolute path is read as TZif;
///   otherwise a zone name, then a POSIX rule (`CET-1CEST,M3.5.0,M10.5.0/3`); unusable: UTC;
/// - `TZ` unset: the name after `zoneinfo/` in the target of the `/etc/localtime` link
///   (`/var/db/timezone/zoneinfo/Europe/Berlin`, `../usr/share/zoneinfo/posix/Asia/Tokyo`),
///   else the file itself as TZif (`Etc/Unknown`), else UTC.
pub(crate) fn detect(env: &Env<'_>) -> Zone {
    let from_file = |path: &str, name: Option<&str>| {
        let table = tzif::parse(&(env.read)(path)?)?;
        Some(Zone::from_table(name.unwrap_or(UNKNOWN), table))
    };
    match env.tz {
        Some(tz) => {
            let tz = tz.strip_prefix(':').unwrap_or(tz);
            if tz.is_empty() {
                return Zone::utc();
            }
            let found = if tz.starts_with('/') {
                from_file(tz, zone_name_of(tz))
            } else {
                (env.named)(tz)
                    .or_else(|| Some(Zone::new(tz, Rules::Table(posix_only(Posix::parse(tz)?)))))
            };
            found.unwrap_or_else(Zone::utc)
        }
        None => {
            const LOCALTIME: &str = "/etc/localtime";
            let name = (env.read_link)(LOCALTIME);
            let name = name.as_deref().and_then(zone_name_of);
            name.and_then(env.named)
                .or_else(|| from_file(LOCALTIME, name))
                .unwrap_or_else(Zone::utc)
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    allow(dead_code, reason = "no TZ or files in the browser")
)]
fn posix_only(rule: Posix) -> Table {
    Table {
        times: Box::new([]),
        offsets: Box::new([]),
        initial: rule.std,
        rule: Some(rule),
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    allow(dead_code, reason = "no TZ or files in the browser")
)]
/// `Europe/Berlin` from `…/zoneinfo/Europe/Berlin` (also `…/zoneinfo/posix/…` and
/// `…/zoneinfo/right/…`); `None` when the path has no `zoneinfo/` component or the rest is
/// not a valid name.
pub(crate) fn zone_name_of(path: &str) -> Option<&str> {
    let (_, rest) = path.rsplit_once("zoneinfo/")?;
    let rest = rest
        .strip_prefix("posix/")
        .or_else(|| rest.strip_prefix("right/"))
        .unwrap_or(rest);
    valid_name(rest).then_some(rest)
}
