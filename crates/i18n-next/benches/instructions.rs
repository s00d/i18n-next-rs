//! gungraun (Valgrind, linux only): instruction counts of the hot-path scenarios
//! (DESIGN 7.7, level 2). The CI `perf` job runs it on the base commit with
//! `--save-baseline=base` and on the PR with `--baseline=base --callgrind-limits='ir=3%'`:
//! more than 3% extra instructions fails the job. Counts do not depend on machine load.
//!
//! Elsewhere the bench is an empty binary, so `cargo bench` / clippy work on every host.

#[cfg(target_os = "linux")]
use gungraun::{library_benchmark, library_benchmark_group, main};

#[cfg(target_os = "linux")]
#[library_benchmark]
fn version() -> &'static str {
    std::hint::black_box(i18n_next::version())
}

#[cfg(target_os = "linux")]
library_benchmark_group!(name = lookup; benchmarks = version);

fn main() {
    #[cfg(target_os = "linux")]
    main!(library_benchmark_groups = lookup);
}
