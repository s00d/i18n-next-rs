//! Stories of dates (DESIGN 5.3, 5.4): refused options, invalid dates, the injected clock,
//! named formats looked up by truncated codes, the `iso` of a locale, dates in a `Text`, in
//! arguments and the macros. Every instance pins its zone and clock: nothing reads real time.

// zones and CLDR data: features `format` and `tz` (defaults)
#![cfg(all(feature = "format", feature = "tz"))]

use i18n_next::{Arg, Error, I18n, RelativeOptions, scope, t, td, tdr};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

/// 2024-01-05 04:24:00 UTC, a Friday.
const AT: i64 = 1_704_428_640_000;
const DAY: i64 = 86_400_000;

/// Given named date formats with options outside the subset (or values `Intl` throws on),
/// when the catalog is built, then `build` fails naming the locale, the format and the
/// option; an unknown builder zone fails with `UnknownTimeZone`.
#[test]
fn unsupported_date_options_fail_the_build() {
    for (options, option) in [
        (r#"{"timeStyle": "full"}"#, "timeStyle"),
        (
            r#"{"dateStyle": "short", "timeStyle": "long"}"#,
            "timeStyle",
        ),
        (
            r#"{"hour": "numeric", "timeZoneName": "short"}"#,
            "timeZoneName",
        ),
        (r#"{"year": "numeric", "era": "short"}"#, "era"),
        (r#"{"hour": "numeric", "dayPeriod": "short"}"#, "dayPeriod"),
        (
            r#"{"second": "numeric", "fractionalSecondDigits": 2}"#,
            "fractionalSecondDigits",
        ),
        (r#"{"calendar": "buddhist"}"#, "calendar"),
        (r#"{"dateStyle": "short", "hour": "numeric"}"#, "dateStyle"),
        (r#"{"timeStyle": "short", "year": "numeric"}"#, "timeStyle"),
        (r#"{"month": "numerical"}"#, "month"),
        (r#"{"year": true}"#, "year"),
        (r#"{"hourCycle": "h25"}"#, "hourCycle"),
        (r#"{"timeZone": "Mars/Olympus"}"#, "timeZone"),
        (r#"{"numberingSystem": "x"}"#, "numberingSystem"),
    ] {
        let err = I18n::builder()
            .datetime_formats(&format!(r#"{{"en": {{"x": {options}}}}}"#))
            .build()
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("format \"x\" of \"en\": option \"{option}\" is not supported"),
            "{options}"
        );
    }
    // a numbering system other than the locale's default, from `i18n.json`
    let config = r#"{"locales": [{"code": "ar", "iso": "ar-EG"}],
        "datetimeFormats": {"ar": {"latn": {"numberingSystem": "latn"}}}}"#;
    assert!(matches!(
        I18n::builder().config_json(config).build(),
        Err(Error::UnsupportedFormat { option, .. }) if option == "numberingSystem"
    ));
    assert!(matches!(
        I18n::builder().time_zone("Mars/Olympus").build(),
        Err(Error::UnknownTimeZone(z)) if z == "Mars/Olympus"
    ));
    // values at their default and unknown keys are fine, as for `Intl`
    let ok = I18n::builder()
        .time_zone("utc")
        .datetime_formats(r#"{"en": {"x": {"calendar": "gregory", "localeMatcher": "lookup", "formatMatcher": "basic", "hour12": true, "foo": 1}}}"#)
        .build()
        .unwrap();
    assert_eq!(ok.td_as(AT, "x"), "1/5/2024");
}

/// Given dates JS cannot hold (`NaN`, past ±8.64e15 ms), when they are formatted, then
/// every form shows `Invalid Date`; the ends of the range still format.
#[test]
fn invalid_dates_show_invalid_date() {
    let i18n = I18n::builder()
        .time_zone("UTC")
        .clock(|| AT)
        .messages(
            "en",
            None,
            r#"{"due": "Due {date}", "seen": "Seen {relativeDate}"}"#,
        )
        .build()
        .unwrap();
    for bad in [f64::NAN, f64::INFINITY, 8.64e15 + 1.0] {
        assert_eq!(i18n.td(bad), "Invalid Date");
        assert_eq!(i18n.tdr(bad), "Invalid Date");
        assert_eq!(i18n.text("due").date(bad), "Due Invalid Date");
        assert_eq!(i18n.text("seen").relative(bad), "Seen Invalid Date");
    }
    assert_eq!(i18n.td(8_640_000_000_000_000i64), "9/13/275760");
    assert_eq!(i18n.td(-8_640_000_000_000_000i64), "4/20/271822");
    assert_eq!(i18n.td(-1i64), "12/31/1969");
}

/// Given a clock the test moves, when `tdr` and a `Text` with a relative date are shown
/// again, then they follow the clock: the unit grows, the past turns to `ago`, and under
/// a second in the past is `0 seconds ago`, as `formatRelativeTime` says.
#[test]
fn relative_dates_follow_the_injected_clock() {
    let now = Arc::new(AtomicI64::new(AT));
    let clock = now.clone();
    let i18n = I18n::builder()
        .clock(move || clock.load(Ordering::SeqCst))
        .messages("en", None, r#"{"seen": "Seen {relativeDate}"}"#)
        .messages("de", None, r#"{"seen": "Gesehen {relativeDate}"}"#)
        .build()
        .unwrap();
    let seen = i18n.text("seen").relative(AT);
    assert_eq!(i18n.tdr(AT), "in 0 seconds");
    now.store(AT + 500, Ordering::SeqCst);
    assert_eq!(i18n.tdr(AT), "0 seconds ago");
    now.store(AT + 90_000, Ordering::SeqCst);
    assert_eq!(seen, "Seen 1 minute ago");
    now.store(AT + 3 * DAY, Ordering::SeqCst);
    assert_eq!(seen, "Seen 3 days ago");
    i18n.set_locale("de").unwrap();
    assert_eq!(seen, "Gesehen vor 3 Tagen");
    now.store(AT - 400 * DAY, Ordering::SeqCst);
    assert_eq!(i18n.tdr(AT), "in 1 Jahr");
    let auto = RelativeOptions::default().auto();
    now.store(AT + DAY, Ordering::SeqCst);
    assert_eq!(i18n.tdr_with(AT, auto), "gestern");
    assert_eq!(
        i18n.with_locale("en").tdr_with(AT, auto.narrow()),
        "yesterday"
    );
    assert_eq!(
        i18n.with_locale("en")
            .tdr_with(AT - 5 * DAY, RelativeOptions::default().short()),
        "6 days ago"
    );
    // a clock past a JS `Date` (±8.64e15 ms) is no now: `Invalid Date`, the edge still works
    now.store(8_640_000_000_000_001, Ordering::SeqCst);
    assert_eq!(i18n.tdr(AT), "Invalid Date");
    now.store(8_640_000_000_000_000, Ordering::SeqCst);
    assert_ne!(i18n.tdr(AT), "Invalid Date");
}

/// Given named date formats under `zh`, `zh-Hant` and `de`, when a longer code formats with
/// a name, then the format of the longest listed truncation is used (`zh-Hant-TW` →
/// `zh-Hant`, `de-AT` → `de`); a name nowhere to be found formats with the defaults.
#[test]
fn named_date_formats_are_found_by_truncated_codes() {
    let i18n = I18n::builder()
        .time_zone("UTC")
        .datetime_formats(
            r#"{"zh": {"m": {"month": "long"}, "y": {"year": "numeric"}},
                "zh-Hant": {"m": {"month": "short"}},
                "de": {"d": {"weekday": "long", "day": "numeric", "month": "long"}}}"#,
        )
        .build()
        .unwrap();
    assert_eq!(i18n.with_locale("zh-Hant-TW").td_as(AT, "m"), "1月");
    assert_eq!(i18n.with_locale("zh-Hant-TW").td_as(AT, "y"), "2024年");
    assert_eq!(
        i18n.with_locale("de-AT").td_as(AT, "d"),
        "Freitag, 5. Jänner"
    );
    assert_eq!(i18n.with_locale("de-AT").td_as(AT, "missing"), "5.1.2024");
}

/// Given locale codes with an `iso` (`ua` → `uk-UA`, `ar` → `ar-EG`) and one `Intl` does
/// not know, when they format dates, then the data is the tag's (Arabic digits, Ukrainian
/// months) and the unknown code formats as `en`, as `FormatService` does.
#[test]
fn dates_use_the_iso_of_a_locale() {
    let i18n = I18n::builder()
        .time_zone("UTC")
        .clock(|| AT)
        .iso("ua", "uk-UA")
        .iso("ar", "ar-EG")
        .datetime_formats(r#"{"ua": {"long": {"dateStyle": "long"}}}"#)
        .build()
        .unwrap();
    assert_eq!(i18n.with_locale("ua").td_as(AT, "long"), "5 січня 2024 р.");
    assert_eq!(i18n.with_locale("ar").td(AT), "٥‏/١‏/٢٠٢٤");
    assert_eq!(i18n.with_locale("ar").tdr(AT - 2 * DAY), "قبل يومين");
    assert_eq!(i18n.with_locale("xx").td(AT), "1/5/2024");
}

/// Given a zone on the instance and one in a named format, when the same instant is shown,
/// then the format's zone wins and both cross DST as `Intl` does.
#[test]
fn zones_of_the_instance_and_of_a_format() {
    let i18n = I18n::builder()
        .time_zone("America/New_York")
        .datetime_formats(r#"{"en": {"berlin": {"timeStyle": "short", "timeZone": "Europe/Berlin"}, "here": {"timeStyle": "short"}}}"#)
        .build()
        .unwrap();
    // 2024-03-31 01:00 UTC: Berlin has just moved to summer time, New York already had
    let spring = 1_711_846_800_000i64;
    assert_eq!(i18n.td_as(spring, "berlin"), "3:00 AM");
    assert_eq!(i18n.td_as(spring - 1000, "berlin"), "1:59 AM");
    assert_eq!(i18n.td_as(spring, "here"), "9:00 PM");
}

/// Given zone names spelled as `Intl` takes them (any ASCII case, an offset, ICU's legacy
/// aliases), when used on the instance or in a named format, then they resolve as in JS;
/// names `Intl` rejects (`posixrules`, `Factory`, `posix/…`, `right/…`, `localtime`, bad
/// offsets) fail the build; an empty instance zone is no zone (the system's), as in
/// `FormatService`, while an empty format `timeZone` is an error, as in `Intl`.
#[test]
fn zone_names_as_intl_takes_them() {
    // 2024-07-01 12:00 UTC
    let summer = 1_719_835_200_000i64;
    let formats = r#"{"en": {"t": {"timeStyle": "short", "hourCycle": "h23"},
        "kolkata": {"timeStyle": "short", "hourCycle": "h23", "timeZone": "+0530"},
        "west": {"timeStyle": "short", "hourCycle": "h23", "timeZone": "-08:00"},
        "five": {"timeStyle": "short", "hourCycle": "h23", "timeZone": "+05"},
        "lower": {"timeStyle": "short", "hourCycle": "h23", "timeZone": "etc/gmt+3"},
        "ast4": {"timeStyle": "short", "hourCycle": "h23", "timeZone": "systemv/ast4"}}}"#;
    for zone in [
        "Europe/Berlin",
        "europe/berlin",
        "EUROPE/BERLIN",
        "eUrOpE/bErLiN",
    ] {
        let i18n = I18n::builder()
            .time_zone(zone)
            .datetime_formats(formats)
            .build()
            .unwrap_or_else(|e| panic!("{zone}: {e}"));
        assert_eq!(i18n.td_as(summer, "t"), "14:00", "{zone}");
        assert_eq!(i18n.td_as(summer, "kolkata"), "17:30");
        assert_eq!(i18n.td_as(summer, "west"), "04:00");
        assert_eq!(i18n.td_as(summer, "five"), "17:00");
        assert_eq!(i18n.td_as(summer, "lower"), "09:00");
        assert_eq!(i18n.td_as(summer, "ast4"), "08:00");
    }
    let pacific = I18n::builder()
        .time_zone("US/Pacific-New")
        .datetime_formats(formats)
        .build()
        .unwrap();
    assert_eq!(pacific.td_as(summer, "t"), "05:00");
    for bad in [
        "posixrules",
        "Factory",
        "posix/Europe/Berlin",
        "right/Europe/Berlin",
        "localtime",
        "+5",
        "+24:00",
        "+05:3",
        "+05:30:00",
        "05:30",
        "Etc/Unknown",
    ] {
        assert!(
            matches!(I18n::builder().time_zone(bad).build(), Err(Error::UnknownTimeZone(z)) if z == bad),
            "{bad}"
        );
        let json = format!(r#"{{"en": {{"x": {{"timeZone": "{bad}"}}}}}}"#);
        assert!(
            I18n::builder().datetime_formats(&json).build().is_err(),
            "{bad}"
        );
    }
    let unset = I18n::builder().time_zone("").build().unwrap();
    let system = I18n::builder().build().unwrap();
    assert_eq!(unset.td(summer), system.td(summer));
    let empty_format = r#"{"en": {"x": {"timeZone": ""}}}"#;
    assert!(
        I18n::builder()
            .datetime_formats(empty_format)
            .build()
            .is_err()
    );
}

/// Given a `Text` with a number, a date and a relative date, when it is shown, then the
/// number wins, then the date (JS checks `number`, `date`, `relativeDate` in that order);
/// dates also go in as arguments and through the macros, on the scope's instance.
#[test]
fn dates_in_texts_arguments_and_macros() {
    let i18n = I18n::builder()
        .time_zone("UTC")
        .clock(|| AT)
        .datetime_formats(r#"{"en": {"day": {"weekday": "long"}}}"#)
        .messages(
            "en",
            None,
            r#"{"all": "{number}|{date}|{relativeDate}", "due": "Due {at}", "on": "On {date}"}"#,
        )
        .build()
        .unwrap();
    let all = i18n.text("all").relative(AT).date(AT).number(7);
    assert_eq!(all, "7|{date}|{relativeDate}");
    assert_eq!(
        i18n.text("all").relative(AT - DAY).date(AT),
        "{number}|1/5/2024|{relativeDate}"
    );
    assert_eq!(
        i18n.text("all").relative(AT - DAY),
        "{number}|{date}|1 day ago"
    );
    assert_eq!(i18n.text("on").date_as(AT, "day"), "On Friday");
    assert_eq!(t!(i18n => "on", @date = AT), "On 1/5/2024");
    assert_eq!(
        t!(i18n => "all", @relative = AT + 2 * DAY),
        "{number}|{date}|in 2 days"
    );
    assert_eq!(
        t!(i18n => "due", at = Arg::date_as(AT, "day")),
        "Due Friday"
    );
    assert_eq!(
        t!(i18n => "due", at = Arg::relative(AT - 60_000)),
        "Due 1 minute ago"
    );
    scope().instance(i18n.clone()).locale("ru").run(|| {
        assert_eq!(td!(AT), "05.01.2024");
        assert_eq!(tdr!(AT - 3_600_000), "1 час назад");
        assert_eq!(td(AT), "05.01.2024");
        assert_eq!(tdr(AT + 3_600_000), "через 1 час");
    });
}

/// Given `time` date-times (offset, UTC, primitive), when they become a `Timestamp`, then
/// they are the same instant in whole milliseconds toward zero.
#[cfg(feature = "time")]
#[test]
fn timestamps_from_time() {
    use i18n_next::Timestamp;
    use time::{OffsetDateTime, UtcDateTime, UtcOffset};
    let ns = i128::from(AT) * 1_000_000;
    let at = OffsetDateTime::from_unix_timestamp_nanos(ns).unwrap();
    assert_eq!(Timestamp::from(at).millis(), Some(AT));
    let east = at.to_offset(UtcOffset::from_hms(5, 30, 0).unwrap());
    assert_eq!(Timestamp::from(east).millis(), Some(AT));
    assert_eq!(
        Timestamp::from(UtcDateTime::from_unix_timestamp_nanos(ns).unwrap()).millis(),
        Some(AT)
    );
    let primitive = time::PrimitiveDateTime::new(at.date(), at.time());
    assert_eq!(Timestamp::from(primitive).millis(), Some(AT));
    let before = OffsetDateTime::from_unix_timestamp_nanos(-400_000).unwrap();
    assert_eq!(Timestamp::from(before).millis(), Some(0), "toward zero");
    let i18n = I18n::builder().time_zone("UTC").build().unwrap();
    assert_eq!(i18n.td(at), "1/5/2024");
}

/// Given views in more locales the instance does not know than the cache of resolved
/// locales keeps (8), when each formats twice, then every answer is that locale's, the
/// same as an instance whose active locale it is.
#[test]
fn views_in_unknown_locales_keep_their_formats() {
    let codes = [
        "zh-Hant-TW",
        "de-CH",
        "ar-EG",
        "hi",
        "ja",
        "fr-CA",
        "pt-PT",
        "es-MX",
        "ru",
        "sw",
        "de-Latn-AT",
        "en-IN",
    ];
    let i18n = I18n::builder().time_zone("UTC").build().unwrap();
    for _ in 0..2 {
        for code in codes {
            let own = I18n::builder()
                .time_zone("UTC")
                .locale(code)
                .build()
                .unwrap();
            let view = i18n.with_locale(code);
            assert_eq!(view.td(AT), own.td(AT), "{code}");
            assert_eq!(view.tn(1_234_567.5), own.tn(1_234_567.5), "{code}");
        }
    }
}
