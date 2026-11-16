//! Heap held by a one-locale catalog of 1k / 10k keys, per library (the catalogs of the
//! `cold` rows in compare.rs, built the same way). dhat counts the bytes live while the
//! catalog is alive minus those live before its source text was made: a source the library
//! does not keep is dropped before the count (i18n-next's JSON), one it keeps is counted (a
//! `FluentResource` owns its FTL string, so fluent's number includes it). rust-i18n's
//! strings are `'static` in the binary, here leaked before the baseline: its number is the
//! `HashMap` only.
//! Writes `$CRITERION_HOME/compare_memory.json` for scripts/bench-table.mjs.

use compare::{ftl, json, pairs};
use fluent_bundle::{FluentBundle, FluentResource};
use i18n_next::I18n;
use rust_i18n::SimpleBackend;
use std::borrow::Cow;
use std::collections::HashMap;
use std::hint::black_box;
use unic_langid::langid;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn live() -> u64 {
    dhat::HeapStats::get().curr_bytes as u64
}

/// Live heap bytes of what `build` returns, counted from before its input was made; `build`
/// consumes the input, so what it does not keep is freed before the count.
fn held<I, T>(input: impl FnOnce() -> I, build: impl FnOnce(I) -> T) -> u64 {
    let before = live();
    let kept = black_box(build(input()));
    let after = live();
    drop(kept);
    after.saturating_sub(before)
}

fn main() {
    // `cargo bench -- --test` (the smoke) and plain runs both measure: it takes milliseconds
    let _profiler = dhat::Profiler::builder().testing().build();
    let mut rows = Vec::new();
    for keys in [1_000usize, 10_000] {
        let p = pairs(keys, "en");
        let ours = held(
            || json(&p, ""),
            |src| {
                I18n::builder()
                    .locale("en")
                    .messages("en", None, &src)
                    .build()
                    .unwrap()
            },
        );
        // what `i18n!` bakes into the binary: leaked before the baseline, never counted
        let flat: Vec<(&'static str, &'static str)> = p
            .iter()
            .map(|(k, v)| -> (&'static str, &'static str) { (k.clone().leak(), v.clone().leak()) })
            .collect();
        let rust_i18n = held(
            || &flat,
            |flat| {
                let mut backend = SimpleBackend::new();
                let map: HashMap<Cow<'static, str>, Cow<'static, str>> = flat
                    .iter()
                    .map(|&(k, v)| (Cow::Borrowed(k), Cow::Borrowed(v)))
                    .collect();
                backend.add_translations(Cow::Borrowed("en"), map);
                backend
            },
        );
        let fluent = held(
            || ftl(&p, ""),
            |src| {
                let mut bundle: FluentBundle<FluentResource> =
                    FluentBundle::new(vec![langid!("en")]);
                bundle
                    .add_resource(FluentResource::try_new(src).unwrap())
                    .unwrap();
                bundle
            },
        );
        println!(
            "memory {keys} keys: i18n_next {ours} B, rust_i18n {rust_i18n} B, fluent {fluent} B"
        );
        rows.push(format!(
            "\"{}k\": {{\"i18n_next\": {ours}, \"rust_i18n\": {rust_i18n}, \"fluent\": {fluent}}}",
            keys / 1000
        ));
    }
    if let Ok(home) = std::env::var("CRITERION_HOME") {
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            format!("{home}/compare_memory.json"),
            format!("{{{}}}\n", rows.join(", ")),
        )
        .unwrap();
    }
}
