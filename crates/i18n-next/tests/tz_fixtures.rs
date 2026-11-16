//! The own `tz` on hand-made inputs (DESIGN 5.4):
//! - `fixtures/tzif` (written by `node scripts/tzdb.mjs fixtures`): v1 (32-bit, no footer),
//!   v2 (64-bit times before 1901 and after 2038, LMT with seconds, footer), v3 (rule times
//!   outside 0..24 h), no transitions with a POSIX-only footer, no transitions and no footer,
//!   and an Android tzdata container holding two of them; each against jiff and by hand;
//! - truncated and corrupted files: `None` or a table, never a panic;
//! - POSIX rules (`Mm.w.d`, `Jn`, `n`, negative and large offsets and times, quoted names,
//!   permanent DST) against `jiff::tz::TimeZone::posix`, and the strings both reject;
//! - system zone detection on a fake environment and file system (`tz::Env`), so no test
//!   touches the real `TZ`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use std::collections::HashMap;
use tz::posix::Posix;
use tz::{Env, Zone, tzif};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/tzif/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo) as u64) as i64
    }
}

/// 1800-01-01 .. 2200-01-01: wider than the zones' data on both sides.
const FROM: i64 = -5_364_662_400;
const TO: i64 = 7_258_118_400;

/// `mine` equals jiff on its transitions ± 1 s and 3000 random instants in `[FROM, TO)`.
fn same_as_jiff(what: &str, mine: impl Fn(i64) -> i32, oracle: &TimeZone) {
    let mut rng = Rng(what.len() as u64);
    let mut ts: Vec<i64> = (0..3000).map(|_| rng.range(FROM, TO)).collect();
    let mut prev = i64::MIN;
    for tr in oracle.following(Timestamp::from_second(FROM).unwrap()) {
        let t = tr.timestamp().as_second();
        // jiff 0.2.45 repeats the last transition forever on a TZif without a footer (v1)
        if t >= TO || t <= prev {
            break;
        }
        prev = t;
        if t > FROM {
            ts.extend([t - 1, t, t + 1]);
        }
    }
    for t in ts {
        let want = oracle
            .to_offset(Timestamp::from_second(t).unwrap())
            .seconds();
        assert_eq!(mine(t), want, "{what} at {t}");
    }
}

/// Given each TZif fixture, when parsed by the own reader and by jiff, then both give the
/// same offsets everywhere from 1800 to 2200.
#[test]
fn tzif_fixtures_match_jiff() {
    for name in [
        "v1.tzif",
        "v2.tzif",
        "v3.tzif",
        "posix-only.tzif",
        "empty.tzif",
    ] {
        let bytes = fixture(name);
        let table = tzif::parse(&bytes).unwrap_or_else(|| panic!("own reader rejects {name}"));
        let oracle = TimeZone::tzif(name, &bytes).unwrap();
        same_as_jiff(name, |t| table.offset_at(t), &oracle);
    }
}

