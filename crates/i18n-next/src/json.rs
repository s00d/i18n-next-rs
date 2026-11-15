//! Own JSON reader and string writer (DESIGN 2.3): strict RFC 8259, pull events, strings
//! borrowed from the input when they hold no escapes, no allocation besides decoded
//! strings (the depth stack is a `u128` bit set).
//!
//! Decisions beyond RFC 8259 (JSONTestSuite `i_` cases, asserted in
//! `tests/json_differential.rs`):
//! - a leading UTF-8 BOM is skipped;
//! - invalid UTF-8 is rejected, and so are lone surrogates in `\u` escapes: `JSON.parse`
//!   accepts them, a Rust `str` cannot hold them, and replacing them with U+FFFD would
//!   silently change a translation;
//! - more than [`MAX_DEPTH`] nested containers is an error;
//! - numbers are checked against the grammar and kept as text, their magnitude is not
//!   checked: `1e400` is accepted and [`number_to_js_string`] gives `Infinity`, as JS does.
//!
//! This file is self-contained (no `crate::` paths): the tests and the bench include it
//! with `#[path]`, so the crate's public API stays as it is.

use std::borrow::Cow;
use std::fmt;

/// Most nested containers a document may hold; one more is an error.
pub(crate) const MAX_DEPTH: usize = 128;

/// One step of a document, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event<'a> {
    ObjectStart,
    ObjectEnd,
    ArrayStart,
    ArrayEnd,
    Key(Cow<'a, str>),
    Str(Cow<'a, str>),
    /// Raw JSON number text, checked against the grammar.
    Num(&'a str),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonErrorKind {
    UnexpectedEof,
    UnexpectedByte,
    ExpectedKey,
    ExpectedColon,
    ExpectedCommaOrEnd,
    InvalidNumber,
    InvalidEscape,
    LoneSurrogate,
    ControlCharacter,
    InvalidUtf8,
    TooDeep,
    TrailingCharacters,
}

impl JsonErrorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::UnexpectedEof => "unexpected end of input",
            Self::UnexpectedByte => "unexpected byte",
            Self::ExpectedKey => "expected a string key",
            Self::ExpectedColon => "expected ':'",
            Self::ExpectedCommaOrEnd => "expected ',' or the end of the container",
            Self::InvalidNumber => "invalid number",
            Self::InvalidEscape => "invalid escape",
            Self::LoneSurrogate => "lone surrogate in \\u escape",
            Self::ControlCharacter => "control character in string",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::TooDeep => "nesting deeper than 128",
            Self::TrailingCharacters => "trailing characters",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JsonError {
    /// Byte offset in the input (the BOM counts).
    pub(crate) offset: usize,
    pub(crate) kind: JsonErrorKind,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid JSON at byte {}: {}",
            self.offset,
            self.kind.as_str()
        )
    }
}

impl std::error::Error for JsonError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// A value must come (document start, after `:` or after `,` in an array).
    Value,
    /// Right after `[`: a value or `]`.
    ArrayFirst,
    /// Right after `{`: a key or `}`.
    ObjectFirst,
    /// After a value: `,`, the end of the container, or the end of input at depth 0.
    AfterValue,
    Done,
}

/// Pull reader: each [`Reader::next`] returns the next [`Event`], `None` after the
/// single top-level value and the trailing whitespace.
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
    depth: usize,
    /// Bit `i` set: the container at depth `i` is an object.
    objects: u128,
    state: State,
}

impl<'a> Reader<'a> {
    /// A reader over `input`; a leading UTF-8 BOM is skipped.
    pub(crate) fn new(input: &'a [u8]) -> Self {
        let pos = if input.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        Reader {
            input,
            pos,
            depth: 0,
            objects: 0,
            state: State::Value,
        }
    }

    /// Byte offset of the next unread byte.
    pub(crate) fn offset(&self) -> usize {
        self.pos
    }

