//! `tn` / `tn_as` / `td` / `td_as` / `tdr` / `tdr_with` / `language_name` on [`I18n`] and
//! [`View`], and the implicit-instance forms the macros use (DESIGN 4.2, 4.4, 5.3, 8b.14).

use super::data::{EN, LOCALES};
use super::{DateOptions, Loc, Number, NumberOptions, RelativeOptions, Timestamp, Zone};
use crate::i18n::{I18n, Snapshot, View};
use crate::scope::{self, Code};
use std::fmt;

impl Snapshot {
    /// The data locale and options of a number in `locale` (the active one when `None`):
    /// the named format `name` by the locale code, else (also when it is missing) the
    /// defaults. The flag says a name was asked for and not found.
    fn number_setup(
        &self,
        locale: Option<&str>,
        name: Option<&str>,
    ) -> (&'static Loc, &NumberOptions, bool) {
        let loc = locale.map_or(self.number, |c| self.number_of(c));
        let named = name.map(|n| self.meta.named_number(locale.unwrap_or(&self.locale), n));
        match named {
            Some(Some(o)) => (loc, o, false),
            Some(None) => (loc, &NumberOptions::DEFAULT, true),
            None => (loc, &NumberOptions::DEFAULT, false),
        }
    }
}

impl Snapshot {
    /// The data locale and options of a date in `locale` (the active one when `None`), as
    /// [`Snapshot::number_setup`]; the zone is the format's own, else the instance's.
    pub(crate) fn date_setup(
        &self,
        locale: Option<&str>,
        name: Option<&str>,
    ) -> (&'static Loc, &DateOptions, &Zone, bool) {
        let loc = locale.map_or(self.number, |c| self.number_of(c));
        let named = name.map(|n| self.meta.named_date(locale.unwrap_or(&self.locale), n));
        let (o, miss) = match named {
            Some(Some(o)) => (o, false),
            Some(None) => (&DateOptions::DEFAULT, true),
            None => (&DateOptions::DEFAULT, false),
        };
        (
            loc,
            o,
            o.zone.as_ref().unwrap_or_else(|| self.meta.zone()),
            miss,
        )
    }
}

/// JS `warnMissingFormat`: a named format that no locale of the chain of truncations has.
#[cold]
fn missing(name: &str, locale: &str) {
    crate::trace::missing_format("number", name, locale);
}

#[cold]
fn missing_date(name: &str, locale: &str) {
    crate::trace::missing_format("datetime", name, locale);
}

/// The system clock in milliseconds.
pub(crate) fn system_now() -> Option<i64> {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Timestamp::from(std::time::SystemTime::now()).millis()
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "tz"))]
    {
        Timestamp::from(js_sys::Date::now()).millis()
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "tz")))]
    {
        None
    }
}

/// A clock of [`Builder::clock`](crate::Builder::clock): milliseconds since 1970.
#[derive(Clone)]
pub(crate) struct Clock(pub(crate) std::sync::Arc<dyn Fn() -> i64 + Send + Sync>);

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Clock")
    }
}

