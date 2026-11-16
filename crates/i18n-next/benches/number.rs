//! criterion benches of M5 and M6 (DESIGN 5.5, 7.7): `tn` per style (decimal, integer,
//! percent, currency by symbol and code, compact), a locale from the snapshot's table (warm)
//! against one resolved on every call (cold), `Text` with `{number}` written into a buffer,
//! the builder's format parsing and `language_name`; `td` by styles and by components (with
//! a glued date and time, a zone of `tz`), `tdr` per option, warm and cold locales, dates in
//! `Text` and `Arg`. Ids follow `tests/support/inventory.rs`.

use criterion::{Criterion, criterion_group, criterion_main};
use i18n_next::{Arg, I18n, RelativeOptions, Timestamp, scope, td, td_as, tdr, tn, tn_as};
use std::fmt::Write as _;
use std::hint::black_box;

const FORMATS: &str = r#"{"en": {"pct": {"style": "percent", "maximumFractionDigits": 1},
    "usd": {"style": "currency", "currency": "USD"},
    "code": {"style": "currency", "currency": "EUR", "currencyDisplay": "code"},
    "short": {"notation": "compact"}},
  "ar": {"egp": {"style": "currency", "currency": "EGP"}}}"#;

fn catalog() -> I18n {
    I18n::builder()
        .locale("en")
        .iso("ar", "ar-EG")
        .fallback("ar")
        .messages("en", None, r#"{"total": "Total: {number}"}"#)
        .formats(FORMATS)
        .build()
        .unwrap()
}

fn numbers(c: &mut Criterion) {
    let i18n = catalog();
    c.bench_function("i18n/tn/decimal", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(1234567.891)))
    });
    c.bench_function("i18n/tn/integer", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(1234567)))
    });
    c.bench_function("i18n/tn/long_fraction", |b| {
        b.iter(|| black_box(&i18n).tn(black_box(0.1 + 0.2)))
    });
    c.bench_function("i18n/tn_as/percent", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(0.1234), black_box("pct")))
    });
    c.bench_function("i18n/tn_as/currency", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(1234.5), black_box("usd")))
    });
    c.bench_function("i18n/tn_as/currency_code", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(-1234.5), black_box("code")))
    });
    c.bench_function("i18n/tn_as/compact", |b| {
        b.iter(|| black_box(&i18n).tn_as(black_box(1234567.0), black_box("short")))
    });
    // warm: a known locale (data looked up once per snapshot); cold: a code nobody
    // declared, its tag resolved on every call
    let ar = i18n.with_locale("ar");
    c.bench_function("view/tn/warm_arabic_digits", |b| {
        b.iter(|| black_box(&ar).tn(black_box(1234567.891)))
    });
    let cold = i18n.with_locale("zh-Hant-TW");
    c.bench_function("view/tn/cold_resolve", |b| {
        b.iter(|| black_box(&cold).tn(black_box(1234567.891)))
    });
    c.bench_function("view/tn_as/currency", |b| {
        b.iter(|| black_box(&ar).tn_as(black_box(1234.5), black_box("egp")))
    });
    let total = i18n.text("total").number(1234.5);
    let priced = i18n.text("total").number_as(1234.5, "usd");
    let mut buf = String::with_capacity(64);
    c.bench_function("text/number/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&total)).unwrap();
        })
    });
    c.bench_function("text/number/to_string", |b| {
        b.iter(|| black_box(&total).to_string())
    });
    c.bench_function("text/number_as/display", |b| {
        b.iter(|| {
            buf.clear();
            black_box(&priced).write_to(&mut buf).unwrap();
        })
    });
    c.bench_function("i18n/language_name/known", |b| {
        b.iter(|| black_box(&i18n).language_name(black_box("ar")))
    });
    c.bench_function("builder/formats/two_locales", |b| {
        b.iter(|| I18n::builder().formats(black_box(FORMATS)).build().unwrap())
    });
    scope().instance(i18n.clone()).locale("ar").run(|| {
        c.bench_function("global/tn/scope", |b| b.iter(|| tn(black_box(1234.5))));
        c.bench_function("global/tn_as/scope", |b| {
            b.iter(|| tn_as(black_box(1234.5), black_box("egp")))
        });
    });
}

const DATE_FORMATS: &str = r#"{"en": {"full": {"dateStyle": "full", "timeStyle": "short"},
    "short": {"dateStyle": "short"},
    "glued": {"weekday": "short", "month": "long", "day": "numeric", "hour": "numeric", "minute": "2-digit"},
    "clock": {"hour": "2-digit", "minute": "2-digit", "second": "2-digit", "hourCycle": "h23"},
    "berlin": {"timeStyle": "short", "timeZone": "Europe/Berlin"}},
  "ar": {"long": {"dateStyle": "long"}}}"#;
/// 2024-01-05 04:24 UTC.
const AT: i64 = 1_704_428_640_000;

