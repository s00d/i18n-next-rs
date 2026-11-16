//! Parameters of `t_with` (DESIGN 2.2, spike S4): up to four live inline, so building
//! them allocates nothing; a fifth and later go to a `Vec`. Lookup is a linear scan from
//! the newest, which beats hashing for the real 1–4 parameters (spike S3).

#[cfg(feature = "format")]
use crate::json::push_js_number;
use crate::json::{Out, push_u64, u64_len};
use crate::plural::{Whole, shortest_whole};
use crate::text::{Key, MAX_NESTING, Text};
use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

/// One parameter value, printed like JS `String(value)`.
///
/// A [`Text`] takes `Arg<'static>`: literals, owned strings (`String`, `&String` is
/// copied), numbers, booleans, a `char`, and other `Text` / [`Key`] values, resolved when
/// the outer text is shown; any other [`Display`](fmt::Display) value through
/// [`Arg::display`]. [`Args`] also borrows (`Arg::Str`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Arg<'a> {
    /// Text, inserted as is.
    Str(&'a str),
    /// Owned text, shared on clone: what a `String` becomes.
    String(Arc<str>),
    /// An integer: its decimal digits.
    Int(i64),
    /// A number as JS prints it: `1.5`, `1e+21`, `NaN`, `Infinity`, `-0` as `0`. With the
    /// `format` feature: shortest float printing costs ~17 KB of `.wasm` (DESIGN 2.4).
    #[cfg(feature = "format")]
    Float(f64),
    /// `true` / `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// A translation inside a translation, resolved each time the outer one is shown.
    Text(Box<Text>),
    /// A number formatted when shown (JS `tn`), in the outer `Text`'s instance and locale
    /// (in `t_with`: the nearest scope's, else the global instance's), with the named
    /// `format` of that locale or the defaults. Feature `format`.
    ///
    /// ```
    /// use i18n_next::{Arg, I18n};
    ///
    /// let i18n = I18n::builder().messages("de", None, r#"{"sum": "Summe: {n}"}"#).locale("de").build()?;
    /// let n = Arg::Number { value: 1234.5.into(), format: None };
    /// assert_eq!(i18n.text("sum").arg("n", n), "Summe: 1.234,5");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[cfg(feature = "format")]
    Number {
        /// The number.
        value: crate::Number,
        /// A named format ([`I18n::tn_as`](crate::I18n::tn_as)), else the defaults.
        format: Option<Cow<'a, str>>,
    },
    /// A date (JS `td`) or a relative date (`tdr`) formatted when shown, like
    /// [`Arg::Number`]: in the outer `Text`'s instance and locale, with a named format of
    /// that locale or the defaults. Built by [`Arg::date`], [`Arg::date_as`],
    /// [`Arg::relative`]. Feature `format`.
    #[cfg(feature = "format")]
    Date(crate::DateArg<'a>),
}

#[cfg(feature = "format")]
impl<'a> Arg<'a> {
    /// A date formatted when shown with the default options (JS `td(date)`).
    ///
    /// ```
    /// use i18n_next::{Arg, I18n};
    ///
    /// let i18n = I18n::builder().time_zone("UTC").messages("en", None, r#"{"due": "Due {at}"}"#).build()?;
    /// assert_eq!(i18n.text("due").arg("at", Arg::date(1_704_428_640_000i64)), "Due 1/5/2024");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn date(at: impl Into<crate::Timestamp>) -> Self {
        Arg::Date(crate::DateArg::date(at.into(), None))
    }

    /// A date formatted when shown with the named date format `name` (JS `td(date, name)`).
    ///
    /// ```
    /// use i18n_next::{Arg, I18n, t};
    ///
    /// let i18n = I18n::builder()
    ///     .time_zone("UTC")
    ///     .datetime_formats(r#"{"en": {"short": {"month": "short", "day": "numeric"}}}"#)
    ///     .messages("en", None, r#"{"due": "Due {at}"}"#)
    ///     .build()?;
    /// assert_eq!(t!(i18n => "due", at = Arg::date_as(1_704_428_640_000i64, "short")), "Due Jan 5");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn date_as(at: impl Into<crate::Timestamp>, name: impl Into<Cow<'a, str>>) -> Self {
        Arg::Date(crate::DateArg::date(at.into(), Some(name.into())))
    }

    /// An instant relative to the instance's clock when shown (JS `tdr(date)`): `3 days ago`.
    ///
    /// ```
    /// use i18n_next::{Arg, I18n};
    ///
    /// let now = 1_704_428_640_000i64;
    /// let i18n = I18n::builder().clock(move || now).messages("en", None, r#"{"seen": "Seen {at}"}"#).build()?;
    /// assert_eq!(i18n.text("seen").arg("at", Arg::relative(now - 7_200_000)), "Seen 2 hours ago");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn relative(at: impl Into<crate::Timestamp>) -> Self {
        Arg::Date(crate::DateArg::relative(at.into()))
    }
}