/// Given the fixtures, when asked at instants picked by hand, then the offsets are the ones
/// their bytes say (scripts/tzdb.mjs `fixtures`).
#[test]
fn tzif_fixtures_by_hand() {
    let v1 = tzif::parse(&fixture("v1.tzif")).unwrap();
    assert!(v1.rule.is_none());
    for (t, want) in [
        (i64::MIN, 3600),
        (99_999_999, 3600),
        (100_000_000, 7200),
        (115_000_000, 3600),
        (125_000_000, 7200),
        (i64::MAX, 3600),
    ] {
        assert_eq!(v1.offset_at(t), want, "v1 at {t}");
    }

    let v2 = tzif::parse(&fixture("v2.tzif")).unwrap();
    assert_eq!(
        v2.times.first(),
        Some(&-2_717_650_800),
        "a 64-bit time before 1901"
    );
    for (t, want) in [
        (-3_000_000_000, -17_762), // LMT -4:56:02 before the first transition
        (-2_717_650_800, -18_000), // 1883: EST
        (-1_620_000_000, -14_400), // 1918: EDT
        (3_000_000_000, -18_000),  // 2065-01: past 32 bits, still the 1918 EST
        (3_003_721_200, -14_400),  // 2065-03-08: a 64-bit transition
        (3_024_280_800, -18_000),  // the last transition: from here the footer (EST too)
        (3_047_788_800, -14_400),  // 2066-07-31: EDT by the rule
        (3_061_065_600, -18_000),  // 2067-01-01: EST by the rule
    ] {
        assert_eq!(v2.offset_at(t), want, "v2 at {t}");
    }

    let v3 = tzif::parse(&fixture("v3.tzif")).unwrap();
    // one transition (2023-03-26T01:00Z, to -01), from there <-02>2<-01>,M3.5.0/-1,M10.5.0/0,
    // whose DST starts at that same instant
    assert_eq!(v3.offset_at(1_679_792_399), -10_800);
    assert_eq!(v3.offset_at(1_679_792_400), -3600);
    assert_eq!(v3.offset_at(1_893_456_000), -7200, "2030-01-01");
    assert_eq!(v3.offset_at(1_909_180_800), -3600, "2030-07-01");

    let posix = tzif::parse(&fixture("posix-only.tzif")).unwrap();
    assert!(posix.times.is_empty());
    let empty = tzif::parse(&fixture("empty.tzif")).unwrap();
    assert!(empty.rule.is_none());
    for t in [i64::MIN, -1, 0, 1_700_000_000, i64::MAX] {
        assert_eq!(posix.offset_at(t), 12_600, "posix-only at {t}");
        assert_eq!(empty.offset_at(t), -36_000, "empty at {t}");
    }
}

/// Given an Android tzdata container with two zones, when a zone is looked up by name, then
/// its slice is exactly that zone's TZif file; an unknown name, a bad magic or a truncated
/// index give `None`.
#[test]
fn android_container() {
    let c = fixture("tzdata");
    assert_eq!(
        tzif::android_find(&c, "Test/V2"),
        Some(&fixture("v2.tzif")[..])
    );
    assert_eq!(
        tzif::android_find(&c, "Test/PosixOnly"),
        Some(&fixture("posix-only.tzif")[..])
    );
    let t = tzif::parse(tzif::android_find(&c, "Test/V2").unwrap()).unwrap();
    assert_eq!(t.offset_at(-1_620_000_000), -14_400);
    assert_eq!(tzif::android_find(&c, "Test/V"), None);
    assert_eq!(tzif::android_find(&c, "Test/V2X"), None);
    assert_eq!(tzif::android_find(&c[..60], "Test/V2"), None);
    let mut bad = c.clone();
    bad[0] = b'x';
    assert_eq!(tzif::android_find(&bad, "Test/V2"), None);
    for n in 0..c.len() {
        let _ = tzif::android_find(&c[..n], "Test/PosixOnly").map(tzif::parse);
    }
}

/// Given every fixture truncated at every length and with every byte flipped, when parsed,
/// then the reader returns `None` or a table and never panics (the fuzz target `tzif` does
/// the same with arbitrary bytes).
#[test]
fn damaged_tzif_never_panics() {
    for name in [
        "v1.tzif",
        "v2.tzif",
        "v3.tzif",
        "posix-only.tzif",
        "empty.tzif",
    ] {
        let bytes = fixture(name);
        for n in 0..bytes.len() {
            if let Some(t) = tzif::parse(&bytes[..n]) {
                let _ = t.offset_at(0);
            }
        }
        for i in 0..bytes.len() {
            for flip in [0x01, 0x80, 0xff] {
                let mut b = bytes.clone();
                b[i] ^= flip;
                if let Some(t) = tzif::parse(&b) {
                    for at in [i64::MIN, -1, 0, 4_000_000_000, i64::MAX] {
                        let _ = t.offset_at(at);
                    }
                }
            }
        }
    }
    assert!(tzif::parse(b"").is_none());
    assert!(tzif::parse(b"TZif2").is_none());
}

