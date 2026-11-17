//! The same workloads across i18n-next, rust-i18n, fluent-bundle and ICU4X (DESIGN 7.7).
//! Ids are `versus/<workload>/<lib>[_case]`; `scripts/bench-table.mjs` turns them into the
//! comparison table of docs/content/5.performance.md.
//!
//! Each library is used the way its docs show, and every setup asserts each library's
//! output before anything is timed: the same text, or on a miss what that library returns
//! for one. What differs, and so what the numbers mean:
//!
//! - **rust-i18n** (`i18n!` + `t!`) bakes `locales/*.json` into the binary at compile time;
//!   on the first `t!` the generated code fills a `SimpleBackend` (a `HashMap` per locale)
//!   from those static strings. `t!` returns a `Cow<str>` (borrowed on a hit without
//!   arguments); arguments fill `%{name}` by a scan of the text on every call (no parsed
//!   template). No plurals and no
//!   number or date formats. `cold` measures that first fill (`SimpleBackend::add_translations`
//!   from the flat list of 1k / 10k pairs), the only runtime load it has.
//! - **fluent-bundle** parses FTL into a `FluentResource` and indexes it in a
//!   `FluentBundle` per locale. A lookup is `get_message` + `format_pattern`, which returns
//!   a `Cow<str>` (borrowed when the pattern has no placeables). A miss falls back to the
//!   next bundle the way `fluent-fallback` walks its bundles. Bidi isolation marks are off
//!   (`set_use_isolating(false)`) so the text matches the others. Fluent ids cannot hold
//!   dots: `s3.g4.k5` is `s3-g4-k5`. Fluent's own number formatting does not group digits,
//!   so it is not in the number rows.
//! - **ICU4X** (`icu` 2, compiled data) has no message catalog: it is in the plural row
//!   (`PluralRules::category_for`, the category alone, no message), and in the number and
//!   date rows (`DecimalFormatter`, `DateTimeFormatter`), formatters built once outside
//!   the loop. Its date input is a calendar `Date` built outside the loop, while i18n-next
//!   gets a UTC millisecond timestamp and converts it through its zone on every call. Its
//!   number input is converted from the same `f64` inside the loop (`Decimal::try_from_f64`).
//! - **i18n-next** reads JSON at runtime into its own tables (`I18n::builder().messages`);
//!   `t` / `t_with` / `tc` / `tn` / `td` return an owned `String` (one allocation, DESIGN
//!   7.7). A miss walks the `de → en` fallback chain like the others.
//!
//! Memory is in the `memory` bench (dhat; a separate binary so its allocator does not slow
//! these).

use compare::{ftl, json, pairs};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use fixed_decimal::{Decimal, FloatPrecision};
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use i18n_next::{Args, I18n};
use icu::calendar::Date;
use icu::datetime::DateTimeFormatter;
use icu::datetime::fieldsets::YMD;
use icu::datetime::options::YearStyle;
use icu::decimal::DecimalFormatter;
use icu::locale::locale;
use icu::plurals::{PluralCategory, PluralRules};
use rust_i18n::{Backend, SimpleBackend, t};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hint::black_box;
use unic_langid::langid;

rust_i18n::i18n!("locales", fallback = "en");

/// 2024-01-05 04:24 UTC, the date of the main crate's date benches.
const AT: i64 = 1_704_428_640_000;

