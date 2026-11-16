//! criterion benches of the own `tz` module (DESIGN 5.4, 7.7). Ids are `group/function/case`.
//!
//! - `tz/offset_at/<zone>`: one offset of a cached zone; `table_2026` lands in the
//!   transitions (fat file range), `rule_2080` past them, on the POSIX rule, `posix_only`
//!   the rule alone, `fixed` a constant offset;
//! - `tz/named/warm`: `Zone::named` of a cached name (read lock, map hit, `Arc` clone);
//! - `tz/named/cold`: the uncached platform lookup behind it (file read and TZif parse, or
//!   the bundled tzdb where there are no files);
//! - `tz/civil_from_unix/spread`: calendar fields of instants spread over ±10000 years.
//!
//! The module is crate-private: it is included with `#[path]`, so the public API stays as
//! it is.

#[path = "../src/tz/mod.rs"]
#[allow(dead_code)]
mod tz;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use tz::Zone;

fn zone(name: &str) -> Zone {
    Zone::named(name).unwrap_or_else(|| panic!("zone {name} not found on this system"))
}

fn offset_at(c: &mut Criterion) {
    let ny = zone("America/New_York");
    // 2026-07-01 and 2080-07-01: inside the slim file's transitions vs. past them
    let mut g = c.benchmark_group("tz/offset_at");
    g.bench_function("table_2026", |b| {
        b.iter(|| ny.offset_at(black_box(1_782_864_000)))
    });
    g.bench_function("rule_2080", |b| {
        b.iter(|| ny.offset_at(black_box(3_487_017_600)))
    });
    g.bench_function("posix_only", |b| {
        let rule = tz::posix::Posix::parse("EST5EDT,M3.2.0,M11.1.0").unwrap();
        b.iter(|| rule.offset_at(black_box(3_487_017_600)));
    });
    g.bench_function("fixed", |b| {
        let z = Zone::fixed(19_800);
        b.iter(|| z.offset_at(black_box(1_782_864_000)));
    });
    g.finish();
}

fn named(c: &mut Criterion) {
    zone("Europe/Berlin");
    let mut g = c.benchmark_group("tz/named");
    g.bench_function("warm", |b| {
        b.iter(|| Zone::named(black_box("Europe/Berlin")))
    });
    g.bench_function("cold", |b| b.iter(|| tz::load(black_box("Europe/Berlin"))));
    g.finish();
}

fn civil(c: &mut Criterion) {
    let ts: Vec<i64> = (0..1024i64).map(|i| (i - 512) * 617_283_950).collect();
    c.bench_function("tz/civil_from_unix/spread", |b| {
        b.iter(|| {
            ts.iter()
                .map(|&t| tz::civil_from_unix(black_box(t)).day as u32)
                .sum::<u32>()
        });
    });
}

criterion_group!(benches, offset_at, named, civil);
criterion_main!(benches);