/// POSIX strings both parsers accept: every rule form and the corners of v3.
const POSIX_OK: &[&str] = &[
    "UTC0",
    "<+14>-14",
    "<+0330>-3:30",
    "<-0930>9:30",
    "EST5EDT,M3.2.0,M11.1.0",
    "CET-1CEST,M3.5.0,M10.5.0/3",
    "AEST-10AEDT,M10.1.0,M4.1.0/3",
    "NZST-12NZDT,M9.5.0,M4.1.0/3",
    "<+1245>-12:45<+1345>,M9.5.0/2:45,M4.1.0/3:45",
    "IST-2IDT,M3.4.4/26,M10.5.0",
    "<-03>3<-02>,M3.5.0/-2,M10.5.0/-1",
    "<-02>2<-01>,M3.5.0/-1,M10.5.0/0",
    "IST-1GMT0,M10.5.0,M3.5.0/1",
    "XXX-2<+01>-1,0/0,J365/23",
    "EST5EDT4,0/0,J365/25",
    "WART4WARST,J1/0,J365/25",
    "ABC3DEF,J60/2,J300/2",
    "ABC3DEF,59/2,365/2",
    "ABC3DEF,0,365",
    "ABC-14DEF-13:45:30,M1.1.0/0,M12.5.6/24",
    "ABC+24DEF,M2.5.0/-167,M11.5.0/167",
    "ABC-24DEF+24,M1.5.6/167:59:59,M12.1.0/-167:59:59",
    "ABC3DEF2:30:15,M3.2.0/0:00:01,M11.1.0/23:59:59",
    "abc3def,M3.2.0,M11.1.0",
];

/// Given each POSIX string, when the own rule and jiff's answer instants 1800–2200 (random,
/// jiff's transitions ± 1 s, and every Jan 1 / Dec 31 boundary), then they agree.
#[test]
fn posix_rules_match_jiff() {
    for s in POSIX_OK {
        let mine = Posix::parse(s).unwrap_or_else(|| panic!("own parser rejects {s}"));
        let oracle = TimeZone::posix(s).unwrap_or_else(|e| panic!("jiff rejects {s}: {e}"));
        same_as_jiff(s, |t| mine.offset_at(t), &oracle);
        for year in 1990..2040 {
            let jan1 = tz::days_from_civil(year, 1, 1) * 86_400;
            for t in [jan1 - 1, jan1, jan1 + 1, jan1 + 3600, jan1 - 3600] {
                let want = oracle
                    .to_offset(Timestamp::from_second(t).unwrap())
                    .seconds();
                assert_eq!(mine.offset_at(t), want, "{s} at {t}");
            }
        }
    }
}

/// Given malformed POSIX strings, when parsed, then both parsers reject them.
#[test]
fn posix_rejects_what_jiff_rejects() {
    for s in [
        "",
        "AB5",
        "EST",
        "EST5EDT",
        "EST5EDT4",
        "EST5EDT,M3.2.0",
        "EST5EDT,M3.2.0,",
        "EST25",
        "EST5:60",
        "EST5:5",
        "EST5EDT,M13.1.0,M11.1.0",
        "EST5EDT,M3.6.0,M11.1.0",
        "EST5EDT,M3.1.7,M11.1.0",
        "EST5EDT,M0.1.0,M11.1.0",
        "EST5EDT,J0,J365",
        "EST5EDT,J1,J366",
        "EST5EDT,1,366",
        "<AB>5",
        "<ABC5",
        "<AB C>5",
        "EST5EDT,M3.2.0/168,M11.1.0",
        "EST5EDT,M3.2.0/-168,M11.1.0",
        "EST5 ",
        "EST5EDT;M3.2.0,M11.1.0",
        "5EST",
        "EST5EDT,M3.2,M11.1.0",
        "EST5EDT,M3.2.0/2:3,M11.1.0",
        "E1T5",
    ] {
        assert!(Posix::parse(s).is_none(), "own parser accepts {s:?}");
        assert!(
            TimeZone::posix(s).is_err(),
            "jiff accepts {s:?}: the case is not a rejection"
        );
    }
}

