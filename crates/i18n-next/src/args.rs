//! Parameters of `t_with` (DESIGN 2.2, spike S4): up to four live inline, so building
//! them allocates nothing; a fifth and later go to a `Vec`. Lookup is a linear scan from
//! the newest, which beats hashing for the real 1–4 parameters (spike S3).

#[cfg(feature = "format")]
use crate::json::push_js_number;
use crate::json::push_u64;
use crate::plural::Whole;

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

    /// Bytes [`Arg::write`] appends at most: exact for text, an upper bound otherwise.
    pub(crate) fn max_len(&self) -> usize {
        match self {
            Arg::Str(s) => s.len(),
            // `-9223372036854775808`; `-2.2250738585072014e-308`
            Arg::Int(_) => 24,
            #[cfg(feature = "format")]
            Arg::Float(_) => 24,
            Arg::Bool(_) | Arg::Null => 5,
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

/// The count of [`I18n::tc`](crate::I18n::tc): an integer, or (feature `format`) an `f64`.
///
/// It picks the plural form and fills `{count}`, printed as JS prints the number. A
/// negative, fractional or non-finite count takes the last form, as in JS. Integers are
/// exact: a `u64` past 2^53 is read as written, where JS would round it first.
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
    Whole(u64),
    Negative(i64),
    #[cfg(feature = "format")]
    Float(f64),
}

impl Count {
    /// The count as `pluralForm` reads it: whole and not negative, else `None` (last form).
    pub(crate) fn whole(self) -> Option<Whole> {
        match self.0 {
            CountRepr::Whole(n) => Some(Whole::Int(n)),
            CountRepr::Negative(_) => None,
            // `-0.0 >= 0.0`: JS `Number.isInteger(-0) && !(-0 < 0)`, the zero form.
            #[cfg(feature = "format")]
            CountRepr::Float(x) if x >= 0.0 && x.fract() == 0.0 => {
                // Below 2^53 every whole f64 is exact and spelled with its own digits.
                if x < 9_007_199_254_740_992.0 {
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
            #[cfg(feature = "format")]
            CountRepr::Float(v) => push_js_number(out, v),
        }
    }
}

impl From<i64> for Count {
    fn from(n: i64) -> Self {
        Count(match u64::try_from(n) {
            Ok(n) => CountRepr::Whole(n),
            Err(_) => CountRepr::Negative(n),
        })
    }
}

impl From<i32> for Count {
    fn from(n: i32) -> Self {
        Count::from(i64::from(n))
    }
}

impl From<u64> for Count {
    fn from(n: u64) -> Self {
        Count(CountRepr::Whole(n))
    }
}

impl From<u32> for Count {
    fn from(n: u32) -> Self {
        Count(CountRepr::Whole(u64::from(n)))
    }
}

impl From<usize> for Count {
    fn from(n: usize) -> Self {
        Count(CountRepr::Whole(n as u64))
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
            Some(n) if name == "length" => Some(Arg::Int(n as i64)),
            _ => None,
        }
    }

    /// Bytes the parameters add at most when every one is written once.
    pub(crate) fn max_value_len(&self) -> usize {
        self.slots()
            .iter()
            .map(|(_, v)| v.max_len())
            .max()
            .unwrap_or(0)
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
