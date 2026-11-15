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
            expect(codes.join(", "), "en (English), ru (Русский), ar (العربية)"),
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
    // M2: plurals by the CLDR rules of each locale's `iso` (en-US, ru-RU, ar-EG).
    let tc = |code: &str, key: &str, counts: &[i64]| {
        let view = i18n.with_locale(code);
        let forms: Vec<String> = counts.iter().map(|&n| view.tc(key, n)).collect();
        forms.join(" / ")
    };
    stops.extend([
        (
            "tc en",
            expect(
                tc("en", "apples", &[0, 1, 5]),
                "no apples / one apple / 5 apples",
            ),
        ),
        (
            "tc ru",
            expect(
                tc("ru", "apples", &[0, 1, 3, 11, 21]),
                "нет яблок / 1 яблоко / 3 яблока / 11 яблок / 21 яблоко",
            ),
        ),
        (
            "tc ar",
            expect(
                tc("ar", "apples", &[0, 1, 2, 3, 11, 100]),
                "لا تفاح / تفاحة واحدة / تفاحتان / 3 تفاحات / 11 تفاحة / 100 تفاحة",
            ),
        ),
        (
            "tc =0",
            expect(
                tc("ru", "cart", &[0, 2, 5]),
                "Корзина пуста / 2 товара / 5 товаров",
            ),
        ),
        (
            "tc_with",
            expect(i18n.tc_with("cart", 1, &Args::new()), "1 item"),
        ),
    ]);
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

    // M3: the same folder read lazily, a library's strings under the app's, subscribe and
    // reload (watch is left out: it polls a thread).
    let library: [(&str, &[u8]); 1] = [(
        "en.json",
        br#"{"lib": {"powered": "Powered by a library"}, "nav": {"home": "Library home"}}"#,
    )];
    let lazy = I18n::builder()
        .config(examples.join("i18n.json"))
        .lazy_dir(examples.join("locales"))
        .extend(library)
        .build()?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let _sub = lazy.subscribe(move |c| {
        log.lock()
            .unwrap()
            .push(format!("{}@{}", c.locale, c.version))
    });
    stops.extend([
        (
            "lazy",
            expect(
                format!(
                    "{} / {}",
                    lazy.pages().join(","),
                    lazy.page("settings").t("title")
                ),
                "settings / Settings",
            ),
        ),
        (
            "extend",
            expect(
                format!("{} / {}", lazy.t("lib.powered"), lazy.t("nav.home")),
                "Powered by a library / Home",
            ),
        ),
    ]);
    lazy.set_locale("ru")?;
    lazy.reload()?;
    stops.extend([
        (
            "reload",
            expect(
                format!("{} v{}", lazy.page("settings").t("title"), lazy.version()),
                "Настройки v2",
            ),
        ),
        (
            "subscribe",
            expect(seen.lock().unwrap().join(", "), "ru@1, ru@2"),
        ),
    ]);
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