impl I18n {
    /// Formats `n` like `Intl.NumberFormat` in the active locale (its `iso`, else its code),
    /// with the default options (JS `tn(value)`): `1234.5` is `1,234.5` in `en`,
    /// `1.234,5` in `de`, `١٬٢٣٤٫٥` in `ar-EG`. One allocation: the result. Feature
    /// `format`; the locales come from `I18N_NEXT_LOCALES` at build time (a locale left out
    /// formats as its nearest selected parent, else `en`).
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().iso("ar", "ar-EG").locale("ar").build()?;
    /// assert_eq!(i18n.tn(1234.5), "١٬٢٣٤٫٥");
    /// assert_eq!(i18n.with_locale("en").tn(1234.5), "1,234.5");
    /// assert_eq!(i18n.with_locale("de").tn(-0.0), "-0");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tn(&self, n: impl Into<Number>) -> String {
        self.number_string(None, n.into(), None)
    }

    /// Formats `n` with the named format `name` of the active locale (JS `tn(value, name)`,
    /// [`Builder::formats`](crate::Builder::formats) or `numberFormats` of `i18n.json`),
    /// looked up by the locale code, then shorter (`zh-Hant-TW` → `zh-Hant` → `zh`). A name
    /// nowhere to be found warns (debug builds, or `tracing`) and formats with the defaults,
    /// as JS does.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .formats(r#"{"en": {"usd": {"style": "currency", "currency": "USD"},
    ///                         "short": {"notation": "compact"},
    ///                         "pct": {"style": "percent", "maximumFractionDigits": 1}}}"#)
    ///     .build()?;
    /// assert_eq!(i18n.tn_as(1234.5, "usd"), "$1,234.50");
    /// assert_eq!(i18n.tn_as(1_234_567, "short"), "1.2M");
    /// assert_eq!(i18n.tn_as(0.1234, "pct"), "12.3%");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tn_as(&self, n: impl Into<Number>, name: &str) -> String {
        self.number_string(None, n.into(), Some(name))
    }

    pub(crate) fn number_string(
        &self,
        locale: Option<&str>,
        n: Number,
        name: Option<&str>,
    ) -> String {
        let (out, miss) = self.inner.read(|s| {
            let (loc, o, miss) = s.number_setup(locale, name);
            (super::to_string(n, o, loc), miss.then(|| s.locale.clone()))
        });
        if let (Some(active), Some(name)) = (miss, name) {
            missing(name, locale.unwrap_or(&active));
        }
        out
    }

    /// Writes `n` (named format `name`) in `locale` (the active one when `None`) to `out`.
    pub(crate) fn write_number(
        &self,
        locale: Option<&str>,
        n: Number,
        name: Option<&str>,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        let (r, miss) = self.inner.read(|s| {
            let (loc, o, miss) = s.number_setup(locale, name);
            (super::write(n, o, loc, out), miss.then(|| s.locale.clone()))
        });
        if let (Some(active), Some(name)) = (miss, name) {
            missing(name, locale.unwrap_or(&active));
        }
        r
    }

    /// Formats `t` like `Intl.DateTimeFormat` in the active locale (its `iso`, else its code)
    /// with the default options (JS `td(value)`): year, month and day, numeric
    /// (`1/5/2024` in `en`, `5.1.2024` in `de`, `٥‏/١‏/٢٠٢٤` in `ar-EG`), in the zone of
    /// [`Builder::time_zone`](crate::Builder::time_zone) (else the system's). An invalid
    /// date is `Invalid Date`. One allocation: the result. Feature `format`; Gregorian only.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().time_zone("UTC").iso("ar", "ar-EG").build()?;
    /// let at = 1_704_428_640_000i64; // 2024-01-05 04:24 UTC
    /// assert_eq!(i18n.td(at), "1/5/2024");
    /// assert_eq!(i18n.with_locale("de").td(at), "5.1.2024");
    /// assert_eq!(i18n.with_locale("ar").td(at), "٥‏/١‏/٢٠٢٤");
    /// assert_eq!(i18n.td(f64::NAN), "Invalid Date");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn td(&self, t: impl Into<Timestamp>) -> String {
        self.date_string(None, t.into(), None)
    }

    /// Takes the IANA zone `iana` as the system zone dates show in, as a front end knows it
    /// (the Tauri plugin passes the WebView's `Intl.DateTimeFormat().resolvedOptions()
    /// .timeZone` with its first `meta`, DESIGN 5.4). It is taken only when the builder set no
    /// [`time_zone`](crate::Builder::time_zone), the system zone was not looked up yet (a
    /// date already shown keeps its zone) and the zone is known; `true` when taken. A taken
    /// zone is sent to the front end with the payload meta as `timeZone`. Feature `format`.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().build()?;
    /// assert!(!i18n.set_system_time_zone("Mars/Olympus"));
    /// assert!(i18n.set_system_time_zone("Asia/Tokyo"));
    /// assert_eq!(i18n.td(1_704_481_200_000i64), "1/6/2024"); // 2024-01-05 19:00 UTC
    /// assert!(!i18n.set_system_time_zone("UTC")); // once
    /// assert!(!I18n::builder().time_zone("UTC").build()?.set_system_time_zone("Asia/Tokyo"));
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn set_system_time_zone(&self, iana: &str) -> bool {
        self.inner.read(|s| s.meta.set_system_zone(iana))
    }

    /// Formats `t` with the named date format `name` of the active locale (JS `td(value,
    /// name)`, [`Builder::datetime_formats`](crate::Builder::datetime_formats) or
    /// `datetimeFormats` of `i18n.json`), looked up by the locale code, then shorter
    /// (`zh-Hant-TW` → `zh-Hant` → `zh`). A name nowhere to be found warns (debug builds,
    /// or `tracing`) and formats with the defaults, as JS does.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder()
    ///     .time_zone("UTC")
    ///     .datetime_formats(r#"{"en": {"long": {"dateStyle": "full", "timeStyle": "short"},
    ///                                  "clock": {"hour": "2-digit", "minute": "2-digit", "hour12": false},
    ///                                  "berlin": {"timeStyle": "short", "timeZone": "Europe/Berlin"}}}"#)
    ///     .build()?;
    /// let at = 1_704_428_640_000i64; // 2024-01-05 04:24 UTC
    /// assert_eq!(i18n.td_as(at, "long"), "Friday, January 5, 2024 at 4:24 AM");
    /// assert_eq!(i18n.td_as(at, "clock"), "04:24");
    /// assert_eq!(i18n.td_as(at, "berlin"), "5:24 AM");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn td_as(&self, t: impl Into<Timestamp>, name: &str) -> String {
        self.date_string(None, t.into(), Some(name))
    }

    /// `t` relative to now (JS `tdr(value)`, `Intl.RelativeTimeFormat` with the default
    /// options): the largest unit that fits at least once (a year is 365 days, a month 30),
    /// rounded down: `3 days ago`, `in 2 hours`, `0 seconds ago` under a second in the past.
    /// Now is [`Builder::clock`](crate::Builder::clock), else the system clock. An invalid
    /// date (or no clock, or a clock outside a JS `Date`'s ±8.64e15 ms) is `Invalid Date`.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let now = 1_704_428_640_000i64;
    /// let i18n = I18n::builder().clock(move || now).build()?;
    /// assert_eq!(i18n.tdr(now - 3 * 86_400_000), "3 days ago");
    /// assert_eq!(i18n.tdr(now + 7_200_000), "in 2 hours");
    /// assert_eq!(i18n.with_locale("ru").tdr(now - 5 * 60_000), "5 минут назад");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tdr(&self, t: impl Into<Timestamp>) -> String {
        self.relative_string(None, t.into(), RelativeOptions::default())
    }

    /// [`I18n::tdr`] with `Intl.RelativeTimeFormat` options (JS `tdr(value, options)`).
    ///
    /// ```
    /// use i18n_next::{I18n, RelativeOptions};
    ///
    /// let now = 1_704_428_640_000i64;
    /// let i18n = I18n::builder().clock(move || now).build()?;
    /// let auto = RelativeOptions::default().auto();
    /// assert_eq!(i18n.tdr_with(now - 86_400_000, auto), "yesterday");
    /// assert_eq!(i18n.tdr_with(now - 3 * 3_600_000, RelativeOptions::default().short()), "3 hr. ago");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn tdr_with(&self, t: impl Into<Timestamp>, options: RelativeOptions) -> String {
        self.relative_string(None, t.into(), options)
    }

    pub(crate) fn date_string(
        &self,
        locale: Option<&str>,
        t: Timestamp,
        name: Option<&str>,
    ) -> String {
        let (out, miss) = self.inner.read(|s| {
            let (loc, o, zone, miss) = s.date_setup(locale, name);
            (
                super::string_of(|w| super::date::write(t, o, loc, zone, w)),
                miss.then(|| s.locale.clone()),
            )
        });
        if let (Some(active), Some(name)) = (miss, name) {
            missing_date(name, locale.unwrap_or(&active));
        }
        out
    }

    /// Writes `t` (named format `name`) in `locale` (the active one when `None`) to `out`.
    pub(crate) fn write_date(
        &self,
        locale: Option<&str>,
        t: Timestamp,
        name: Option<&str>,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        let (r, miss) = self.inner.read(|s| {
            let (loc, o, zone, miss) = s.date_setup(locale, name);
            (
                super::date::write(t, o, loc, zone, out),
                miss.then(|| s.locale.clone()),
            )
        });
        if let (Some(active), Some(name)) = (miss, name) {
            missing_date(name, locale.unwrap_or(&active));
        }
        r
    }

    pub(crate) fn relative_string(
        &self,
        locale: Option<&str>,
        t: Timestamp,
        o: RelativeOptions,
    ) -> String {
        super::string_of(|w| self.write_relative(locale, t, o, w))
    }

    /// Writes `t` relative to the clock in `locale` (the active one when `None`) to `out`.
    pub(crate) fn write_relative(
        &self,
        locale: Option<&str>,
        t: Timestamp,
        o: RelativeOptions,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        self.inner.read(|s| {
            let loc = locale.map_or(s.number, |c| s.number_of(c));
            match (t.millis(), s.meta.now()) {
                (Some(at), Some(now)) => super::relative::write(at, now, o, loc, out),
                _ => out.write_str("Invalid Date"),
            }
        })
    }

    /// The locale's own name (CLDR, 8b.14) of a locale code, by its `iso` (else the code):
    /// `de` → `Deutsch`, `ru` → `русский`, `pt-PT` → `português europeu`; a regional tag
    /// without its own data names its language (`de-AT` → `Österreichisches Deutsch`,
    /// `en-US` → `English`). `None` for a tag with no CLDR data in this build.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().iso("ua", "uk-UA").build()?;
    /// assert_eq!(i18n.language_name("de"), Some("Deutsch"));
    /// assert_eq!(i18n.language_name("ua"), Some("українська"));
    /// assert_eq!(i18n.language_name("xx"), None);
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn language_name(&self, code: &str) -> Option<&'static str> {
        self.inner
            .read(|s| super::locale::autonym(s.meta.tag(code)))
    }
}