impl PartialEq for Arg<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (a, b) if a.as_str().is_some() || b.as_str().is_some() => a.as_str() == b.as_str(),
            (Arg::Int(a), Arg::Int(b)) => a == b,
            #[cfg(feature = "format")]
            (Arg::Float(a), Arg::Float(b)) => a == b,
            (Arg::Bool(a), Arg::Bool(b)) => a == b,
            (Arg::Null, Arg::Null) => true,
            (Arg::Text(a), Arg::Text(b)) => a.same(b),
            #[cfg(feature = "format")]
            (
                Arg::Number {
                    value: a,
                    format: fa,
                },
                Arg::Number {
                    value: b,
                    format: fb,
                },
            ) => a == b && fa == fb,
            #[cfg(feature = "format")]
            (Arg::Date(a), Arg::Date(b)) => a == b,
            _ => false,
        }
    }
}

/// A stack buffer for one number, boolean or `null` ([`MAX_SCALAR_LEN`] fits).
pub(crate) struct Small {
    bytes: [u8; 32],
    len: usize,
}

impl Small {
    pub(crate) fn new() -> Self {
        Small {
            bytes: [0; 32],
            len: 0,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(self.bytes.get(..self.len).unwrap_or(&[])).unwrap_or("")
    }
}

impl Out for Small {
    fn push_str(&mut self, s: &str) {
        let end = self.len + s.len();
        if let Some(slot) = self.bytes.get_mut(self.len..end) {
            slot.copy_from_slice(s.as_bytes());
            self.len = end;
        }
    }

    fn push(&mut self, c: char) {
        self.push_str(c.encode_utf8(&mut [0; 4]));
    }
}

/// `&<>"'` as HTML entities, like JS `escapeHtml`.
pub(crate) fn escape_html(out: &mut dyn fmt::Write, s: &str) -> fmt::Result {
    let mut from = 0;
    for (i, b) in s.bytes().enumerate() {
        let entity = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            _ => continue,
        };
        out.write_str(s.get(from..i).unwrap_or(""))?;
        out.write_str(entity)?;
        from = i + 1;
    }
    out.write_str(s.get(from..).unwrap_or(""))
}

/// A writer that escapes what goes through it ([`escape_html`]).
pub(crate) struct Escape<'w>(pub(crate) &'w mut dyn fmt::Write);

impl fmt::Write for Escape<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        escape_html(self.0, s)
    }
}