    // Not `Iterator`: the item is a `Result` and the stream ends with `Ok(None)`.
    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Result<Option<Event<'a>>, JsonError> {
        self.skip_ws();
        match self.state {
            State::Value => self.value(),
            State::ArrayFirst if self.peek() == Some(b']') => self.close(),
            State::ArrayFirst => self.value(),
            State::ObjectFirst if self.peek() == Some(b'}') => self.close(),
            State::ObjectFirst => self.key(),
            State::AfterValue => self.after_value(),
            State::Done => Ok(None),
        }
    }

    fn err<T>(&self, kind: JsonErrorKind) -> Result<T, JsonError> {
        Err(JsonError {
            offset: self.pos,
            kind,
        })
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn after_value(&mut self) -> Result<Option<Event<'a>>, JsonError> {
        if self.depth == 0 {
            if self.pos < self.input.len() {
                return self.err(JsonErrorKind::TrailingCharacters);
            }
            self.state = State::Done;
            return Ok(None);
        }
        let in_object = (self.objects >> (self.depth - 1)) & 1 == 1;
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                self.skip_ws();
                if in_object { self.key() } else { self.value() }
            }
            Some(b'}') if in_object => self.close(),
            Some(b']') if !in_object => self.close(),
            None => self.err(JsonErrorKind::UnexpectedEof),
            Some(_) => self.err(JsonErrorKind::ExpectedCommaOrEnd),
        }
    }

    fn open(&mut self, object: bool) -> Result<Option<Event<'a>>, JsonError> {
        if self.depth == MAX_DEPTH {
            return self.err(JsonErrorKind::TooDeep);
        }
        let bit = 1u128 << self.depth;
        if object {
            self.objects |= bit;
        } else {
            self.objects &= !bit;
        }
        self.depth += 1;
        self.pos += 1;
        if object {
            self.state = State::ObjectFirst;
            Ok(Some(Event::ObjectStart))
        } else {
            self.state = State::ArrayFirst;
            Ok(Some(Event::ArrayStart))
        }
    }

    fn close(&mut self) -> Result<Option<Event<'a>>, JsonError> {
        self.pos += 1;
        self.depth -= 1;
        self.state = State::AfterValue;
        if (self.objects >> self.depth) & 1 == 1 {
            Ok(Some(Event::ObjectEnd))
        } else {
            Ok(Some(Event::ArrayEnd))
        }
    }

    fn key(&mut self) -> Result<Option<Event<'a>>, JsonError> {
        match self.peek() {
            Some(b'"') => {}
            None => return self.err(JsonErrorKind::UnexpectedEof),
            Some(_) => return self.err(JsonErrorKind::ExpectedKey),
        }
        let key = self.string()?;
        self.skip_ws();
        match self.peek() {
            Some(b':') => self.pos += 1,
            None => return self.err(JsonErrorKind::UnexpectedEof),
            Some(_) => return self.err(JsonErrorKind::ExpectedColon),
        }
        self.state = State::Value;
        Ok(Some(Event::Key(key)))
    }

    fn value(&mut self) -> Result<Option<Event<'a>>, JsonError> {
        let event = match self.peek() {
            None => return self.err(JsonErrorKind::UnexpectedEof),
            Some(b'{') => return self.open(true),
            Some(b'[') => return self.open(false),
            Some(b'"') => Event::Str(self.string()?),
            Some(b't') => self.literal(b"true", Event::Bool(true))?,
            Some(b'f') => self.literal(b"false", Event::Bool(false))?,
            Some(b'n') => self.literal(b"null", Event::Null)?,
            Some(b'-' | b'0'..=b'9') => self.number()?,
            Some(_) => return self.err(JsonErrorKind::UnexpectedByte),
        };
        self.state = State::AfterValue;
        Ok(Some(event))
    }

    fn literal(&mut self, text: &[u8], event: Event<'a>) -> Result<Event<'a>, JsonError> {
        if !self.input[self.pos..].starts_with(text) {
            return self.err(JsonErrorKind::UnexpectedByte);
        }
        self.pos += text.len();
        Ok(event)
    }

    fn digits(&mut self) -> bool {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos > start
    }

    /// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`
    fn number(&mut self) -> Result<Event<'a>, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.err(JsonErrorKind::InvalidNumber),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !self.digits() {
                return self.err(JsonErrorKind::InvalidNumber);
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if !self.digits() {
                return self.err(JsonErrorKind::InvalidNumber);
            }
        }
        // ASCII by construction.
        Ok(Event::Num(self.utf8(start, self.pos)?))
    }

    fn utf8(&self, start: usize, end: usize) -> Result<&'a str, JsonError> {
        let input: &'a [u8] = self.input;
        std::str::from_utf8(&input[start..end]).map_err(|e| JsonError {
            offset: start + e.valid_up_to(),
            kind: JsonErrorKind::InvalidUtf8,
        })
    }

    /// Advances to the next `"` or `\` of a string and returns it.
    fn scan(&mut self) -> Result<u8, JsonError> {
        let rest = &self.input[self.pos..];
        match rest
            .iter()
            .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
        {
            None => {
                self.pos = self.input.len();
                self.err(JsonErrorKind::UnexpectedEof)
            }
            Some(i) => {
                self.pos += i;
                match rest[i] {
                    b @ (b'"' | b'\\') => Ok(b),
                    _ => self.err(JsonErrorKind::ControlCharacter),
                }
            }
        }
    }

    /// A string starting at the opening quote: borrowed when it holds no escapes.
    fn string(&mut self) -> Result<Cow<'a, str>, JsonError> {
        self.pos += 1;
        let start = self.pos;
        if self.scan()? == b'"' {
            let s = self.utf8(start, self.pos)?;
            self.pos += 1;
            return Ok(Cow::Borrowed(s));
        }
        let mut out = String::with_capacity(self.pos - start + 16);
        out.push_str(self.utf8(start, self.pos)?);
        loop {
            self.escape(&mut out)?;
            let run = self.pos;
            let stop = self.scan()?;
            out.push_str(self.utf8(run, self.pos)?);
            if stop == b'"' {
                self.pos += 1;
                return Ok(Cow::Owned(out));
            }
        }
    }

    /// One escape starting at its `\`.
    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let at = self.pos;
        self.pos += 1;
        let c = match self.peek() {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                self.pos += 1;
                let hi = self.hex4()?;
                let code = match hi {
                    0xD800..=0xDBFF => {
                        if !self.input[self.pos..].starts_with(b"\\u") {
                            return Err(JsonError {
                                offset: at,
                                kind: JsonErrorKind::LoneSurrogate,
                            });
                        }
                        self.pos += 2;
                        let lo = self.hex4()?;
                        if !(0xDC00..=0xDFFF).contains(&lo) {
                            return Err(JsonError {
                                offset: at,
                                kind: JsonErrorKind::LoneSurrogate,
                            });
                        }
                        0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                    }
                    0xDC00..=0xDFFF => {
                        return Err(JsonError {
                            offset: at,
                            kind: JsonErrorKind::LoneSurrogate,
                        });
                    }
                    _ => hi,
                };
                // Every value left here is a scalar value.
                out.push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
                return Ok(());
            }
            None => return self.err(JsonErrorKind::UnexpectedEof),
            Some(_) => return self.err(JsonErrorKind::InvalidEscape),
        };
        self.pos += 1;
        out.push(c);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = match self.peek() {
                Some(b @ b'0'..=b'9') => b - b'0',
                Some(b @ b'a'..=b'f') => b - b'a' + 10,
                Some(b @ b'A'..=b'F') => b - b'A' + 10,
                None => return self.err(JsonErrorKind::UnexpectedEof),
                Some(_) => return self.err(JsonErrorKind::InvalidEscape),
            };
            v = v * 16 + u32::from(d);
            self.pos += 1;
        }
        Ok(v)
    }
}

