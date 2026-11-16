//! The own `tz` against `jiff` (dev-dependency only, DESIGN 2.3 / 5.4 / 7.3a):
//! - every TZif file of the system zoneinfo, looked up through `Zone::named` (the real
//!   platform path), × instants 1900–2100: each transition jiff knows (from the file and its
//!   POSIX footer) −1 s / at / +1 s, plus random instants → identical offsets;
//! - `civil_from_unix` / `days_from_civil` against jiff's calendar on random instants over
//!   jiff's whole range (±9999 years) and the edges.
//!
//! The module is crate-private: it is included with `#[path]`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use std::fs;
use std::path::Path;

/// 1900-01-01 and 2100-01-01 UTC.
const FROM: i64 = -2_208_988_800;
const TO: i64 = 4_102_444_800;

/// splitmix64: deterministic, no dependency.
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

/// The instants checked for one zone: jiff's transitions in `[from, to)` ± 1 s and
/// `random` random ones.
fn moments(oracle: &TimeZone, from: i64, to: i64, random: usize, rng: &mut Rng) -> Vec<i64> {
    let mut out: Vec<i64> = (0..random).map(|_| rng.range(from, to)).collect();
    let start = Timestamp::from_second(from - 1).unwrap();
    for tr in oracle.following(start) {
        let t = tr.timestamp().as_second();
        if t >= to {
            break;
        }
        out.extend([t - 1, t, t + 1]);
    }
    out
}

/// Compares `mine` with `oracle` on `moments`; mismatches go to `bad` (capped).
fn compare(
    name: &str,
    mine: impl Fn(i64) -> i32,
    oracle: &TimeZone,
    moments: &[i64],
    bad: &mut Vec<String>,
) {
    for &t in moments {
        let want = oracle
            .to_offset(Timestamp::from_second(t).unwrap())
            .seconds();
        let got = mine(t);
        if got != want && bad.len() < 30 {
            bad.push(format!("{name} at {t}: own {got}, jiff {want}"));
        }
    }
}

fn tzif_files(dir: &Path, base: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_str().unwrap();
        let rel = if base.is_empty() {
            name.to_owned()
        } else {
            format!("{base}/{name}")
        };
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            tzif_files(&path, &rel, out);
        } else if let Ok(bytes) = fs::read(&path)
            && bytes.starts_with(b"TZif")
        {
            out.push((rel, bytes));
        }
    }
}

/// Given every zone of the system zoneinfo (the first directory `Zone::named` searches that
/// exists), when the own lookup and jiff (`TimeZone::tzif` on the same bytes) answer the same
/// instants 1900–2100, then the offsets are identical.
#[test]
fn every_system_zone_matches_jiff() {
    let Some(dir) = tz::ZONEINFO.iter().map(Path::new).find(|d| d.is_dir()) else {
        // Windows: no TZif files; tests/tz_tzdb.rs covers the bundled data there
        eprintln!("no system zoneinfo directory: skipped");
        return;
    };
    let mut files = Vec::new();
    tzif_files(dir, "", &mut files);
    assert!(
        files.len() > 300,
        "only {} TZif files under {}",
        files.len(),
        dir.display()
    );
    let mut rng = Rng(0x5eed_1900_2100);
    let (mut bad, mut checked) = (Vec::new(), 0usize);
    for (name, bytes) in &files {
        let oracle =
            TimeZone::tzif(name, bytes).unwrap_or_else(|e| panic!("jiff rejects {name}: {e}"));
        // only names `Intl` takes (posixrules, Factory, posix/…, right/… are files, not zones)
        let Some((canon, file)) = tz::canonical(name) else {
            assert!(tz::Zone::named(name).is_none(), "{name} is no Intl zone");
            continue;
        };
        if file != canon {
            // an ICU alias: it reads another zone's file, compared there
            continue;
        }
        let zone = tz::Zone::named(name).unwrap_or_else(|| panic!("own lookup misses {name}"));
        assert_eq!(zone.name(), name);
        let ms = moments(&oracle, FROM, TO, 1000, &mut rng);
        checked += ms.len();
        compare(name, |t| zone.offset_at(t), &oracle, &ms, &mut bad);
    }
    eprintln!(
        "{} zones under {}, {checked} instants compared",
        files.len(),
        dir.display()
    );
    assert!(
        bad.is_empty(),
        "offsets differ from jiff:\n{}",
        bad.join("\n")
    );
}

/// Given random instants over jiff's whole range (about ±9999 years) and the edges, when
/// split into calendar fields, then year, month, day, time and weekday equal jiff's, and
/// `days_from_civil` of the date gives back the day number.
#[test]
fn civil_matches_jiff() {
    let (lo, hi) = (Timestamp::MIN.as_second(), Timestamp::MAX.as_second());
    let mut rng = Rng(0xc1f1);
    let mut ts: Vec<i64> = (0..300_000).map(|_| rng.range(lo, hi)).collect();
    ts.extend([
        lo,
        hi,
        0,
        -1,
        1,
        86_399,
        86_400,
        -86_400,
        -86_401,
        951_782_400,
        951_868_800,
        -62_135_596_800,
    ]);
    for t in ts {
        let z = Timestamp::from_second(t).unwrap().to_zoned(TimeZone::UTC);
        let c = tz::civil_from_unix(t);
        let want = (
            i64::from(z.year()),
            z.month() as u8,
            z.day() as u8,
            z.hour() as u8,
            z.minute() as u8,
            z.second() as u8,
            z.weekday().to_sunday_zero_offset() as u8,
        );
        assert_eq!(
            (
                c.year, c.month, c.day, c.hour, c.minute, c.second, c.weekday
            ),
            want,
            "at {t}"
        );
        assert_eq!(
            tz::days_from_civil(c.year, c.month, c.day),
            t.div_euclid(86_400),
            "at {t}"
        );
    }
}

/// Given instants far outside any calendar jiff knows (±i64), when split, then nothing
/// overflows and the round trip through `days_from_civil` still holds where it fits.
#[test]
fn civil_extremes_do_not_panic() {
    for t in [
        i64::MIN,
        i64::MIN + 1,
        i64::MAX,
        i64::MAX - 1,
        -(1 << 50),
        1 << 50,
    ] {
        let c = tz::civil_from_unix(t);
        assert!((1..=12).contains(&c.month) && (1..=31).contains(&c.day) && c.weekday < 7);
        if t.unsigned_abs() <= 1 << 50 {
            assert_eq!(
                tz::days_from_civil(c.year, c.month, c.day),
                t.div_euclid(86_400)
            );
        }
    }
}