impl Arg<'_> {
    fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            Arg::String(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn write(&self, out: &mut String) {
        match self {
            Arg::Str(s) => out.push_str(s),
            Arg::String(s) => out.push_str(s),
            Arg::Text(t) => {
                // Writing to a `String` cannot fail.
                let _ = t.show(out);
            }
            #[cfg(feature = "format")]
            Arg::Number { value, format } => {
                let _ = crate::format::api::write_in(None, None, *value, format.as_deref(), out);
            }
            #[cfg(feature = "format")]
            Arg::Date(d) => {
                let _ = d.write_in(None, None, out);
            }
            other => other.scalar(out),
        }
    }

    /// Numbers, booleans and `null`; text kinds write nothing.
    fn scalar<O: Out + ?Sized>(&self, out: &mut O) {
        match *self {
            Arg::Int(n) => {
                if n < 0 {
                    out.push('-');
                }
                push_u64(out, n.unsigned_abs());
            }
            #[cfg(feature = "format")]
            Arg::Float(v) => push_js_number(out, v),
            Arg::Bool(b) => out.push_str(if b { "true" } else { "false" }),
            Arg::Null => out.push_str("null"),
            Arg::Str(_) | Arg::String(_) | Arg::Text(_) => {}
            #[cfg(feature = "format")]
            Arg::Number { .. } | Arg::Date(_) => {}
        }
    }

    /// Writes the value to a formatter: text escaped under `html` (JS `escapeParams`:
    /// numbers and booleans are not), numbers through a stack buffer.
    pub(crate) fn emit(&self, out: &mut dyn fmt::Write, html: bool) -> fmt::Result {
        match self {
            Arg::Str(s) if html => escape_html(out, s),
            Arg::String(s) if html => escape_html(out, s),
            Arg::Str(s) => out.write_str(s),
            Arg::String(s) => out.write_str(s),
            // an `html` Text is markup already; a plain one is text to escape
            Arg::Text(t) if html && !t.is_html() => t.emit(&mut Escape(out)),
            Arg::Text(t) => t.emit(out),
            #[cfg(feature = "format")]
            Arg::Number { value, format } => {
                crate::format::api::write_in(None, None, *value, format.as_deref(), out)
            }
            #[cfg(feature = "format")]
            Arg::Date(d) => d.write_in(None, None, out),
            other => {
                let mut b = Small::new();
                other.scalar(&mut b);
                out.write_str(b.as_str())
            }
        }
    }

    /// Bytes [`Arg::write`] appends: exact, an upper bound for a float, a guess for a
    /// `Text` (it only sizes a buffer).
    pub(crate) fn len(&self) -> usize {
        match self {
            Arg::Str(s) => s.len(),
            Arg::String(s) => s.len(),
            Arg::Int(n) => usize::from(*n < 0) + u64_len(n.unsigned_abs()),
            #[cfg(feature = "format")]
            Arg::Float(_) => MAX_SCALAR_LEN,
            Arg::Bool(true) | Arg::Null => 4,
            Arg::Bool(false) => 5,
            Arg::Text(_) => MAX_SCALAR_LEN,
            // a guess: it only sizes a buffer
            #[cfg(feature = "format")]
            Arg::Number { .. } | Arg::Date(_) => 2 * MAX_SCALAR_LEN,
        }
    }
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Str(s)
    }
}

/// Copied (one allocation), so it fits a `Text`; [`Args::str`] borrows instead.
impl From<&String> for Arg<'_> {
    fn from(s: &String) -> Self {
        Arg::String(Arc::from(s.as_str()))
    }
}

impl From<String> for Arg<'_> {
    fn from(s: String) -> Self {
        Arg::String(Arc::from(s))
    }
}

impl From<Arc<str>> for Arg<'_> {
    fn from(s: Arc<str>) -> Self {
        Arg::String(s)
    }
}

impl From<Box<str>> for Arg<'_> {
    fn from(s: Box<str>) -> Self {
        Arg::String(Arc::from(s))
    }
}

impl<'a> From<Cow<'a, str>> for Arg<'a> {
    fn from(s: Cow<'a, str>) -> Self {
        match s {
            Cow::Borrowed(s) => Arg::Str(s),
            Cow::Owned(s) => Arg::from(s),
        }
    }
}

macro_rules! int_args {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg<'_> {
            fn from(n: $t) -> Self {
                Arg::Int(i64::from(n))
            }
        }
    )*};
}
int_args!(i8, i16, i32, i64, u8, u16, u32);

/// Past 2^53 the value is what JS would hold, the nearest `f64`, as for [`Count`]:
/// `2^53 + 1` shows `9007199254740992`, `u64::MAX` `18446744073709552000`.
impl From<u64> for Arg<'_> {
    fn from(n: u64) -> Self {
        if n <= SAFE {
            Arg::Int(n as i64)
        } else {
            big(n as f64)
        }
    }
}

/// Past ±2^53 the nearest `f64`, printed as JS prints it (`1e+21` and up with an exponent),
/// as for `u64`. Feature `format` (float printing).
#[cfg(feature = "format")]
impl From<u128> for Arg<'_> {
    fn from(n: u128) -> Self {
        match u64::try_from(n) {
            Ok(n) => Arg::from(n),
            Err(_) => Arg::Float(n as f64),
        }
    }
}

