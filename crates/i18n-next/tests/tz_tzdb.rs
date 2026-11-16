//! The bundled tzdb (`src/tz/tzdb.bin`, `node scripts/tzdb.mjs`) against jiff's bundled
//! IANA data of the same release (`jiff-tzdb`, dev-dependency only): the same zone names,
//! and for every zone the same offsets 1970–2100 (transitions ± 1 s and random instants).
//! The bundle drops pre-1970 history by design, so earlier instants are not compared.
//! Runs with every feature set: the test reads the file itself, the `tzdb` feature only
//! decides whether the library compiles it in.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use jiff::Timestamp;
use jiff::tz::TimeZoneDatabase;
use std::collections::BTreeSet;

static DATA: &[u8] = include_bytes!("../src/tz/tzdb.bin");

/// 1970-01-01 and 2100-01-01 UTC.
const FROM: i64 = 0;
const TO: i64 = 4_102_444_800;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Given the header line, then it names the release jiff bundles (2026f) with its sha256.
#[test]
fn header_records_release_and_checksum() {
    let h = tz::tzdb::header(DATA).unwrap();
    let parts: Vec<&str> = h.split(' ').collect();
    assert_eq!(parts[..3], ["i18n-next", "tzdb", "2026f"], "{h}");
    let sha = parts[3].strip_prefix("sha256:").unwrap();
    assert!(
        sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "{h}"
    );
    assert!(parts[4].ends_with("tzdata2026f.tar.gz"), "{h}");
}

/// Given both bundles, then they hold the same zone names, sorted in the own one.
#[test]
fn same_names_as_jiff() {
    let mine = tz::tzdb::names(DATA);
    assert!(mine.windows(2).all(|w| w[0] < w[1]), "names not sorted");
    let db = TimeZoneDatabase::bundled();
    let theirs: BTreeSet<String> = db.available().map(|n| n.as_str().to_owned()).collect();
    let mine: BTreeSet<String> = mine.into_iter().map(str::to_owned).collect();
    assert_eq!(
        mine.difference(&theirs).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "only in the own bundle"
    );
    assert_eq!(
        theirs.difference(&mine).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "only in jiff's bundle"
    );
}

/// Given every zone of the bundle, when the own table and jiff answer instants 1970–2100
/// (jiff's transitions − 1 s / at / + 1 s and 1000 random), then the offsets are identical.
#[test]
fn every_bundled_zone_matches_jiff() {
    let db = TimeZoneDatabase::bundled();
    let mut rng = Rng(1970);
    let (mut bad, mut checked) = (Vec::new(), 0usize);
    let names = tz::tzdb::names(DATA);
    for name in &names {
        let table = tz::tzdb::find(DATA, name).unwrap_or_else(|| panic!("{name} does not decode"));
        let oracle = db.get(name).unwrap();
        let mut ts: Vec<i64> = (0..1000).map(|_| (rng.next() % TO as u64) as i64).collect();
        for tr in oracle.following(Timestamp::from_second(FROM).unwrap()) {
            let t = tr.timestamp().as_second();
            if t >= TO {
                break;
            }
            ts.extend([t - 1, t, t + 1].into_iter().filter(|&t| t >= FROM));
        }
        checked += ts.len();
        for t in ts {
            let want = oracle
                .to_offset(Timestamp::from_second(t).unwrap())
                .seconds();
            let got = table.offset_at(t);
            if got != want && bad.len() < 30 {
                bad.push(format!("{name} at {t}: own {got}, jiff {want}"));
            }
        }
    }
    eprintln!(
        "{} bundled zones, {checked} instants compared, {} bytes",
        names.len(),
        DATA.len()
    );
    assert!(
        bad.is_empty(),
        "offsets differ from jiff:\n{}",
        bad.join("\n")
    );
}

/// Given a damaged bundle (truncated anywhere), when a zone is looked up, then `None` or a
/// table, never a panic.
#[test]
fn damaged_bundle_never_panics() {
    for n in (0..DATA.len()).step_by(97) {
        let _ = tz::tzdb::find(&DATA[..n], "Europe/Berlin");
        let _ = tz::tzdb::names(&DATA[..n]);
    }
    assert!(tz::tzdb::find(DATA, "Nowhere/Zone").is_none());
}

/// Given every bundled zone, when asked past its last transition, then the years the table
/// evaluates ahead (to 2100) give what the rule gives, around every Jan 1 and DST change
/// of the range, at its end (2101-01-01) and beyond, in any order (the transition hint).
#[test]
fn years_ahead_match_the_rule() {
    let mut rng = Rng(2100);
    for name in tz::tzdb::names(DATA) {
        let table = tz::tzdb::find(DATA, name).unwrap();
        let Some(rule) = &table.rule else { continue };
        let last = table.times.last().copied().unwrap_or(i64::MIN);
        let mut ts: Vec<i64> = (1969..=2102)
            .flat_map(|y| {
                let jan1 = (y - 1970) * 31_556_952;
                [jan1 - 86_400, jan1, jan1 + 86_400 * 180]
            })
            .collect();
        ts.extend((0..300).map(|_| (rng.next() % 4_200_000_000) as i64));
        ts.extend([4_133_980_799, 4_133_980_800, 4_133_984_400, i64::MAX]);
        for &t in ts.iter().chain(ts.iter().rev()) {
            if t >= last {
                assert_eq!(table.offset_at(t), rule.offset_at(t), "{name} at {t}");
            }
        }
    }
}
