//! `main` of the allocation tests (Cargo `harness = false`, DESIGN 7.7).
//!
//! dhat counts the allocations of the whole process. Under libtest the test runs on a
//! thread of its own while libtest's main thread, right after starting it, allocates its
//! bookkeeping (the map of running tests, the timeout queue, the channel wait): on a slow
//! runner those blocks landed inside a measurement (CI saw `(7, 950)` for `(3, 50)`).
//! Here the test runs on the main thread and nothing else allocates.
//!
//! It speaks the part of libtest's command line that `cargo test` and nextest use:
//! `--list --format terse [--ignored]`, and a name filter (`--exact` or a substring).

/// Runs `test` named `name`, or lists it.
pub fn run(name: &str, test: fn()) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            println!("{name}: test");
        }
        return;
    }
    // libtest flags that take a value; anything else not starting with `-` is the filter
    let mut filter = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if [
            "--format",
            "--test-threads",
            "--color",
            "--skip",
            "--logfile",
        ]
        .contains(&a.as_str())
        {
            it.next();
        } else if !a.starts_with('-') {
            filter = Some(a.as_str());
        }
    }
    let selected = match filter {
        None => true,
        Some(f) if has("--exact") => f == name,
        Some(f) => name.contains(f),
    };
    if has("--ignored") || !selected {
        println!("test result: ok. 0 passed; 0 failed; 1 filtered out");
        return;
    }
    test();
    println!("test {name} ... ok");
}