/// Like `u128`. Feature `format` (float printing).
#[cfg(feature = "format")]
impl From<i128> for Arg<'_> {
    fn from(n: i128) -> Self {
        if n.unsigned_abs() <= u128::from(SAFE) {
            Arg::Int(n as i64)
        } else {
            Arg::Float(n as f64)
        }
    }
}

/// The character as text (one allocation).
impl From<char> for Arg<'_> {
    fn from(c: char) -> Self {
        Arg::String(Arc::from(&*c.encode_utf8(&mut [0; 4])))
    }
}

/// An integer past 2^53 as JS holds it: a float with `format`, else its digits as text
/// (below 2^64 a rounded whole number never needs an exponent).
fn big(x: f64) -> Arg<'static> {
    #[cfg(feature = "format")]
    return Arg::Float(x);
    #[cfg(not(feature = "format"))]
    {
        let mut s = String::new();
        Count(CountRepr::Big(x)).write(&mut s);
        Arg::String(Arc::from(s))
    }
}

impl<'a> Arg<'a> {
    /// Any [`Display`](fmt::Display) value as text, formatted now (one allocation): a
    /// path, an error, a type of your own.
    ///
    /// ```
    /// use i18n_next::{Arg, I18n, t};
    ///
    /// let i18n = I18n::builder().messages("en", None, r#"{"io": "Cannot read {path}"}"#).build()?;
    /// let path = std::path::Path::new("/tmp/a.json");
    /// assert_eq!(t!(i18n => "io", path = Arg::display(path.display())), "Cannot read /tmp/a.json");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn display(value: impl fmt::Display) -> Self {
        Arg::String(Arc::from(value.to_string()))
    }
}

/// Past `i64::MAX` (only on a 128-bit `usize`) the value saturates.
impl From<usize> for Arg<'_> {
    fn from(n: usize) -> Self {
        Arg::Int(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

#[cfg(feature = "format")]
impl From<f64> for Arg<'_> {
    fn from(v: f64) -> Self {
        Arg::Float(v)
    }
}

#[cfg(feature = "format")]
impl From<f32> for Arg<'_> {
    fn from(v: f32) -> Self {
        Arg::Float(f64::from(v))
    }
}

impl From<bool> for Arg<'_> {
    fn from(b: bool) -> Self {
        Arg::Bool(b)
    }
}

impl From<Text> for Arg<'_> {
    fn from(t: Text) -> Self {
        Arg::Text(Box::new(t))
    }
}

impl From<Key> for Arg<'_> {
    fn from(k: Key) -> Self {
        Arg::Text(Box::new(k.args()))
    }
}

/// The longest spelling of a number, boolean or `null`, a count included: an `f64`'s,
/// `-0.0000012345678901234567` (17 digits after `0.00000`), one more than
/// `-2.2250738585072014e-308`.
pub(crate) const MAX_SCALAR_LEN: usize = 25;

/// 2^53: integers up to it are exact in an `f64`, as JS holds every number.
const SAFE: u64 = 1 << 53;

/// The count of [`I18n::tc`](crate::I18n::tc): an integer, or (feature `format`) an `f64`.
///
/// It picks the plural form and fills `{count}`, printed as JS prints the number. A
/// negative, fractional or non-finite count takes the last form, as in JS. An integer past
/// ±2^53 is what JS would hold: the nearest `f64` (`2^53 + 1` is `9007199254740992`,
/// `u64::MAX` is `18446744073709552000`), for the form and for `{count}` alike.
///
/// ```
/// use i18n_next::Count;
///
/// let _ = [Count::from(3), Count::from(3u64), Count::from(3usize), Count::from(-1i64)];
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Count(CountRepr);

#[derive(Debug, Clone, Copy, PartialEq)]
enum CountRepr {
    /// 0..=2^53.
    Whole(u64),
    /// -2^53..0.
    Negative(i64),
    /// An integer past ±2^53, rounded to the nearest `f64` (no `format` feature needed).
    Big(f64),
    #[cfg(feature = "format")]
    Float(f64),
}

