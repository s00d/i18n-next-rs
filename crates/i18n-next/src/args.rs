//! Parameters of `t_with` (DESIGN 2.2, spike S4): up to four live inline, so building
//! them allocates nothing; a fifth and later go to a `Vec`. Lookup is a linear scan from
//! the newest, which beats hashing for the real 1–4 parameters (spike S3).

#[cfg(feature = "format")]
use crate::json::push_js_number;
use crate::json::push_u64;

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
        if self.len < INLINE {
            self.inline[self.len] = (name, value);
            self.len += 1;
        } else {
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

    /// Every parameter, oldest first.
    fn slots(&self) -> impl DoubleEndedIterator<Item = &Slot<'a>> {
        self.inline[..self.len].iter().chain(&self.overflow)
    }

    /// The value of placeholder `name`: the newest named match, a list item for a canonical
    /// index (`0`, `12`, never `01`), or the list length for `length`.
    pub(crate) fn find(&self, name: &str) -> Option<Arg<'a>> {
        let index = canonical_index(name);
        for (slot, value) in self.slots().rev() {
            let hit = match *slot {
                Name::Named(n) => n == name,
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
        self.slots().map(|(_, v)| v.max_len()).max().unwrap_or(0)
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