/// Appends `s` quoted and escaped exactly like JS `JSON.stringify`: `"` and `\`, the
/// short escapes `\b \f \n \r \t`, other control characters below U+0020 as `\u00xx`
/// (lowercase hex); everything else, U+007F and U+2028 / U+2029 included, as is.
#[allow(
    dead_code,
    reason = "the payload writer of M7; tested and benched already"
)]
pub(crate) fn write_str(out: &mut String, s: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.reserve(s.len() + 2);
    out.push('"');
    let mut run = 0;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        let short = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x08 => "\\b",
            0x0c => "\\f",
            0..=0x1f => "",
            _ => continue,
        };
        // `i` and `run` sit at ASCII bytes, so the slice is on char boundaries.
        out.push_str(s.get(run..i).unwrap_or(""));
        if short.is_empty() {
            out.push_str("\\u00");
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0xf)]));
        } else {
            out.push_str(short);
        }
        run = i + 1;
    }
    out.push_str(s.get(run..).unwrap_or(""));
    out.push('"');
}

/// ECMAScript `Number::toString` of the `f64` that raw JSON number text denotes.
///
/// A number with at most 15 significant digits in the normal range is laid out from its
/// own digits: such a decimal survives the round trip through `f64` unchanged, so its
/// shortest digits are its digits (no `f64` parsing or formatting, which would cost the
/// micro build about 30 KB of `.wasm`, DESIGN 2.4). Anything else goes through `f64`
/// with the `format` feature; without it, the digits are kept as written (normalized),
/// which differs from JS only past 15 significant digits or outside 1e-306..1e308.
pub(crate) fn number_to_js_string(raw: &str) -> String {
    let mut out = String::new();
    let negative = raw.starts_with('-');
    let body = raw.trim_start_matches('-');
    let (mantissa, exp) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.trim_start_matches('+')),
        None => (body, "0"),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = || int.bytes().chain(frac.bytes());
    let lead = digits().take_while(|&b| b == b'0').count();
    let total = int.len() + frac.len();
    let trail = digits().rev().take_while(|&b| b == b'0').count();
    if lead == total {
        out.push('0');
        return out;
    }
    let sig: Vec<u8> = digits().skip(lead).take(total - lead - trail).collect();
    // value = 0.sig × 10^n; a huge exponent is clamped, it only picks the slow path
    let exp: i64 = exp.parse().unwrap_or(if exp.starts_with('-') {
        -100_000
    } else {
        100_000
    });
    let n = (int.len() as i64 - lead as i64).saturating_add(exp.clamp(-100_000, 100_000));
    let fast = sig.len() <= 15 && n > -306 && n < 309;
    #[cfg(feature = "format")]
    if !fast {
        push_js_number(&mut out, raw.parse().unwrap_or(f64::NAN));
        return out;
    }
    let _ = fast;
    // ASCII digits.
    let sig = std::str::from_utf8(&sig).unwrap_or("0");
    layout(&mut out, negative, sig, n as i32);
    out
}