fn i18n_next() -> I18n {
    I18n::builder()
        .config_json(r#"{"locales": [{"code": "en"}, {"code": "de"}, {"code": "ru"}]}"#)
        .locale("de")
        .fallback("en")
        .time_zone("UTC")
        .messages(
            "en",
            None,
            &json(
                &pairs(1000, "en"),
                r#""only_en": "x", "hello": "Hello, {name}!""#,
            ),
        )
        .messages(
            "de",
            None,
            &json(&pairs(1000, "de"), r#""hello": "Hallo, {name}!""#),
        )
        .messages(
            "ru",
            None,
            r#"{"apples": "нет яблок | {count} яблоко | {count} яблока | {count} яблок"}"#,
        )
        .datetime_formats(r#"{"en": {"long": {"dateStyle": "long"}}}"#)
        .build()
        .unwrap()
}

/// `de` first, `en` behind it: what `fluent-fallback` walks.
struct Fluent {
    chain: Vec<FluentBundle<FluentResource>>,
    ru: FluentBundle<FluentResource>,
}

impl Fluent {
    fn new() -> Self {
        let bundle = |lang, src: String| {
            let mut b = FluentBundle::new(vec![lang]);
            b.set_use_isolating(false);
            b.add_resource(FluentResource::try_new(src).unwrap())
                .unwrap();
            b
        };
        let de = bundle(
            langid!("de"),
            ftl(&pairs(1000, "de"), "hello = Hallo, { $name }!\n"),
        );
        let en = bundle(
            langid!("en"),
            ftl(
                &pairs(1000, "en"),
                "only_en = x\nhello = Hello, { $name }!\n",
            ),
        );
        let ru = bundle(
            langid!("ru"),
            "apples = { $count ->\n    [one] { $count } яблоко\n    [few] { $count } яблока\n   *[many] { $count } яблок\n}\n"
                .into(),
        );
        Fluent {
            chain: vec![de, en],
            ru,
        }
    }

    fn format<'a>(&'a self, id: &str, args: Option<&FluentArgs>) -> Option<Cow<'a, str>> {
        self.chain.iter().find_map(|b| {
            let pattern = b.get_message(id)?.value()?;
            Some(b.format_pattern(pattern, args, &mut Vec::new()))
        })
    }

    fn plural<'a>(&'a self, args: &FluentArgs) -> Cow<'a, str> {
        let pattern = self.ru.get_message("apples").unwrap().value().unwrap();
        self.ru.format_pattern(pattern, Some(args), &mut Vec::new())
    }
}

fn lookup(c: &mut Criterion) {
    let ours = i18n_next();
    let fluent = Fluent::new();
    // same texts before timing
    assert_eq!(
        ours.t("s3.g4.k5", str::to_owned),
        t!("s3.g4.k5", locale = "de")
    );
    assert_eq!(
        ours.t("s3.g4.k5", str::to_owned),
        fluent.format("s3-g4-k5", None).unwrap()
    );
    assert_eq!(
        ours.t("only_en", str::to_owned),
        t!("only_en", locale = "de")
    );
    assert_eq!(
        ours.t("only_en", str::to_owned),
        fluent.format("only_en", None).unwrap()
    );

    c.bench_function("versus/lookup_hit/i18n_next", |b| {
        b.iter(|| ours.t(black_box("s3.g4.k5"), |t| black_box(t).len()))
    });
    c.bench_function("versus/lookup_hit/rust_i18n", |b| {
        b.iter(|| t!(black_box("s3.g4.k5"), locale = "de"))
    });
    c.bench_function("versus/lookup_hit/fluent", |b| {
        b.iter(|| fluent.format(black_box("s3-g4-k5"), None))
    });
    // missing in `de`, found in `en`
    c.bench_function("versus/lookup_fallback/i18n_next", |b| {
        b.iter(|| ours.t(black_box("only_en"), |t| black_box(t).len()))
    });
    c.bench_function("versus/lookup_fallback/rust_i18n", |b| {
        b.iter(|| t!(black_box("only_en"), locale = "de"))
    });
    c.bench_function("versus/lookup_fallback/fluent", |b| {
        b.iter(|| fluent.format(black_box("only_en"), None))
    });
    // missing everywhere: each returns its miss (the key, for rust-i18n too; `None` for
    // fluent)
    assert_eq!(ours.t("s3.g4.nope", str::to_owned), "s3.g4.nope");
    assert_eq!(t!("s3.g4.nope", locale = "de"), "s3.g4.nope");
    assert_eq!(fluent.format("s3-g4-nope", None), None);
    c.bench_function("versus/lookup_miss/i18n_next", |b| {
        b.iter(|| ours.t(black_box("s3.g4.nope"), |t| black_box(t).len()))
    });
    c.bench_function("versus/lookup_miss/rust_i18n", |b| {
        b.iter(|| t!(black_box("s3.g4.nope"), locale = "de"))
    });
    c.bench_function("versus/lookup_miss/fluent", |b| {
        b.iter(|| fluent.format(black_box("s3-g4-nope"), None))
    });
}