impl Count {
    /// The count as `pluralForm` reads it: whole and not negative, else `None` (last form).
    pub(crate) fn whole(self) -> Option<Whole> {
        match self.0 {
            CountRepr::Whole(n) => Some(Whole::Int(n)),
            CountRepr::Negative(_) => None,
            CountRepr::Big(x) if x > 0.0 => Some(Whole::Big(x)),
            CountRepr::Big(_) => None,
            // `-0.0 >= 0.0`: JS `Number.isInteger(-0) && !(-0 < 0)`, the zero form.
            #[cfg(feature = "format")]
            CountRepr::Float(x) if x >= 0.0 && x.fract() == 0.0 => {
                // Up to 2^53 every whole f64 is exact and spelled with its own digits.
                if x <= SAFE as f64 {
                    Some(Whole::Int(x as u64))
                } else {
                    Some(Whole::Big(x))
                }
            }
            #[cfg(feature = "format")]
            CountRepr::Float(_) => None,
        }
    }

    /// The count as a JS number (a custom plural rule gets it).
    pub(crate) fn as_f64(self) -> f64 {
        match self.0 {
            CountRepr::Whole(n) => n as f64,
            CountRepr::Negative(n) => n as f64,
            CountRepr::Big(x) => x,
            #[cfg(feature = "format")]
            CountRepr::Float(x) => x,
        }
    }

    /// Appends the count as JS `String(count)` writes it.
    pub(crate) fn write<O: Out + ?Sized>(self, out: &mut O) {
        match self.0 {
            CountRepr::Whole(n) => push_u64(out, n),
            CountRepr::Negative(n) => {
                out.push('-');
                push_u64(out, n.unsigned_abs());
            }
            CountRepr::Big(x) => {
                if x < 0.0 {
                    out.push('-');
                }
                // Past 2^53 and at most 2^64: never `None`, never an exponent.
                let (digits, zeros) = shortest_whole(x.abs()).unwrap_or((0, 0));
                push_u64(out, digits);
                out.push_zeros(zeros as usize);
            }
            #[cfg(feature = "format")]
            CountRepr::Float(v) => push_js_number(out, v),
        }
    }

    /// Bytes [`Count::write`] appends: exact, an upper bound for a float.
    pub(crate) fn len(self) -> usize {
        match self.0 {
            CountRepr::Whole(n) => u64_len(n),
            CountRepr::Negative(n) => 1 + u64_len(n.unsigned_abs()),
            // `-18446744073709552000`
            CountRepr::Big(_) => 21,
            #[cfg(feature = "format")]
            CountRepr::Float(_) => MAX_SCALAR_LEN,
        }
    }
}

impl From<i64> for Count {
    fn from(n: i64) -> Self {
        match u64::try_from(n) {
            Ok(n) => Count::from(n),
            Err(_) if n.unsigned_abs() <= SAFE => Count(CountRepr::Negative(n)),
            Err(_) => Count(CountRepr::Big(n as f64)),
        }
    }
}

impl From<i32> for Count {
    fn from(n: i32) -> Self {
        Count::from(i64::from(n))
    }
}

impl From<u64> for Count {
    fn from(n: u64) -> Self {
        Count(if n <= SAFE {
            CountRepr::Whole(n)
        } else {
            CountRepr::Big(n as f64)
        })
    }
}

impl From<u32> for Count {
    fn from(n: u32) -> Self {
        Count(CountRepr::Whole(u64::from(n)))
    }
}

impl From<usize> for Count {
    fn from(n: usize) -> Self {
        Count::from(n as u64)
    }
}

#[cfg(feature = "format")]
impl From<f64> for Count {
    fn from(v: f64) -> Self {
        Count(CountRepr::Float(v))
    }
}

/// What [`Args::find`] found: a stored value, or the `{length}` of a list. `Copy`, not a
/// `Cow`: a lookup has no drop glue, so the callers' `put` stays inlined (~2 ns a
/// placeholder, `t_with` with ten arguments -10 ns).
#[derive(Clone, Copy)]
pub(crate) enum Hit<'x, 'a> {
    Arg(&'x Arg<'a>),
    Len(i64),
}

