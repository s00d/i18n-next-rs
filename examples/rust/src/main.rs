//! Console tour of i18n-next (DESIGN 8a): one line per feature, grows with each stage.
//!
//! `cargo run -p playground-rust` prints the tour; `-- --check` also verifies every
//! line and exits non-zero on any problem (CI runs it).

use std::process::ExitCode;

/// One stop of the tour: a label and what the crate returned, or why it is wrong.
type Stop = (&'static str, Result<String, String>);

fn tour() -> Vec<Stop> {
    let version = i18n_next::version();
    vec![(
        "version",
        if version == env!("CARGO_PKG_VERSION") {
            Ok(version.to_owned())
        } else {
            Err(format!(
                "crate {version} != playground {}",
                env!("CARGO_PKG_VERSION")
            ))
        },
    )]
}

fn main() -> ExitCode {
    let check = std::env::args().skip(1).any(|a| a == "--check");
    let mut failed = 0;
    for (label, result) in tour() {
        match result {
            Ok(text) => println!("{label:>10}  {text}"),
            Err(problem) => {
                failed += 1;
                eprintln!("{label:>10}  FAIL: {problem}");
            }
        }
    }
    if check && failed > 0 {
        eprintln!("--check: {failed} problem(s)");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