/// ECMA-262 `Number::toString` layout of `0.digits × 10^n` (`digits` without leading or
/// trailing zeros): plain digits up to 21 integer digits, `0.000ddd` down to 1e-7,
/// exponent form (`1e+21`, `1.5e-7`) outside.
fn layout(out: &mut String, negative: bool, digits: &str, n: i32) {
    // `_checked`: ASCII digits never fail, and no slice panic (with its `fmt` code) is linked.
    let (d0, rest) = digits.split_at_checked(1).unwrap_or((digits, ""));
    let k = digits.len() as i32;
    if negative {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        let (a, b) = digits.split_at_checked(n as usize).unwrap_or((digits, ""));
        out.push_str(a);
        out.push('.');
        out.push_str(b);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(digits);
    } else {
        out.push_str(d0);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push('e');
        let e = n - 1;
        out.push(if e < 0 { '-' } else { '+' });
        push_u64(out, u64::from(e.unsigned_abs()));
    }
}

/// Stack buffer for `{:e}` of an `f64`: the longest is `2.2250738585072014e-308`
/// (23 bytes), so formatting never touches the heap.
#[cfg(feature = "format")]
struct ExpBuf {
    bytes: [u8; 32],
    len: usize,
}

#[cfg(feature = "format")]
impl fmt::Write for ExpBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        let slot = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        slot.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Appends ECMAScript `Number::toString(v)` (JS `String(v)`) to `out`, with no heap
/// temporary: shortest round-trip digits from Rust's `{:e}` (the same digits JS uses) in
/// the [`layout`] of ECMA-262; `NaN`, `Infinity`, `-Infinity`, and `-0` as `0`.
#[cfg(feature = "format")]
pub(crate) fn push_js_number(out: &mut String, v: f64) {
    if v.is_nan() {
        out.push_str("NaN");
        return;
    }
    if v == 0.0 {
        out.push('0');
        return;
    }
    if v.is_infinite() {
        out.push_str(if v < 0.0 { "-Infinity" } else { "Infinity" });
        return;
    }
    let mut buf = ExpBuf {
        bytes: [0; 32],
        len: 0,
    };
    // Cannot fail: 32 bytes hold every finite `f64` in `{:e}`.
    let _ = fmt::Write::write_fmt(&mut buf, format_args!("{:e}", v.abs()));
    let sci = std::str::from_utf8(&buf.bytes[..buf.len]).unwrap_or("0e0");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits = [0u8; 20];
    let mut k = 0;
    for b in mantissa.bytes().filter(u8::is_ascii_digit) {
        digits[k] = b;
        k += 1;
    }
    layout(
        out,
        v < 0.0,
        std::str::from_utf8(&digits[..k]).unwrap_or("0"),
        exp + 1,
    );
}

/// Appends the decimal digits of `n` (no `core::fmt`).
pub(crate) fn push_u64(out: &mut String, mut n: u64) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &digits[i..] {
        out.push(char::from(d));
    }
}