impl<'a> Hit<'_, 'a> {
    /// Runs `f` on the value (`{length}` as an [`Arg::Int`]).
    #[inline]
    pub(crate) fn with<R>(self, f: impl FnOnce(&Arg<'a>) -> R) -> R {
        match self {
            Hit::Arg(a) => f(a),
            Hit::Len(n) => f(&Arg::Int(n)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Name<'a> {
    Named(&'a str),
    /// A name built at run time (a `Text` argument named by a `String`).
    Shared(Arc<str>),
    /// A list item: `{0}`, `{1}`…
    Index(usize),
}

type Slot<'a> = (Name<'a>, Arg<'a>);

const INLINE: usize = 4;

const EMPTY: Slot<'static> = (Name::Index(0), Arg::Null);

/// Parameters for `{name}` placeholders, plus an optional default text.
///
/// ```
/// use i18n_next::{Args, I18n};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"hi": "Hi, {name}! {n} new", "pair": "{0} and {1}"}"#)
///     .build()?;
/// assert_eq!(i18n.t_with("hi", &Args::new().str("name", "Ann").int("n", 3)), "Hi, Ann! 3 new");
/// assert_eq!(i18n.t_with("pair", &Args::list(["Ann", "Bob"])), "Ann and Bob");
/// assert_eq!(i18n.t_with("nope", &Args::new().int("n", 1).or("Default {n}")), "Default 1");
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// A name that is not given leaves `{name}` as written. A name given twice: the last wins.
#[derive(Clone)]
pub struct Args<'a> {
    inline: [Slot<'a>; INLINE],
    len: usize,
    overflow: Vec<Slot<'a>>,
    /// Items of [`Args::list`]: `{length}` reads it, as on a JS array.
    list_len: Option<usize>,
    default: Option<&'a str>,
}

/// The parameters as a map (`{"name": Str("Ann"), 0: Int(1)}`) and the default.
impl fmt::Debug for Args<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut m = f.debug_map();
        for (name, value) in self.slots() {
            match name {
                Name::Named(n) => m.entry(n, value),
                Name::Shared(n) => m.entry(n, value),
                Name::Index(i) => m.entry(i, value),
            };
        }
        if let Some(d) = self.default {
            m.entry(&"or", &d);
        }
        m.finish()
    }
}

impl Default for Args<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Args<'a> {
    /// No parameters.
    #[must_use]
    pub const fn new() -> Self {
        Args {
            inline: [EMPTY; INLINE],
            len: 0,
            overflow: Vec::new(),
            list_len: None,
            default: None,
        }
    }

    /// Positional parameters `{0}`, `{1}`… and `{length}`, like a JS array.
    #[must_use]
    pub fn list<I>(items: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Arg<'a>>,
    {
        let mut args = Self::new();
        args.extend_list(items.into_iter().map(Into::into));
        args
    }

    /// Appends list items, numbered after those already there.
    pub(crate) fn extend_list(&mut self, items: impl Iterator<Item = Arg<'a>>) {
        let mut n = self.list_len.unwrap_or(0);
        for item in items {
            // too deep a `Text` (see `Text`): dropped, `{n}` shows as written
            if !matches!(&item, Arg::Text(t) if t.depth >= MAX_NESTING) {
                self.push(Name::Index(n), item);
            }
            n += 1;
        }
        self.list_len = Some(n);
    }

    pub(crate) fn push_named(&mut self, name: Cow<'a, str>, value: Arg<'a>) {
        let name = match name {
            Cow::Borrowed(n) => Name::Named(n),
            Cow::Owned(n) => Name::Shared(Arc::from(n)),
        };
        self.push(name, value);
    }

    /// Same parameters, same order (`Text` equality inside an [`Arg`]).
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.list_len == other.list_len && self.slots() == other.slots()
    }

    /// Whether there is no parameter at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0 && self.overflow.is_empty() && self.list_len.is_none()
    }

    fn push(&mut self, name: Name<'a>, value: Arg<'a>) {
        if let Some(slot) = self.inline.get_mut(self.len) {
            *slot = (name, value);
            self.len += 1;
        } else {
            // A fifth parameter moves all of them to the `Vec`, so lookups scan one slice.
            if self.overflow.is_empty() {
                self.overflow.reserve(2 * INLINE);
                self.overflow
                    .extend(self.inline.iter_mut().map(|s| std::mem::replace(s, EMPTY)));
            }
            self.overflow.push((name, value));
        }
    }

    /// Adds a parameter of any [`Arg`] kind.
    #[must_use]
    pub fn arg(mut self, name: &'a str, value: impl Into<Arg<'a>>) -> Self {
        self.push(Name::Named(name), value.into());
        self
    }

    /// Adds a text parameter.
    #[must_use]
    pub fn str(self, name: &'a str, value: &'a str) -> Self {
        self.arg(name, Arg::Str(value))
    }

    /// Adds an integer parameter.
    #[must_use]
    pub fn int(self, name: &'a str, value: i64) -> Self {
        self.arg(name, Arg::Int(value))
    }

    /// Adds a number parameter, printed as JS prints numbers (feature `format`).
    #[cfg(feature = "format")]
    #[must_use]
    pub fn float(self, name: &'a str, value: f64) -> Self {
        self.arg(name, Arg::Float(value))
    }

    /// Adds a boolean parameter.
    #[must_use]
    pub fn bool(self, name: &'a str, value: bool) -> Self {
        self.arg(name, Arg::Bool(value))
    }

    /// Adds a parameter printed as `null`.
    #[must_use]
    pub fn null(self, name: &'a str) -> Self {
        self.arg(name, Arg::Null)
    }

    /// The text returned when the key is missing everywhere, interpolated with these
    /// parameters. An empty default counts as none (the key is returned), as in JS.
    #[must_use]
    pub fn or(mut self, default: &'a str) -> Self {
        self.default = Some(default);
        self
    }

    pub(crate) fn default_text(&self) -> Option<&'a str> {
        self.default.filter(|d| !d.is_empty())
    }

    /// The default as given: `tc` keeps an empty one (JS `defaultValue ?? key`).
    pub(crate) fn default_raw(&self) -> Option<&'a str> {
        self.default
    }

    /// Every parameter, oldest first: the inline ones, or all of them in the `Vec` once
    /// there are more than four.
    fn slots(&self) -> &[Slot<'a>] {
        if self.overflow.is_empty() {
            self.inline.get(..self.len).unwrap_or(&[])
        } else {
            &self.overflow
        }
    }

    /// The value of placeholder `name`: the newest named match, a list item for a canonical
    /// index (`0`, `12`, never `01`), or the list length for `length`.
    #[inline]
    pub(crate) fn find(&self, name: &str) -> Option<Hit<'_, 'a>> {
        self.lookup(name, true)
    }

    /// What JS `{ ...args }` holds under `name`: [`Args::find`] without `length` (a spread
    /// array has none).
    #[inline]
    pub(crate) fn spread(&self, name: &str) -> Option<Hit<'_, 'a>> {
        self.lookup(name, false)
    }

    // One function for both: as two (`spread` + a `length` fallback) `t_with` with one
    // argument took +20 ns.
    #[inline]
    fn lookup(&self, name: &str, length: bool) -> Option<Hit<'_, 'a>> {
        // Only `list` makes index slots.
        let index = self.list_len.and_then(|_| canonical_index(name));
        for (slot, value) in self.slots().iter().rev() {
            let hit = match slot {
                Name::Named(n) => same_name(n, name),
                Name::Shared(n) => same_name(n, name),
                Name::Index(i) => index == Some(*i),
            };
            if hit {
                return Some(Hit::Arg(value));
            }
        }
        match self.list_len {
            Some(n) if length && name == "length" => Some(Hit::Len(n as i64)),
            _ => None,
        }
    }

    /// At least the longest [`Arg::len`] of the parameters, `{length}` of a list included:
    /// exact for text, [`MAX_SCALAR_LEN`] for anything else (no digit counting: it only
    /// sizes the cheap bound of an interpolation).
    pub(crate) fn max_value_len(&self) -> usize {
        let list = if self.list_len.is_some() {
            MAX_SCALAR_LEN
        } else {
            0
        };
        self.slots()
            .iter()
            .map(|(_, v)| match v {
                Arg::Str(s) => s.len(),
                Arg::String(s) => s.len(),
                _ => MAX_SCALAR_LEN,
            })
            .fold(list, usize::max)
    }
}

// An inlined byte loop: names are short, and a `memcmp` call per slot made many parameters
// cost ~25 ns each.
#[inline(always)]
fn same_name(n: &str, name: &str) -> bool {
    n.len() == name.len() && n.bytes().zip(name.bytes()).all(|(a, b)| a == b)
}

/// `name` as an array index the way JS reads one: digits, no leading zero but `0` itself.
fn canonical_index(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    if b.is_empty() || (b.len() > 1 && b[0] == b'0') || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    name.parse().ok()
}
