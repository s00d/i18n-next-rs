//! `tn` / `tn_as` / `language_name` on [`I18n`] and [`View`], and the implicit-instance
//! forms the macros use (DESIGN 4.2, 4.4, 5.3, 8b.14).

use super::data::{EN, LOCALES};
use super::{Loc, Number, NumberOptions};
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

/// JS `warnMissingFormat`: a named format that no locale of the chain of truncations has.
#[cold]
fn missing(name: &str, locale: &str) {
    crate::trace::missing_format(name, locale);
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