impl View {
    /// [`I18n::td`] in this view's locale.
    #[must_use]
    pub fn td(&self, t: impl Into<Timestamp>) -> String {
        self.i18n
            .date_string(self.locale.as_deref(), t.into(), None)
    }

    /// [`I18n::td_as`] in this view's locale: the name is looked up by this locale's code.
    #[must_use]
    pub fn td_as(&self, t: impl Into<Timestamp>, name: &str) -> String {
        self.i18n
            .date_string(self.locale.as_deref(), t.into(), Some(name))
    }

    /// [`I18n::tdr`] in this view's locale.
    #[must_use]
    pub fn tdr(&self, t: impl Into<Timestamp>) -> String {
        self.i18n
            .relative_string(self.locale.as_deref(), t.into(), RelativeOptions::default())
    }

    /// [`I18n::tdr_with`] in this view's locale.
    #[must_use]
    pub fn tdr_with(&self, t: impl Into<Timestamp>, options: RelativeOptions) -> String {
        self.i18n
            .relative_string(self.locale.as_deref(), t.into(), options)
    }

    /// [`I18n::tn`] in this view's locale.
    #[must_use]
    pub fn tn(&self, n: impl Into<Number>) -> String {
        self.i18n
            .number_string(self.locale.as_deref(), n.into(), None)
    }