/// Given random byte strings built from POSIX characters, when both parse them, then they
/// accept the same ones and agree on offsets where both accept.
#[test]
fn posix_random_strings_agree_with_jiff() {
    const ALPHABET: &[u8] = b"ESTDCJM0123456789,./:+-<>";
    let mut rng = Rng(42);
    let mut accepted = 0;
    for _ in 0..200_000 {
        let len = 4 + rng.next() as usize % 24;
        let s: String = (0..len)
            .map(|_| ALPHABET[rng.next() as usize % ALPHABET.len()] as char)
            .collect();
        let mine = Posix::parse(&s);
        let oracle = TimeZone::posix(&s);
        assert_eq!(
            mine.is_some(),
            oracle.is_ok(),
            "{s:?}: own {mine:?}, jiff {oracle:?}"
        );
        if let (Some(m), Ok(o)) = (mine, oracle) {
            accepted += 1;
            for t in [
                -1_000_000_000,
                0,
                1_700_000_000,
                1_720_000_000,
                4_000_000_000,
            ] {
                assert_eq!(
                    m.offset_at(t),
                    o.to_offset(Timestamp::from_second(t).unwrap()).seconds(),
                    "{s} at {t}"
                );
            }
        }
    }
    assert!(accepted > 0, "the generator never produced a valid string");
}

/// A fake system for `tz::detect`: files, links and the zones `named` knows.
struct Fake {
    files: HashMap<&'static str, Vec<u8>>,
    links: HashMap<&'static str, &'static str>,
    zones: HashMap<&'static str, &'static str>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            files: HashMap::new(),
            links: HashMap::new(),
            zones: HashMap::from([("Test/V2", "v2.tzif"), ("Test/V3", "v3.tzif")]),
        }
    }

    fn detect(&self, tz: Option<&str>) -> Zone {
        let named = |n: &str| {
            self.zones
                .get(n)
                .map(|f| Zone::from_table(n, tzif::parse(&fixture(f)).unwrap()))
        };
        tz::detect(&Env {
            tz,
            read: &|p| self.files.get(p).cloned(),
            read_link: &|p| self.links.get(p).map(|s| (*s).to_owned()),
            named: &named,
        })
    }
}

/// 1918-06-01: EDT in Test/V2, a plain day elsewhere.
const JUNE_1918: i64 = -1_627_689_600;

/// Given `TZ` unset, when `/etc/localtime` links into a zoneinfo directory (absolute,
/// relative, under `posix/`), then the zone is the one named after `zoneinfo/`.
#[test]
fn detect_localtime_link() {
    let mut fake = Fake::new();
    for target in [
        "/var/db/timezone/zoneinfo/Test/V2",
        "../usr/share/zoneinfo/Test/V2",
        "/usr/share/zoneinfo/posix/Test/V2",
        "/usr/share/zoneinfo/right/Test/V2",
    ] {
        fake.links.insert("/etc/localtime", target);
        let z = fake.detect(None);
        assert_eq!(z.name(), "Test/V2", "link to {target}");
        assert_eq!(z.offset_at(JUNE_1918), -14_400);
    }
}

/// Given `TZ` unset and `/etc/localtime` a copied file (no link) or a link to a zone the
/// lookup does not know, when detected, then the file itself is read: named `Etc/Unknown`,
/// or after the link target.
#[test]
fn detect_localtime_file() {
    let mut fake = Fake::new();
    fake.files.insert("/etc/localtime", fixture("v1.tzif"));
    let z = fake.detect(None);
    assert_eq!(z.name(), "Etc/Unknown");
    assert_eq!(z.offset_at(100_000_000), 7200);
    fake.links
        .insert("/etc/localtime", "/usr/share/zoneinfo/Test/Copied");
    assert_eq!(fake.detect(None).name(), "Test/Copied");
    fake.files.insert("/etc/localtime", b"not a tzif".to_vec());
    assert_eq!(
        fake.detect(None).name(),
        "UTC",
        "unreadable file, unknown name"
    );
}

/// Given nothing (no `TZ`, no `/etc/localtime`, as on Windows), when detected, then UTC.
#[test]
fn detect_nothing_is_utc() {
    let z = Fake::new().detect(None);
    assert_eq!((z.name(), z.offset_at(JUNE_1918)), ("UTC", 0));
}

