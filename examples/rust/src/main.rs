//! Console tour of i18n-next (DESIGN 8a): one line per feature, grows with each stage.
//!
//! `cargo run -p playground-rust` prints the tour; `-- --check` also verifies every
//! line and exits non-zero on any problem, including any key the catalog misses (CI runs it).
//! `-- --texts` prints what each page of `examples/nuxt-site` shows, per locale, as JSON: its
//! check compares the generated site with it.

use i18n_next::{Args, CheckItem, Error, I18n, Text, key, scope, t, tc};
use std::collections::BTreeMap;
use std::fmt;
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
    // M3: linked messages, resolved once at build.
    let i18n = I18n::builder()
        .config(examples.join("i18n.json"))
        .dir(examples.join("locales"))
        .linked_messages(true)
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
        (
            "pages",
            expect(i18n.pages().join(", "), "about, settings, tray"),
        ),
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
    // M3: linked messages (`@:brand`, `@.upper:{'nav.home'}`): a locale that lacks the message
    // reads the fallback's with its own words; check() lists what translators left out.
    stops.extend([
        ("linked", expect(i18n.t("footer"), "Made with i18n-next")),
        ("linked ru", expect(i18n.with_locale("ru").t("shout"), "ГЛАВНАЯ!")),
        ("check", expect(gaps(&i18n), "ar: page settings; ar/index/cart; ar/index/footer; ar/index/shout; ru/index/footer; ru/index/shout")),
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
                "about,settings,tray / Settings",
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

    // M4: one global instance, then `t!` anywhere; `key!` in an error enum; a `Text` kept in
    // a struct follows set_locale; scope per request; slots with fold; negotiate.
    let log = misses.clone();
    let app = I18n::builder()
        .config(examples.join("i18n.json"))
        .dir(examples.join("locales"))
        .messages(
            "en",
            None,
            r#"{"errors": {"not_found": "Not found: {what}"}, "terms": "I accept the {link}"}"#,
        )
        .messages(
            "ru",
            None,
            r#"{"errors": {"not_found": "Не найдено: {what}"}}"#,
        )
        .missing(move |locale, key, page| {
            log.lock().unwrap().push(format!("{locale}/{page}/{key}"))
        })
        .install()?;
    let header = Header {
        title: app.page("settings").text("title"),
    };
    let lost = TourError::NotFound("cart.json".to_owned());
    stops.extend([
        (
            "t!",
            expect(t!("greeting", name = "Ann").to_string(), "Hello, Ann!"),
        ),
        ("tc!", expect(tc!("apples", 3).to_string(), "3 apples")),
        (
            "html",
            expect(
                t!("greeting", name = "<b>Ann</b>", @html).to_string(),
                "Hello, &lt;b&gt;Ann&lt;/b&gt;!",
            ),
        ),
        (
            "key!",
            expect(
                format!("{} / {lost}", TourError::Home),
                "Home / Not found: cart.json",
            ),
        ),
        ("Text", expect(header.title.to_string(), "Settings")),
        (
            "scope",
            expect(
                scope()
                    .locale("ru")
                    .page("settings")
                    .run(|| t!("title").to_string()),
                "Настройки",
            ),
        ),
        (
            "fold",
            expect(
                t!("terms")
                    .fold(str::to_owned, &[("link", &|| "[terms]".to_owned())])
                    .concat(),
                "I accept the [terms]",
            ),
        ),
        (
            "negotiate",
            expect(
                i18n_next::negotiate("ru-RU,ru;q=0.9,en;q=0.5", ["en", "ru", "ar"])
                    .unwrap_or_default(),
                "ru",
            ),
        ),
    ]);
    i18n_next::set_locale("ru")?;
    stops.extend([
        ("Text ru", expect(header.title.to_string(), "Настройки")),
        ("key! ru", expect(lost.to_string(), "Не найдено: cart.json")),
    ]);
    // M5: numbers by CLDR 48 as `Intl.NumberFormat` shows them; named formats from
    // i18n.json (`numberFormats`), the locale's `iso` (`ar` → `ar-EG`: Arabic digits).
    let n = 1234567.891;
    stops.extend([
        (
            "tn en",
            expect(i18n.with_locale("en").tn(n), "1,234,567.891"),
        ),
        (
            "tn ru",
            expect(i18n.with_locale("ru").tn(n), "1\u{a0}234\u{a0}567,891"),
        ),
        (
            "tn ar",
            expect(i18n.with_locale("ar").tn(n), "١٬٢٣٤٬٥٦٧٫٨٩١"),
        ),
        (
            "tn hi",
            expect(i18n.with_locale("hi").tn(n), "12,34,567.891"),
        ),
        (
            "tn_as currency",
            expect(
                format!(
                    "{} / {}",
                    i18n.with_locale("en").tn_as(1234.5, "currency"),
                    i18n.with_locale("ru").tn_as(1234.5, "currency")
                ),
                "$1,234.50 / 1\u{a0}234,50\u{a0}₽",
            ),
        ),
        (
            "compact",
            expect(
                format!(
                    "{} / {}",
                    i18n.with_locale("en").tn_as(1234567, "short"),
                    i18n.with_locale("ru").tn_as(1234567, "short")
                ),
                "1.2M / 1,2\u{a0}млн",
            ),
        ),
        ("tn! ru", expect(i18n_next::tn!(1234.5), "1\u{a0}234,5")),
        (
            "Text number",
            expect(
                t!(
                    "greeting",
                    name = i18n_next::Arg::Number {
                        value: 0.5.into(),
                        format: None
                    }
                )
                .to_string(),
                "Привет, 0,5!",
            ),
        ),
        (
            "language_name",
            expect(
                ["en", "ru", "ar", "hi"]
                    .map(|c| i18n.language_name(c).unwrap_or("?"))
                    .join(", "),
                "English, русский, العربية (مصر), हिन्दी",
            ),
        ),
    ]);
    // M6: dates and relative time as `Intl` shows them (Gregorian, CLDR 48 patterns taken
    // from ICU); `datetimeFormats` of i18n.json, a zone and a clock pinned for the tour.
    let at = 1_704_428_640_000i64; // 2024-01-05 04:24 UTC
    let days = I18n::builder()
        .config(examples.join("i18n.json"))
        .iso("ja", "ja-JP")
        .time_zone("Europe/Berlin")
        .clock(move || at)
        .messages(
            "en",
            None,
            r#"{"due": "Due {date}", "seen": "Seen {relativeDate}"}"#,
        )
        .messages(
            "ru",
            None,
            r#"{"due": "Срок: {date}", "seen": "Был {relativeDate}"}"#,
        )
        .build()?;
    let on = |code: &str| days.with_locale(code).td(at);
    stops.extend([
        (
            "td en / ru / ar / ja",
            expect(
                format!("{} / {} / {} / {}", on("en"), on("ru"), on("ar"), on("ja")),
                "1/5/2024 / 05.01.2024 / ٥\u{200f}/١\u{200f}/٢٠٢٤ / 2024/1/5",
            ),
        ),
        (
            "td_as short",
            expect(days.with_locale("ru").td_as(at, "short"), "05.01.2024"),
        ),
        (
            "Text date",
            expect(
                days.text("due").date(at).locale("ru").to_string(),
                "Срок: 05.01.2024",
            ),
        ),
        (
            "tdr en / ru",
            expect(
                format!(
                    "{} / {}",
                    days.tdr(at - 3 * 86_400_000),
                    days.with_locale("ru").tdr(at + 7_200_000)
                ),
                "3 days ago / через 2 часа",
            ),
        ),
        (
            "tdr auto ar",
            expect(
                days.with_locale("ar").tdr_with(
                    at - 86_400_000,
                    i18n_next::RelativeOptions::default().auto(),
                ),
                "أمس",
            ),
        ),
        (
            "Text relative",
            expect(
                days.text("seen").relative(at - 60_000).to_string(),
                "Seen 1 minute ago",
            ),
        ),
    ]);
    // M10: the axum server (bin/server.rs) on 127.0.0.1:0: a request's `Accept-Language`
    // picks the locale of its scope, the answer is JSON of `Text`.
    stops.push((
        "server",
        expect(
            server(),
            r#"{"locale":"ru","texts":{"language":"Язык","title":"Настройки"}}"#,
        ),
    ));
    Ok(stops)
}

/// One request to the server, or what went wrong.
fn server() -> String {
    let run = || -> std::io::Result<String> {
        let i18n = playground_rust::catalog().map_err(std::io::Error::other)?;
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?
            .block_on(playground_rust::with_server(i18n, |addr| async move {
                let (status, body) =
                    playground_rust::fetch(addr, "/settings", "ru;q=0.9, en;q=0.5").await?;
                Ok(if status == 200 {
                    body
                } else {
                    format!("HTTP {status}")
                })
            }))
    };
    run().unwrap_or_else(|e| e.to_string())
}

/// `--texts`: `{locale: {page: {key: text}}}` of the pages `examples/nuxt-site` renders.
fn texts() -> Result<String, Error> {
    let i18n = playground_rust::catalog()?;
    let mut all = BTreeMap::new();
    for locale in i18n.locales() {
        let mut pages = BTreeMap::new();
        for page in playground_rust::PAGES {
            let texts: BTreeMap<&str, String> = playground_rust::page_texts(&i18n, page)
                .unwrap_or_default()
                .into_iter()
                .map(|(key, text)| (key, text.locale(locale.code.clone()).to_string()))
                .collect();
            pages.insert(page, texts);
        }
        all.insert(locale.code, pages);
    }
    Ok(serde_json::to_string_pretty(&all).unwrap_or_default())
}

/// UI state that keeps a translation: it shows the language of the moment.
struct Header {
    title: Text,
}

/// An app error with translated messages, the way `thiserror` would write them:
/// `#[error("{}", key!("nav.home"))]`, `#[error("{}", t!("errors.not_found", what = .0))]`.
#[derive(Debug)]
enum TourError {
    Home,
    NotFound(String),
}

impl fmt::Display for TourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TourError::Home => write!(f, "{}", key!("nav.home")),
            TourError::NotFound(what) => {
                write!(f, "{}", t!("errors.not_found", what = what.clone()))
            }
        }
    }
}

/// `check()` in one line: the known gaps of the example catalog (the fallback stops rely on
/// them); anything else it finds shows in full and fails the stop.
fn gaps(i18n: &I18n) -> String {
    let items: Vec<String> = i18n
        .check()
        .items
        .iter()
        .map(|item| match item {
            CheckItem::MissingPage { locale, page } => format!("{locale}: page {page}"),
            CheckItem::MissingKey { locale, page, key } => format!("{locale}/{page}/{key}"),
            other => other.to_string(),
        })
        .collect();
    items.join("; ")
}

fn main() -> ExitCode {
    if std::env::args().skip(1).any(|a| a == "--texts") {
        return match texts() {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("cannot load the catalog: {e}");
                ExitCode::FAILURE
            }
        };
    }
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
