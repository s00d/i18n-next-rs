//! `Intl.RelativeTimeFormat` as `FormatService.formatRelativeTime` drives it (DESIGN 5.3):
//! the largest unit that fits at least once, rounded down, `-0` (under a second in the past)
//! as `0 … ago`. The patterns per plural form and the words of `numeric: "auto"` were read
//! back from Node's ICU by `scripts/cldr.mjs`; the number is the locale's default format.

use super::NumberOptions;
use super::data::{self, DATE_REL, DATE_REL_PARTS, Loc};
use crate::plural::{self, Operands};
use std::fmt;

/// Options of [`I18n::tdr_with`](crate::I18n::tdr_with), `Intl.RelativeTimeFormat`'s:
/// `numeric` (`always` by default, [`auto`](Self::auto) says `yesterday` / `now`) and
/// `style` (`long` by default, [`short`](Self::short), [`narrow`](Self::narrow)).
///
/// ```
/// use i18n_next::RelativeOptions;
///
/// let o = RelativeOptions::default().auto().short();
/// assert_ne!(o, RelativeOptions::default());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct RelativeOptions {
    auto: bool,
    /// 0 long, 1 short, 2 narrow
    style: u8,
}

impl RelativeOptions {
    /// `numeric: "auto"`: `yesterday`, `now`, `next year` where the locale has a word.
    #[must_use]
    pub const fn auto(mut self) -> Self {
        self.auto = true;
        self
    }

    /// `style: "short"`.
    #[must_use]
    pub const fn short(mut self) -> Self {
        self.style = 1;
        self
    }

    /// `style: "narrow"`.
    #[must_use]
    pub const fn narrow(mut self) -> Self {
        self.style = 2;
        self
    }
}

/// Seconds per unit, largest first (`FormatService` `UNITS`).
const SPANS: [f64; 6] = [31_536_000.0, 2_592_000.0, 86_400.0, 3600.0, 60.0, 1.0];

/// Writes `at` relative to `now` (both milliseconds of valid dates).
#[allow(
    clippy::cast_precision_loss,
    reason = "JS subtracts the two as doubles"
)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole count"
)]
pub(crate) fn write(
    at: i64,
    now: i64,
    o: RelativeOptions,
    loc: &Loc,
    out: &mut dyn fmt::Write,
) -> fmt::Result {
    let seconds = (at as f64 - now as f64) / 1000.0;
    let size = seconds.abs();
    let unit = SPANS
        .iter()
        .position(|&s| size >= s)
        .unwrap_or(SPANS.len() - 1);
    let n = (size / SPANS.get(unit).copied().unwrap_or(1.0)).floor();
    let past = seconds < 0.0;
    // the locale's row names one part per unit and style: 5 words, then future and past
    // patterns by plural category
    let part = DATE_REL
        .get(usize::from(loc.drel))
        .and_then(|r| r.get(unit * 3 + usize::from(o.style)))
        .and_then(|&p| DATE_REL_PARTS.get(usize::from(p)));
    let s = |i: usize| data::dname(part.and_then(|r| r.get(i)).copied().unwrap_or(0));
    if o.auto && n <= 2.0 {
        // ICU: -2 … 2 by their own words (0 whatever its sign), where the locale has one
        let word = s(if past { 2 - n as usize } else { 2 + n as usize });
        if !word.is_empty() {
            return out.write_str(word);
        }
    }
    let cat = plural::select_rule(loc.plural, &Operands::int(n as u64)) as usize;
    let list = if past { 11 } else { 5 };
    let pattern = match s(list + cat) {
        "" => s(list + 5),
        p => p,
    };
    // a form may spell its number out (`ar`: `قبل سنتين`, two years ago)
    let Some((before, after)) = pattern.split_once("{0}") else {
        return out.write_str(pattern);
    };
    out.write_str(before)?;
    super::number::write(n, &NumberOptions::DEFAULT, loc, out)?;
    out.write_str(after)
}