/// Given `TZ` set, when detected, then it wins over `/etc/localtime`: empty or `:` is UTC, a
/// name (with or without `:`) is looked up, an absolute path is read, a POSIX rule is used
/// as such, anything else is UTC.
#[test]
fn detect_tz_variable() {
    let mut fake = Fake::new();
    fake.links
        .insert("/etc/localtime", "/usr/share/zoneinfo/Test/V3");
    fake.files
        .insert("/usr/share/zoneinfo/Test/V1", fixture("v1.tzif"));
    fake.files.insert("/opt/zone", fixture("v1.tzif"));
    assert_eq!(fake.detect(None).name(), "Test/V3");
    for (tz, name) in [
        ("", "UTC"),
        (":", "UTC"),
        ("Test/V2", "Test/V2"),
        (":Test/V2", "Test/V2"),
        (":/usr/share/zoneinfo/Test/V1", "Test/V1"),
        ("/opt/zone", "Etc/Unknown"),
        ("/opt/missing", "UTC"),
        ("CET-1CEST,M3.5.0,M10.5.0/3", "CET-1CEST,M3.5.0,M10.5.0/3"),
        ("Nowhere/Zone", "UTC"),
        ("../../etc/passwd", "UTC"),
    ] {
        assert_eq!(fake.detect(Some(tz)).name(), name, "TZ={tz:?}");
    }
    let cet = fake.detect(Some("CET-1CEST,M3.5.0,M10.5.0/3"));
    assert_eq!(cet.offset_at(1_782_864_000), 7200, "2026-07-01: CEST");
    assert_eq!(cet.offset_at(1_767_225_600), 3600, "2026-01-01: CET");
    assert_eq!(fake.detect(Some("/opt/zone")).offset_at(100_000_000), 7200);
}

/// Given the real process, when the system zone is asked for, then some zone comes back
/// (its value depends on the machine; detection itself is covered above on fakes).
#[test]
fn system_zone_resolves() {
    let z = Zone::system();
    assert!(!z.name().is_empty());
    let _ = z.offset_at(1_782_864_000);
}

/// Given fixed offsets, when named, then `+HH:MM` (seconds only when present), 0 is UTC.
#[test]
fn fixed_zone_names() {
    for (off, name) in [
        (0, "UTC"),
        (19_800, "+05:30"),
        (-3600, "-01:00"),
        (3723, "+01:02:03"),
        (-50_400, "-14:00"),
    ] {
        let z = Zone::fixed(off);
        assert_eq!(
            (z.name(), z.offset_at(i64::MIN), z.offset_at(i64::MAX)),
            (name, off, off)
        );
    }
    assert_eq!(Zone::utc().name(), "UTC");
}

/// Given names that are not IANA names, when looked up, then `None` without touching a file
/// outside the zoneinfo directories.
#[test]
fn named_rejects_paths() {
    for bad in [
        "",
        "/etc/passwd",
        "../etc/passwd",
        "Europe/../../etc/passwd",
        "Europe//Berlin",
        "Europe/.hidden",
        "a b",
        "Europe/Berlin\0",
    ] {
        assert!(!tz::valid_name(bad), "{bad:?}");
        assert!(Zone::named(bad).is_none(), "{bad:?}");
    }
    // files under zoneinfo that are no zones for `Intl`
    for file in [
        "posixrules",
        "Factory",
        "posix/Europe/Berlin",
        "right/Europe/Berlin",
        "localtime",
    ] {
        assert!(Zone::named(file).is_none(), "{file:?}");
    }
    for good in [
        "UTC",
        "Europe/Berlin",
        "Etc/GMT+5",
        "America/Port-au-Prince",
        "America/Argentina/Buenos_Aires",
    ] {
        assert!(tz::valid_name(good), "{good:?}");
    }
    assert_eq!(
        tz::zone_name_of("/usr/share/zoneinfo/Asia/Tokyo"),
        Some("Asia/Tokyo")
    );
    assert_eq!(tz::zone_name_of("/etc/zoneinfo-backup"), None);
}