fn dates(c: &mut Criterion) {
    let i18n = I18n::builder()
        .locale("en")
        .iso("ar", "ar-EG")
        .fallback("ar")
        .time_zone("America/New_York")
        .clock(|| AT)
        .messages(
            "en",
            None,
            r#"{"due": "Due {date}", "seen": "Seen {relativeDate}", "at": "At {at}"}"#,
        )
        .datetime_formats(DATE_FORMATS)
        .build()
        .unwrap();
    c.bench_function("i18n/td/default", |b| {
        b.iter(|| black_box(&i18n).td(black_box(AT)))
    });
    // the builder's zone wins: refused without a lookup
    c.bench_function("i18n/set_system_time_zone/builder_zone", |b| {
        b.iter(|| black_box(&i18n).set_system_time_zone(black_box("Asia/Tokyo")))
    });
    // taken: the zone is looked up and stored, once per catalog
    c.bench_function("i18n/set_system_time_zone/taken", |b| {
        b.iter_batched(
            || I18n::builder().build().unwrap(),
            |fresh| fresh.set_system_time_zone(black_box("Asia/Tokyo")),
            criterion::BatchSize::SmallInput,
        )
    });
    c.bench_function("i18n/td_as/style_full", |b| {
        b.iter(|| black_box(&i18n).td_as(black_box(AT), black_box("full")))
    });
    c.bench_function("i18n/td_as/style_short", |b| {
        b.iter(|| black_box(&i18n).td_as(black_box(AT), black_box("short")))
    });
    c.bench_function("i18n/td_as/components_glued", |b| {
        b.iter(|| black_box(&i18n).td_as(black_box(AT), black_box("glued")))
    });
    c.bench_function("i18n/td_as/components_clock", |b| {
        b.iter(|| black_box(&i18n).td_as(black_box(AT), black_box("clock")))
    });
    c.bench_function("i18n/td_as/zone_berlin", |b| {
        b.iter(|| black_box(&i18n).td_as(black_box(AT), black_box("berlin")))
    });
    c.bench_function("i18n/tdr/days", |b| {
        b.iter(|| black_box(&i18n).tdr(black_box(AT - 3 * 86_400_000)))
    });
    c.bench_function("i18n/tdr_with/auto_short", |b| {
        let o = RelativeOptions::default().auto().short();
        b.iter(|| black_box(&i18n).tdr_with(black_box(AT - 86_400_000), black_box(o)))
    });
    // warm: a known locale (data from the snapshot's table); cold: resolved on every call
    let ar = i18n.with_locale("ar");
    c.bench_function("view/td/warm_arabic_digits", |b| {
        b.iter(|| black_box(&ar).td(black_box(AT)))
    });
    let cold = i18n.with_locale("zh-Hant-TW");
    c.bench_function("view/td/cold_resolve", |b| {
        b.iter(|| black_box(&cold).td(black_box(AT)))
    });
    c.bench_function("view/td_as/style_long", |b| {
        b.iter(|| black_box(&ar).td_as(black_box(AT), black_box("long")))
    });
    c.bench_function("view/tdr/warm", |b| {
        b.iter(|| black_box(&ar).tdr(black_box(AT - 7_200_000)))
    });
    c.bench_function("view/tdr/cold_resolve", |b| {
        b.iter(|| black_box(&cold).tdr(black_box(AT - 7_200_000)))
    });
    c.bench_function("view/tdr_with/narrow", |b| {
        let o = RelativeOptions::default().narrow();
        b.iter(|| black_box(&ar).tdr_with(black_box(AT - 7_200_000), black_box(o)))
    });
    let due = i18n.text("due").date(AT);
    let due_as = i18n.text("due").date_as(AT, "full");
    let seen = i18n.text("seen").relative(AT - 60_000);
    let at = i18n.text("at").arg("at", Arg::date_as(AT, "short"));
    let mut buf = String::with_capacity(64);
    c.bench_function("text/date/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&due)).unwrap();
        })
    });
    c.bench_function("text/date_as/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&due_as)).unwrap();
        })
    });
    c.bench_function("text/relative/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&seen)).unwrap();
        })
    });
    c.bench_function("arg/date_as/display", |b| {
        b.iter(|| {
            buf.clear();
            write!(buf, "{}", black_box(&at)).unwrap();
        })
    });
    c.bench_function("arg/date/build", |b| b.iter(|| Arg::date(black_box(AT))));
    c.bench_function("arg/relative/build", |b| {
        b.iter(|| Arg::relative(black_box(AT)))
    });
    c.bench_function("timestamp/millis/f64", |b| {
        b.iter(|| Timestamp::from(black_box(1.7e12)).millis())
    });
    c.bench_function("timestamp/is_valid/nan", |b| {
        b.iter(|| Timestamp::from(black_box(f64::NAN)).is_valid())
    });
    c.bench_function("relative_options/auto/const", |b| {
        b.iter(|| black_box(RelativeOptions::default()).auto())
    });
    c.bench_function("relative_options/short/const", |b| {
        b.iter(|| black_box(RelativeOptions::default()).short())
    });
    c.bench_function("relative_options/narrow/const", |b| {
        b.iter(|| black_box(RelativeOptions::default()).narrow())
    });
    c.bench_function("builder/datetime_formats/two_locales", |b| {
        b.iter(|| {
            I18n::builder()
                .datetime_formats(black_box(DATE_FORMATS))
                .build()
                .unwrap()
        })
    });
    c.bench_function("builder/time_zone/named", |b| {
        b.iter(|| {
            I18n::builder()
                .time_zone(black_box("Europe/Berlin"))
                .build()
                .unwrap()
        })
    });
    c.bench_function("builder/clock/fixed", |b| {
        b.iter(|| I18n::builder().clock(|| AT).build().unwrap())
    });
    scope().instance(i18n.clone()).locale("ar").run(|| {
        c.bench_function("global/td/scope", |b| b.iter(|| td(black_box(AT))));
        c.bench_function("global/td_as/scope", |b| {
            b.iter(|| td_as(black_box(AT), black_box("long")))
        });
        c.bench_function("global/tdr/scope", |b| {
            b.iter(|| tdr(black_box(AT - 60_000)))
        });
    });
}

criterion_group!(benches, numbers, dates);
criterion_main!(benches);