    /// [`I18n::tn_as`] in this view's locale: the name is looked up by this locale's code.
    #[must_use]
    pub fn tn_as(&self, n: impl Into<Number>, name: &str) -> String {
        self.i18n
            .number_string(self.locale.as_deref(), n.into(), Some(name))
    }
}

/// Writes `n` on `i18n` (else the nearest scope's instance, else the global one) in `locale`
/// (else the scope's, else the active one). Without any instance: `en` and the defaults.
pub(crate) fn write_in(
    i18n: Option<&I18n>,
    locale: Option<&str>,
    n: Number,
    name: Option<&str>,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    if let Some(i) = i18n {
        return i.write_number(locale, n, name, out);
    }
    let (scoped, scope_locale, ..) = scope::current(true, locale.is_none(), false);
    let locale = locale.or(scope_locale.as_ref().map(Code::as_str));
    match scoped.as_ref().or(scope::global_instance()) {
        Some(i) => i.write_number(locale, n, name, out),
        None => super::write(n, &NumberOptions::DEFAULT, &LOCALES[EN], out),
    }
}

/// [`I18n::tn`] on the implicit instance: the nearest [`scope`](crate::scope())'s, else the
/// global one, in the scope's locale (else the active one); without either, `en`. What
/// [`tn!`](crate::tn!) calls.
///
/// ```
/// use i18n_next::{I18n, scope, tn};
///
/// let i18n = I18n::builder().build()?;
/// scope().instance(i18n).locale("de").run(|| {
///     assert_eq!(tn!(1234.5), "1.234,5");
/// });
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn tn(n: impl Into<Number>) -> String {
    scoped_string(n.into(), None)
}

/// [`I18n::tn_as`] on the implicit instance (see [`tn()`]).
#[must_use]
pub fn tn_as(n: impl Into<Number>, name: &str) -> String {
    scoped_string(n.into(), Some(name))
}

fn scoped_string(n: Number, name: Option<&str>) -> String {
    let (scoped, locale, ..) = scope::current(true, true, false);
    let locale = locale.as_ref().map(Code::as_str);
    match scoped.as_ref().or(scope::global_instance()) {
        Some(i) => i.number_string(locale, n, name),
        None => super::to_string(n, &NumberOptions::DEFAULT, &LOCALES[EN]),
    }
}