/// Given the generated name list, then it is strictly sorted by ASCII lowercase (the
/// binary search of `canonical` relies on it, and no two names differ by case only),
/// every alias reads an IANA name of the list, and names `Intl` rejects are absent.
#[test]
fn name_list_is_searchable() {
    let lower = |s: &str| s.to_ascii_lowercase();
    assert!(
        tz::names::NAMES
            .windows(2)
            .all(|w| lower(w[0].0) < lower(w[1].0))
    );
    for (name, file) in tz::names::NAMES {
        assert!(tz::valid_name(name), "{name}");
        if !file.is_empty() {
            assert_eq!(tz::canonical(file), Some((file, file)), "{name} → {file}");
        }
    }
    for bad in [
        "posixrules",
        "Factory",
        "posix/Europe/Berlin",
        "right/Europe/Berlin",
        "localtime",
        "Etc/Unknown",
        "SystemV/EST5EDT",
        "",
        "Europe/Berlin\0",
    ] {
        assert_eq!(tz::canonical(bad), None, "{bad:?}");
    }
}

/// Given a name in any ASCII case, an IANA link or an ICU legacy alias, when made
/// canonical, then it is the list's spelling and the IANA zone it reads.
#[test]
fn canonical_spelling() {
    for (name, want) in [
        ("Europe/Berlin", ("Europe/Berlin", "Europe/Berlin")),
        ("EUROPE/BERLIN", ("Europe/Berlin", "Europe/Berlin")),
        ("etc/gmt+3", ("Etc/GMT+3", "Etc/GMT+3")),
        ("asia/calcutta", ("Asia/Calcutta", "Asia/Calcutta")),
        ("systemv/ast4", ("SystemV/AST4", "Etc/GMT+4")),
        ("US/Pacific-New", ("US/Pacific-New", "America/Los_Angeles")),
        ("ist", ("IST", "Asia/Calcutta")),
    ] {
        assert_eq!(tz::canonical(name), Some(want), "{name}");
    }
}

/// Given offset names, then the ones `Intl` takes are fixed offsets and the rest are not
/// (Node 22: `+05`, `+0530`, `+05:30`, `-23:59`; not `+5`, `+24:00`, `+05:30:00`).
#[test]
fn offset_names() {
    for (name, want) in [
        ("+05:30", Some(19_800)),
        ("+0530", Some(19_800)),
        ("+05", Some(18_000)),
        ("-08:00", Some(-28_800)),
        ("-23:59", Some(-86_340)),
        ("+23:59", Some(86_340)),
        ("-00:00", Some(0)),
        ("+0000", Some(0)),
        ("+5", None),
        ("+05:3", None),
        ("+24:00", None),
        ("+05:60", None),
        ("+05:30:00", None),
        ("05:30", None),
        ("+05-30", None),
        ("+1", None),
        ("+ab", None),
        ("", None),
    ] {
        assert_eq!(tz::offset_name(name), want, "{name:?}");
    }
    assert_eq!(Zone::named("+0530").map(|z| z.offset_at(0)), Some(19_800));
    assert_eq!(
        Zone::named("+05:30")
            .map(|z| z.name().to_owned())
            .as_deref(),
        Some("+05:30")
    );
}

/// Given 20 000 spellings of one zone, when each is looked up, then all are that zone and
/// the cache holds one entry for it (it used to keep one per spelling: 1.9 → 22.7 MB).
#[test]
fn cache_is_bounded_by_real_zones() {
    let Some(berlin) = Zone::named("Europe/Berlin") else {
        eprintln!("no Europe/Berlin on this platform: skipped");
        return;
    };
    let base: Vec<u8> = b"europe/berlin".to_vec();
    for i in 0u32..20_000 {
        let spelled: String = base
            .iter()
            .enumerate()
            .map(|(j, &b)| {
                let up = (i >> (j % 14)) & 1 == 1 || (i / 7 + j as u32) % 3 == 0;
                char::from(if up { b.to_ascii_uppercase() } else { b })
            })
            .collect();
        let z = Zone::named(&spelled).unwrap_or_else(|| panic!("{spelled}"));
        assert_eq!(z.name(), berlin.name(), "{spelled}");
    }
    // other tests may cache other zones meanwhile: count only this one's keys
    let cache = tz::CACHE
        .get()
        .unwrap()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let keys: Vec<&str> = cache
        .keys()
        .map(|k| &**k)
        .filter(|k| k.eq_ignore_ascii_case("europe/berlin"))
        .collect();
    assert_eq!(keys, ["Europe/Berlin"]);
}
