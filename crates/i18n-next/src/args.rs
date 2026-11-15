//! Parameters of `t_with` (DESIGN 2.2, spike S4): up to four live inline, so building
//! them allocates nothing; a fifth and later go to a `Vec`. Lookup is a linear scan from
//! the newest, which beats hashing for the real 1–4 parameters (spike S3).

#[cfg(feature = "format")]
use crate::json::push_js_number;
use crate::json::{push_u64, u64_len};
use crate::plural::{Whole, shortest_whole};

/// One parameter value, printed like JS `String(value)`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Arg<'a> {
    /// Text, inserted as is.
    Str(&'a str),
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
}

impl Arg<'_> {
    pub(crate) fn write(&self, out: &mut String) {
        match *self {
            Arg::Str(s) => out.push_str(s),
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
        }
    }

    /// Bytes [`Arg::write`] appends: exact, an upper bound for a float.
    pub(crate) fn len(&self) -> usize {
        match *self {
            Arg::Str(s) => s.len(),
            Arg::Int(n) => usize::from(n < 0) + u64_len(n.unsigned_abs()),
            #[cfg(feature = "format")]
            Arg::Float(_) => MAX_SCALAR_LEN,
            Arg::Bool(true) | Arg::Null => 4,
            Arg::Bool(false) => 5,
        }
    }
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Str(s)
    }
}

impl<'a> From<&'a String> for Arg<'a> {
    fn from(s: &'a String) -> Self {
        Arg::Str(s)
    }
}

impl From<i64> for Arg<'_> {
    fn from(n: i64) -> Self {
        Arg::Int(n)
    }
}

impl From<i32> for Arg<'_> {
    fn from(n: i32) -> Self {
        Arg::Int(i64::from(n))
    }
}

impl From<u32> for Arg<'_> {
    fn from(n: u32) -> Self {
        Arg::Int(i64::from(n))
    }
}

#[cfg(feature = "format")]
impl From<f64> for Arg<'_> {
    fn from(v: f64) -> Self {
        Arg::Float(v)
    }
}

impl From<bool> for Arg<'_> {
    fn from(b: bool) -> Self {
        Arg::Bool(b)
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

    /// Appends the count as JS `String(count)` writes it.
    pub(crate) fn write(self, out: &mut String) {
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
                out.extend(std::iter::repeat_n('0', zeros as usize));
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Name<'a> {
    Named(&'a str),
    /// A list item: `{0}`, `{1}`…
    Index(usize),
}

type Slot<'a> = (Name<'a>, Arg<'a>);

const INLINE: usize = 4;

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
#[derive(Debug, Clone)]
pub struct Args<'a> {
    inline: [Slot<'a>; INLINE],
    len: usize,
    overflow: Vec<Slot<'a>>,
    /// Items of [`Args::list`]: `{length}` reads it, as on a JS array.
    list_len: Option<usize>,
    default: Option<&'a str>,
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
            inline: [(Name::Index(0), Arg::Null); INLINE],
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
        let mut n = 0;
        for item in items {
            args.push(Name::Index(n), item.into());
            n += 1;
        }
        args.list_len = Some(n);
        args
    }

    fn push(&mut self, name: Name<'a>, value: Arg<'a>) {
        if let Some(slot) = self.inline.get_mut(self.len) {
            *slot = (name, value);
            self.len += 1;
        } else {
            // A fifth parameter moves all of them to the `Vec`, so lookups scan one slice.
            if self.overflow.is_empty() {
                self.overflow.reserve(2 * INLINE);
                self.overflow.extend_from_slice(&self.inline);
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
    pub(crate) fn find(&self, name: &str) -> Option<Arg<'a>> {
        self.lookup(name, true)
    }

    /// What JS `{ ...args }` holds under `name`: [`Args::find`] without `length` (a spread
    /// array has none).
    pub(crate) fn spread(&self, name: &str) -> Option<Arg<'a>> {
        self.lookup(name, false)
    }

    // One function for both: as two (`spread` + a `length` fallback) `t_with` with one
    // argument took +20 ns.
    #[inline]
    fn lookup(&self, name: &str, length: bool) -> Option<Arg<'a>> {
        // Only `list` makes index slots.
        let index = self.list_len.and_then(|_| canonical_index(name));
        for (slot, value) in self.slots().iter().rev() {
            let hit = match *slot {
                // An inlined byte loop: names are short, and a `memcmp` call per slot
                // made many parameters cost ~25 ns each.
                Name::Named(n) => {
                    n.len() == name.len() && n.bytes().zip(name.bytes()).all(|(a, b)| a == b)
                }
                Name::Index(i) => index == Some(i),
            };
            if hit {
                return Some(*value);
            }
        }
        match self.list_len {
            Some(n) if length && name == "length" => Some(Arg::Int(n as i64)),
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
                _ => MAX_SCALAR_LEN,
            })
            .fold(list, usize::max)
    }
}

/// `name` as an array index the way JS reads one: digits, no leading zero but `0` itself.
fn canonical_index(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    if b.is_empty() || (b.len() > 1 && b[0] == b'0') || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    name.parse().ok()
}
