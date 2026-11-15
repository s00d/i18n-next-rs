//! Formats (DESIGN 5): CLDR number data selected by `build.rs` ([`data`]), the tag →
//! data locale resolution of `Intl` ([`locale`]), `Intl.NumberFormat` options ([`options`])
//! and the formatter ([`number`]). Feature `format`.

pub(crate) mod api;
pub(crate) mod data;
pub(crate) mod locale;
mod number;
pub(crate) mod options;

pub(crate) use data::Loc;
pub(crate) use options::{Formats, NumberOptions, parse_formats};
use std::fmt;

/// A number for [`I18n::tn`](crate::I18n::tn): any primitive integer or float, held as JS
/// holds it, an `f64` (an integer past 2^53 becomes the nearest one, as for
/// [`Count`](crate::Count)).
///
/// ```
/// use i18n_next::Number;
///
/// let _ = [Number::from(3), Number::from(3u64), Number::from(-1i64), Number::from(2.5)];
/// assert_eq!(Number::from(u64::MAX), Number::from(18446744073709551616.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Number(pub(crate) f64);

macro_rules! number_from {
    ($($t:ty),*) => {$(
        impl From<$t> for Number {
            #[allow(clippy::cast_precision_loss, reason = "JS holds every number as an f64")]
            fn from(n: $t) -> Self {
                Number(n as f64)
            }
        }
    )*};
}
number_from!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Formats into `out` (a `Text` display: no allocation).
pub(crate) fn write(
    n: Number,
    o: &NumberOptions,
    loc: &Loc,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    number::write(n.0, o, loc, out)
}

/// Formats into a new `String` of the exact size: one allocation (a stack buffer first;
/// past 256 bytes, `1e300` in Devanagari digits, the `String` grows).
pub(crate) fn to_string(n: Number, o: &NumberOptions, loc: &Loc) -> String {
    let mut buf = Stack {
        bytes: [0; 256],
        len: 0,
        spill: None,
    };
    // Writing to the buffer cannot fail.
    let _ = number::write(n.0, o, loc, &mut buf);
    match buf.spill {
        Some(s) => s,
        None => {
            String::from(std::str::from_utf8(buf.bytes.get(..buf.len).unwrap_or(&[])).unwrap_or(""))
        }
    }
}

struct Stack {
    bytes: [u8; 256],
    len: usize,
    spill: Option<String>,
}

impl fmt::Write for Stack {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(spill) = &mut self.spill {
            spill.push_str(s);
            return Ok(());
        }
        let end = self.len + s.len();
        match self.bytes.get_mut(self.len..end) {
            Some(slot) => {
                slot.copy_from_slice(s.as_bytes());
                self.len = end;
            }
            None => {
                let mut spill = String::with_capacity(end * 2);
                spill.push_str(
                    std::str::from_utf8(self.bytes.get(..self.len).unwrap_or(&[])).unwrap_or(""),
                );
                spill.push_str(s);
                self.spill = Some(spill);
            }
        }
        Ok(())
    }
}