/// A date or a relative date to show later: what [`Arg::date`](crate::Arg::date),
/// [`Arg::date_as`](crate::Arg::date_as), [`Arg::relative`](crate::Arg::relative) and
/// [`Text::date`](crate::Text::date) hold. Its formatter is a pointer set by those
/// constructors, so an app that never shows a date links none of the date code.
#[derive(Clone)]
pub struct DateArg<'a> {
    at: Timestamp,
    format: Option<std::borrow::Cow<'a, str>>,
    relative: bool,
    write: WriteWhen,
}

type WriteWhen = fn(Option<&I18n>, Option<&str>, &DateArg<'_>, &mut dyn fmt::Write) -> fmt::Result;

impl<'a> DateArg<'a> {
    pub(crate) fn date(at: Timestamp, format: Option<std::borrow::Cow<'a, str>>) -> Self {
        DateArg {
            at,
            format,
            relative: false,
            write: write_when_in,
        }
    }

    pub(crate) fn relative(at: Timestamp) -> Self {
        DateArg {
            at,
            format: None,
            relative: true,
            write: write_when_in,
        }
    }

    pub(crate) fn is_relative(&self) -> bool {
        self.relative
    }

    /// Writes it on `i18n` (else the nearest scope's instance, else the global one) in
    /// `locale` (else the scope's, else the active one).
    pub(crate) fn write_in(
        &self,
        i18n: Option<&I18n>,
        locale: Option<&str>,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        (self.write)(i18n, locale, self, out)
    }
}

impl fmt::Debug for DateArg<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("DateArg");
        d.field("at", &self.at);
        if let Some(name) = &self.format {
            d.field("format", name);
        }
        if self.relative {
            d.field("relative", &true);
        }
        d.finish()
    }
}

impl PartialEq for DateArg<'_> {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.relative, &self.format) == (other.at, other.relative, &other.format)
    }
}

/// [`DateArg::write_in`] for every date argument.
fn write_when_in(
    i18n: Option<&I18n>,
    locale: Option<&str>,
    d: &DateArg<'_>,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    let write = |i: &I18n, locale: Option<&str>, out: &mut dyn fmt::Write| {
        if d.relative {
            i.write_relative(locale, d.at, RelativeOptions::default(), out)
        } else {
            i.write_date(locale, d.at, d.format.as_deref(), out)
        }
    };
    if let Some(i) = i18n {
        return write(i, locale, out);
    }
    let (scoped, scope_locale, ..) = scope::current(true, locale.is_none(), false);
    let locale = locale.or(scope_locale.as_ref().map(Code::as_str));
    match scoped.as_ref().or(scope::global_instance()) {
        Some(i) => write(i, locale, out),
        None => bare(d, out),
    }
}

/// A date with no instance: `en`, defaults, UTC, the system clock.
fn bare(d: &DateArg<'_>, out: &mut dyn fmt::Write) -> fmt::Result {
    let loc = &LOCALES[EN];
    if !d.relative {
        return super::date::write(d.at, &DateOptions::DEFAULT, loc, &Zone::utc(), out);
    }
    match (d.at.millis(), system_now()) {
        (Some(at), Some(now)) => {
            super::relative::write(at, now, RelativeOptions::default(), loc, out)
        }
        _ => out.write_str("Invalid Date"),
    }
}

/// [`I18n::td`] on the implicit instance (see [`tn()`]); without one, `en` in UTC. What
/// [`td!`](crate::td!) calls.
///
/// ```
/// use i18n_next::{I18n, scope, td};
///
/// let i18n = I18n::builder().time_zone("UTC").build()?;
/// scope().instance(i18n).locale("ru").run(|| {
///     assert_eq!(td!(1_704_428_640_000i64), "05.01.2024");
/// });
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[must_use]
pub fn td(t: impl Into<Timestamp>) -> String {
    scoped_when(&DateArg::date(t.into(), None))
}

/// [`I18n::td_as`] on the implicit instance (see [`tn()`]).
#[must_use]
pub fn td_as(t: impl Into<Timestamp>, name: &str) -> String {
    scoped_when(&DateArg::date(t.into(), Some(name.into())))
}

/// [`I18n::tdr`] on the implicit instance (see [`tn()`]); without one, `en` and the system
/// clock. What [`tdr!`](crate::tdr!) calls.
#[must_use]
pub fn tdr(t: impl Into<Timestamp>) -> String {
    scoped_when(&DateArg::relative(t.into()))
}

fn scoped_when(d: &DateArg<'_>) -> String {
    super::string_of(|out| write_when_in(None, None, d, out))
}
