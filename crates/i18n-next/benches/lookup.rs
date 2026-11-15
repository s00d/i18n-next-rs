//! criterion benches of the lookup path. Every public function has a bench named
//! `group/function/case` (`tests/bench_inventory.rs` checks it).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn version(c: &mut Criterion) {
    c.bench_function("lookup/version/static", |b| {
        b.iter(|| black_box(i18n_next::version()))
    });
}

criterion_group!(benches, version);
criterion_main!(benches);