fn interpolation(c: &mut Criterion) {
    let ours = i18n_next();
    let fluent = Fluent::new();
    let args = Args::new().str("name", "Ann");
    let mut fargs = FluentArgs::new();
    fargs.set("name", "Ann");
    assert_eq!(ours.t_with("hello", &args), "Hallo, Ann!");
    assert_eq!(t!("hello", locale = "de", name = "Ann"), "Hallo, Ann!");
    assert_eq!(fluent.format("hello", Some(&fargs)).unwrap(), "Hallo, Ann!");

    c.bench_function("versus/interpolate_1/i18n_next", |b| {
        b.iter(|| ours.t_with(black_box("hello"), &args))
    });
    c.bench_function("versus/interpolate_1/rust_i18n", |b| {
        b.iter(|| t!(black_box("hello"), locale = "de", name = black_box("Ann")))
    });
    c.bench_function("versus/interpolate_1/fluent", |b| {
        b.iter(|| fluent.format(black_box("hello"), Some(&fargs)))
    });
}

fn plural(c: &mut Criterion) {
    let ours = i18n_next();
    let ru = ours.with_locale("ru");
    let fluent = Fluent::new();
    let icu = PluralRules::try_new(locale!("ru").into(), Default::default()).unwrap();
    let mut fargs = FluentArgs::new();
    fargs.set("count", 21);
    assert_eq!(ru.tc("apples", 21), "21 яблоко");
    assert_eq!(fluent.plural(&fargs), "21 яблоко");
    assert_eq!(icu.category_for(21), PluralCategory::One);

    // the message with its form picked and `{count}` filled
    c.bench_function("versus/plural_message/i18n_next", |b| {
        b.iter(|| ru.tc(black_box("apples"), black_box(21)))
    });
    c.bench_function("versus/plural_message/fluent", |b| {
        b.iter(|| fluent.plural(black_box(&fargs)))
    });
    // the CLDR category alone (ICU4X has no messages): i18n-next's is inside `tc`, so
    // this row shows ICU4X's share of the work above
    c.bench_function("versus/plural_category/icu4x", |b| {
        b.iter(|| icu.category_for(black_box(21)))
    });
}

fn number(c: &mut Criterion) {
    let ours = i18n_next().with_locale("en");
    let icu = DecimalFormatter::try_new(locale!("en").into(), Default::default()).unwrap();
    let icu_f64 = |x: f64| {
        icu.format(&Decimal::try_from_f64(x, FloatPrecision::RoundTrip).unwrap())
            .to_string()
    };
    assert_eq!(ours.tn(1234567.891), icu_f64(1234567.891));
    assert_eq!(
        ours.tn(1234567),
        icu.format(&Decimal::from(1234567)).to_string()
    );

    c.bench_function("versus/number_float/i18n_next", |b| {
        b.iter(|| ours.tn(black_box(1234567.891)))
    });
    c.bench_function("versus/number_float/icu4x", |b| {
        b.iter(|| icu_f64(black_box(1234567.891)))
    });
    c.bench_function("versus/number_int/i18n_next", |b| {
        b.iter(|| ours.tn(black_box(1234567)))
    });
    c.bench_function("versus/number_int/icu4x", |b| {
        b.iter(|| icu.format(&Decimal::from(black_box(1234567))).to_string())
    });
}

