//! Allocations of the own JSON reader (DESIGN 2.2, 7.7; dhat in testing mode, one
//! profiler per process, so one test walks every case).
//!
//! The module is crate-private: it is included with `#[path]`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/json.rs"]
#[allow(dead_code)]
mod json;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Blocks allocated while `f` runs.
fn blocks(f: impl FnOnce()) -> u64 {
    blocks_bytes(f).0
}

/// Blocks and bytes allocated while `f` runs (a growth realloc counts as one more block).
fn blocks_bytes(f: impl FnOnce()) -> (u64, u64) {
    let before = dhat::HeapStats::get();
    f();
    let after = dhat::HeapStats::get();
    (
        after.total_blocks - before.total_blocks,
        after.total_bytes - before.total_bytes,
    )
}

fn drain(input: &[u8]) -> usize {
    let mut r = json::Reader::new(input);
    let mut n = 0;
    while let Some(ev) = r.next().unwrap() {
        std::hint::black_box(&ev);
        n += 1;
    }
    n
}

/// Scenario: a locale file is read event by event. Without escapes nothing is allocated
/// (strings borrow, the depth stack is a bit set, even at the depth limit); each string
/// with escapes costs exactly one block; an error allocates nothing. The writer reserves
/// once: quotes plus the text, more only for escapes.
#[test]
fn reader_allocations() {
    let _profiler = dhat::Profiler::builder().testing().build();

    let plain = "\u{feff}{\"nav\":{\"title\":\"Home\",\"items\":[\"caf\u{e9}\",1.5e3,true,null]},\"deep\":[[[[{}]]]]}";
    let deep = format!(
        "{}{}",
        "[".repeat(json::MAX_DEPTH),
        "]".repeat(json::MAX_DEPTH)
    );
    let escaped = r#"{"a\nb":"x\"y","plain":"z","u":"\u00e9\ud83d\ude00"}"#;

    // case: no escapes → 0 blocks.
    let n = blocks(|| assert_eq!(drain(plain.as_bytes()), 25));
    dhat::assert_eq!(n, 0);
    // case: 128 levels → 0 blocks.
    let n = blocks(|| assert_eq!(drain(deep.as_bytes()), 2 * json::MAX_DEPTH));
    dhat::assert_eq!(n, 0);
    // case: one key and two values with escapes → 3 blocks, one per decoded string, each
    // sized once (the run before the first escape + 16): 17 + 17 + 16 bytes, no regrowth.
    let n = blocks_bytes(|| assert_eq!(drain(escaped.as_bytes()), 8));
    dhat::assert_eq!(n, (3, 50));
    // case: an error → 0 blocks.
    let n = blocks(|| {
        let mut r = json::Reader::new(b"{\"a\":[1,]}");
        while let Ok(Some(_)) = r.next() {}
    });
    dhat::assert_eq!(n, 0);

    // case: write_str of plain text into an empty buffer → 1 block of len + 2 bytes.
    let text = "Welcome back, {name}! caf\u{e9}";
    let n = blocks_bytes(|| {
        let mut out = String::new();
        json::write_str(&mut out, text);
        assert_eq!(out.len(), text.len() + 2);
    });
    dhat::assert_eq!(n, (1, text.len() as u64 + 2));
}
