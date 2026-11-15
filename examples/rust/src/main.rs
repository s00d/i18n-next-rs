//! Console tour of i18n-next (DESIGN 8a): one line per feature, grows with each stage.
//!
//! `cargo run -p playground-rust` prints the tour; `-- --check` also verifies every
//! line and exits non-zero on any problem, including any key the catalog misses (CI runs it).

use i18n_next::{Args, Error, I18n};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

/// One stop of the tour: a label and what the crate returned, or why it is wrong.
type Stop = (&'static str, Result<String, String>);

/// `got` when it equals `want`, else the difference.
fn expect(got: String, want: &str) -> Result<String, String> {
    if got == want {
        Ok(got)
    } else {
        Err(format!("got {got:?}, want {want:?}"))
    }
}

fn tour(misses: &Arc<Mutex<Vec<String>>>) -> Result<Vec<Stop>, Error> {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let log = misses.clone();
    // M1: i18n.json (locales, fallback) + the shared locales folder with pages.
    let i18n = I18n::builder()
        .config(examples.join("i18n.json"))
        .dir(examples.join("locales"))
        .missing(move |locale, key, page| {
            log.lock().unwrap().push(format!("{locale}/{page}/{key}"))
        })
        .build()?;
    let settings = i18n.page("settings");
    let ann = Args::new().str("name", "Ann");
    let codes: Vec<String> = i18n
        .locales()
        .into_iter()
        .map(|l| format!("{} ({})", l.code, l.display_name.unwrap_or_default()))
        .collect();

    let version = i18n_next::version();
    let mut stops: Vec<Stop> = vec![
        (
            "version",
            expect(version.to_owned(), env!("CARGO_PKG_VERSION")),
        ),
        (
            "locales",
            expect(codes.join(", "), "en (English), ru (Русский)"),
        ),
        ("pages", expect(i18n.pages().join(", "), "settings")),
        ("t", expect(i18n.t("nav.home"), "Home")),
        (
            "t_with",
            expect(i18n.t_with("greeting", &ann), "Hello, Ann!"),
        ),
        ("page", expect(settings.t("title"), "Settings")),
        ("page→root", expect(settings.t("nav.settings"), "Settings")),
        (
            "ru",
            expect(
                i18n.with_locale("ru").t_with("greeting", &ann),
                "Привет, Ann!",
            ),
        ),
    ];
    i18n.set_locale("ru")?;
    stops.extend([
        ("set_locale", expect(settings.t("language"), "Язык")),
        ("fallback", expect(i18n.t("footer"), "Made with i18n-next")),
        (
            "has",
            expect(
                format!("{} / {}", i18n.has("nav.home"), i18n.has("footer")),
                "true / false",
            ),
        ),
        ("try_t", expect(format!("{:?}", i18n.try_t("nav")), "None")),
    ]);
    i18n.add_messages("ru", Some("settings"), r#"{"saved": "Сохранено"}"#)?;
    stops.push((
        "add",
        expect(
            format!("{} v{}", settings.t("saved"), i18n.version()),
            "Сохранено v2",
        ),
    ));
    let refused = match i18n.set_locale("../../etc/passwd") {
        Err(e) => e.to_string(),
        Ok(()) => "accepted".to_owned(),
    };
    stops.push((
        "bad name",
        expect(refused, "unknown locale \"../../etc/passwd\""),
    ));
    Ok(stops)
}

fn main() -> ExitCode {
    let check = std::env::args().skip(1).any(|a| a == "--check");
    let misses = Arc::new(Mutex::new(Vec::new()));
    let stops = match tour(&misses) {
        Ok(stops) => stops,
        Err(e) => {
            eprintln!("cannot load the catalog: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = 0;
    for (label, result) in stops {
        match result {
            Ok(text) => println!("{label:>10}  {text}"),
            Err(problem) => {
                failed += 1;
                eprintln!("{label:>10}  FAIL: {problem}");
            }
        }
    }
    for miss in misses.lock().unwrap().iter() {
        failed += 1;
        eprintln!("   missing  FAIL: {miss}");
    }
    if check && failed > 0 {
        eprintln!("--check: {failed} problem(s)");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