fn date(c: &mut Criterion) {
    let ours = i18n_next().with_locale("en");
    let short = DateTimeFormatter::try_new(
        locale!("en").into(),
        YMD::short().with_year_style(YearStyle::Full),
    )
    .unwrap();
    let long = DateTimeFormatter::try_new(locale!("en").into(), YMD::long()).unwrap();
    let day = Date::try_new_iso(2024, 1, 5).unwrap();
    assert_eq!(ours.td(AT), short.format(&day).to_string());
    assert_eq!(ours.td_as(AT, "long"), long.format(&day).to_string());

    c.bench_function("versus/date_numeric/i18n_next", |b| {
        b.iter(|| ours.td(black_box(AT)))
    });
    c.bench_function("versus/date_numeric/icu4x", |b| {
        b.iter(|| short.format(black_box(&day)).to_string())
    });
    c.bench_function("versus/date_long/i18n_next", |b| {
        b.iter(|| ours.td_as(black_box(AT), black_box("long")))
    });
    c.bench_function("versus/date_long/icu4x", |b| {
        b.iter(|| long.format(black_box(&day)).to_string())
    });
}

/// Building a one-locale catalog of 1k / 10k keys from its source text. All three run in
/// `iter_batched`: fluent's owned source is cloned in the untimed setup, and every
/// catalog is dropped outside the timer.
fn cold(c: &mut Criterion) {
    for keys in [1_000, 10_000] {
        let p = pairs(keys, "en");
        let src_json = json(&p, "");
        let src_ftl = ftl(&p, "");
        let flat: Vec<(&'static str, &'static str)> = p
            .iter()
            .map(|(k, v)| -> (&'static str, &'static str) { (k.clone().leak(), v.clone().leak()) })
            .collect();
        let size = format!("{}k", keys / 1000);

        let ours = || {
            I18n::builder()
                .locale("en")
                .messages("en", None, black_box(&src_json))
                .build()
                .unwrap()
        };
        let rust_i18n = || {
            let mut backend = SimpleBackend::new();
            let map: HashMap<Cow<'static, str>, Cow<'static, str>> = black_box(&flat)
                .iter()
                .map(|&(k, v)| (Cow::Borrowed(k), Cow::Borrowed(v)))
                .collect();
            backend.add_translations(Cow::Borrowed("en"), map);
            backend
        };
        let fluent = |src: String| {
            let mut bundle: FluentBundle<FluentResource> = FluentBundle::new(vec![langid!("en")]);
            bundle
                .add_resource(FluentResource::try_new(src).unwrap())
                .unwrap();
            bundle
        };
        // the three catalogs answer the same key with the same text
        let want = "en text 0.0.7";
        assert_eq!(ours().t("s0.g0.k7", str::to_owned), want);
        assert_eq!(rust_i18n().translate("en", "s0.g0.k7").unwrap(), want);
        let bundle = fluent(src_ftl.clone());
        let pattern = bundle.get_message("s0-g0-k7").unwrap().value().unwrap();
        assert_eq!(bundle.format_pattern(pattern, None, &mut Vec::new()), want);

        c.bench_function(&format!("versus/cold_{size}/i18n_next"), |b| {
            b.iter_batched(|| (), |()| ours(), BatchSize::SmallInput)
        });
        // what the code generated by `i18n!` runs on the first `t!`
        c.bench_function(&format!("versus/cold_{size}/rust_i18n"), |b| {
            b.iter_batched(|| (), |()| rust_i18n(), BatchSize::SmallInput)
        });
        c.bench_function(&format!("versus/cold_{size}/fluent"), |b| {
            b.iter_batched(|| src_ftl.clone(), fluent, BatchSize::SmallInput)
        });
    }
}

criterion_group!(benches, lookup, interpolation, plural, number, date, cold);
criterion_main!(benches);
