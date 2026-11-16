//! gungraun (Valgrind, linux only): instruction counts of the hot-path scenarios
//! (DESIGN 7.7, level 2). The CI `perf` job runs it on the base commit with
//! `--save-baseline=base` and on the PR with `--baseline=base --callgrind-limits='ir=3%'`:
//! more than 3% extra instructions fails the job. Counts do not depend on machine load.
//! The catalog is built in `setup`, which is not measured; each case warms the thread
//! cache once, so the counted call is the steady-state hot path.
//!
//! Elsewhere the bench is an empty binary, so `cargo bench` / clippy work on every host.
//! `main!` expands to the binary's own `fn main`, so it must stay at item level.

#[cfg(target_os = "linux")]
use gungraun::{library_benchmark, library_benchmark_group, main};
#[cfg(target_os = "linux")]
use i18n_next::{Args, I18n};
#[cfg(target_os = "linux")]
use std::hint::black_box;

#[cfg(target_os = "linux")]
fn catalog() -> I18n {
    let i18n = I18n::builder()
        .locale("de")
        .fallback("en")
        .messages("en", None, r#"{"nav": {"home": "Home"}, "only_en": "x"}"#)
        .messages(
            "de",
            None,
            r#"{"nav": {"home": "Startseite"}, "hi": "Hallo, {name}!"}"#,
        )
        .messages("de", Some("settings"), r#"{"title": "Einstellungen"}"#)
        .build()
        .unwrap();
    let _ = i18n.t("nav.home");
    i18n
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn version() -> &'static str {
    black_box(i18n_next::version())
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::root_hit(setup = catalog)]
fn t(i18n: I18n) -> i18n_next::Translation<'static> {
    black_box(i18n.t(black_box("nav.home")))
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::fallback_hit(setup = catalog)]
fn get(i18n: I18n) -> Option<std::sync::Arc<str>> {
    black_box(i18n.get(black_box("only_en")))
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::one_arg(setup = catalog)]
fn t_with(i18n: I18n) -> String {
    black_box(i18n.t_with(black_box("hi"), &Args::new().str("name", "Ann")))
}

#[cfg(target_os = "linux")]
fn plural_catalog() -> I18n {
    let i18n = I18n::builder()
        .locale("ru")
        .messages(
            "ru",
            None,
            r#"{"apples": "нет яблок | {count} яблоко | {count} яблока | {count} яблок"}"#,
        )
        .build()
        .unwrap();
    let _ = i18n.tc("apples", 1);
    i18n
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::unmarked_ru(setup = plural_catalog)]
fn tc(i18n: I18n) -> String {
    black_box(i18n.tc(black_box("apples"), black_box(21)))
}

#[cfg(target_os = "linux")]
library_benchmark_group!(name = lookup; benchmarks = version, t, get, t_with, tc);

#[cfg(target_os = "linux")]
main!(library_benchmark_groups = lookup);

#[cfg(not(target_os = "linux"))]
fn main() {}
