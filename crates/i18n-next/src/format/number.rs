//! `Intl.NumberFormat` of ICU for the options of DESIGN 5.3, written straight into a
//! `fmt::Write` (DESIGN 2.2, 5.5): the shortest decimal digits of the `f64` (as JS prints it),
//! rounded half-expand as a decimal (spike S7: 1.005 → 1.01), laid out by the parsed CLDR
//! pattern of the locale: affixes, grouping (`minimumGroupingDigits`, Indian 3;2), the
//! default numbering system's digits, currency symbol and spacing, compact short patterns
//! by plural form. No allocation, no cache: the locale is an index, the fields `&'static`.

use super::data::{self, COMPACT, CURRENCIES, LOCALES, Loc, PATTERNS, Pat};
use super::options::{Display, Grouping, NumberOptions, Style};
use crate::json::shortest_digits;
use crate::plural::{self, Category, Operands};
use std::fmt;

/// Digits of a decimal `0.d1d2… × 10^n`, ASCII, no trailing zero; empty for zero.
#[derive(Clone, Copy)]
struct Decimal {
    digits: [u8; 24],
    len: usize,
    n: i32,
}

impl Decimal {
    const ZERO: Decimal = Decimal {
        digits: [0; 24],
        len: 0,
        n: 0,
    };

    /// `v` finite, ≥ 0.
    fn of(v: f64) -> Decimal {
        if v == 0.0 {
            return Decimal::ZERO;
        }
        let mut d = Decimal::ZERO;
        let mut buf = [0u8; 20];
        let (len, n) = shortest_digits(v, &mut buf);
        if let (Some(dst), Some(src)) = (d.digits.get_mut(..len), buf.get(..len)) {
            dst.copy_from_slice(src);
        }
        d.len = len;
        d.n = n;
        d
    }

    fn is_zero(&self) -> bool {
        self.len == 0
    }

    /// Keeps the digits down to magnitude `-frac` (half-expand on the next digit).
    fn round_frac(&mut self, frac: i32) {
        self.round_at(self.n.saturating_add(frac));
    }

    /// Keeps the first `keep` digits, rounding half away from zero on the next one.
    fn round_at(&mut self, keep: i32) {
        if keep >= self.len as i32 {
            return;
        }
        if keep < 0 {
            *self = Decimal::ZERO;
            return;
        }
        let keep = keep as usize;
        let up = self.digits.get(keep).is_some_and(|&d| d >= b'5');
        self.len = keep;
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    // 9.99 → 10: one digit `1`, one magnitude up
                    if let Some(first) = self.digits.first_mut() {
                        *first = b'1';
                    }
                    self.len = 1;
                    self.n += 1;
                    break;
                }
                i -= 1;
                match self.digits.get_mut(i) {
                    Some(d) if *d == b'9' => self.len = i,
                    Some(d) => {
                        *d += 1;
                        self.len = i + 1;
                        break;
                    }
                    None => break,
                }
            }
        }
        while self.len > 0 && self.digits.get(self.len - 1) == Some(&b'0') {
            self.len -= 1;
        }
        if self.len == 0 {
            *self = Decimal::ZERO;
        }
    }

    /// The digit at magnitude `p` (0: units).
    fn digit(&self, p: i32) -> u8 {
        let i = self.n - 1 - p;
        if i < 0 || i as usize >= self.len {
            return b'0';
        }
        self.digits.get(i as usize).copied().unwrap_or(b'0')
    }

    /// Integer digits shown (at least `min_int`) and fraction digits shown.
    fn shape(&self, min_int: u8, min_frac: u8) -> (i32, i32) {
        let int = self.n.max(i32::from(min_int)).max(1);
        let frac = (self.len as i32 - self.n).max(i32::from(min_frac)).max(0);
        (int, frac)
    }

    /// The value, correctly rounded (`digits` `e` exponent through the `f64` parser; no heap).
    fn value(&self) -> f64 {
        let mut buf = [0u8; 40];
        let len = self.len.min(self.digits.len());
        if let (Some(dst), Some(src)) = (buf.get_mut(..len), self.digits.get(..len)) {
            dst.copy_from_slice(src);
        }
        let mut at = len;
        let mut exp = self.n - len as i32;
        if let Some(b) = buf.get_mut(at) {
            *b = b'e';
        }
        at += 1;
        if exp < 0 {
            if let Some(b) = buf.get_mut(at) {
                *b = b'-';
            }
            at += 1;
            exp = -exp;
        }
        let start = at;
        loop {
            if let Some(b) = buf.get_mut(at) {
                *b = b'0' + (exp % 10) as u8;
            }
            at += 1;
            exp /= 10;
            if exp == 0 {
                break;
            }
        }
        if let Some(e) = buf.get_mut(start..at) {
            e.reverse();
        }
        std::str::from_utf8(buf.get(..at).unwrap_or(&[]))
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.0)
    }

    /// The plural operands of the number as shown with `frac` fraction digits.
    fn operands(&self, frac: i32) -> Operands {
        const I_MOD: u64 = 1_000_000_000_000_000_000;
        let mut i = 0u64;
        for p in (0..self.n.clamp(0, 18)).rev() {
            i = i * 10 + u64::from(self.digit(p) - b'0');
        }
        let (mut f, mut t, mut w) = (0u64, 0u64, 0u64);
        for k in 1..=frac.min(18) {
            let d = u64::from(self.digit(-k) - b'0');
            f = f * 10 + d;
            if d != 0 {
                w = k as u64;
            }
        }
        for k in 1..=w as i32 {
            t = t * 10 + u64::from(self.digit(-k) - b'0');
        }
        // `n` is the value as a double (ICU's `toDouble`): integer part plus the first fraction
        // digits; past 10^18 (`i` keeps the last 18 digits) the digits read as an `f64`, so
        // `1e21` is no `n = 0` (`ro`: `other`, not `few`)
        let n = if self.n > 18 {
            self.value()
        } else {
            i as f64 + f as f64 / 10f64.powi(frac.min(18))
        };
        Operands {
            n,
            i: i % I_MOD,
            v: frac.max(0) as u64,
            w,
            f,
            t,
            e: 0,
            c: 0,
        }
    }
}

/// Where the formatter writes; tracks the last char written (currency spacing reads it).
struct W<'w> {
    out: &'w mut dyn fmt::Write,
}

impl W<'_> {
    #[inline]
    fn s(&mut self, s: &str) -> fmt::Result {
        self.out.write_str(s)
    }
}

/// The ten digits of the locale, as `&str` each (`["0", …]` for Latin).
pub(super) fn digit_table(loc: &Loc) -> [&'static str; 10] {
    const LATN: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let all = data::s(loc.digits);
    if all.is_empty() {
        return LATN;
    }
    let mut out = LATN;
    for (k, (at, c)) in all.char_indices().take(10).enumerate() {
        if let Some(slot) = out.get_mut(k) {
            *slot = all.get(at..at + c.len_utf8()).unwrap_or("");
        }
    }
    out
}

/// ICU `currencySpacing` match `[[:^S:]&[:^Z:]]`: not a symbol nor a separator. The symbols
/// and separators that start or end a currency symbol of CLDR are in `EDGE`.
fn spaced(c: Option<char>) -> bool {
    c.is_some_and(|c| match u8::try_from(c) {
        // ASCII: a byte search (no char decoding of `EDGE`, whose other chars are not ASCII)
        Ok(b) if b.is_ascii() => {
            b.is_ascii_alphanumeric()
                || !(b.is_ascii_whitespace() || b == 0x0b || data::EDGE.as_bytes().contains(&b))
        }
        _ => !c.is_whitespace() && !data::EDGE.contains(c),
    })
}

/// Writes `affix`, `¤` replaced by `cur`; `prefix`: the number follows (spacing on the
/// right), else it precedes. `digit`: the number's side is a digit (not `NaN` / `∞`).
fn affix(w: &mut W<'_>, affix: &str, cur: &str, prefix: bool, digit: bool) -> fmt::Result {
    const SIGN: char = '\u{a4}';
    let Some(at) = affix.find(SIGN) else {
        return w.s(affix);
    };
    let (before, after) = (
        affix.get(..at).unwrap_or(""),
        affix.get(at + SIGN.len_utf8()..).unwrap_or(""),
    );
    if !prefix && before.is_empty() && digit && spaced(cur.chars().next()) {
        w.s("\u{a0}")?;
    }
    w.s(before)?;
    w.s(cur)?;
    w.s(after)?;
    if prefix && after.is_empty() && digit && spaced(cur.chars().next_back()) {
        w.s("\u{a0}")?;
    }
    Ok(())
}

/// ECMA-402 currency digits (ISO 4217 as CLDR `currencyData` has them, default 2).
pub(crate) fn currency_digits(code: [u8; 3]) -> u8 {
    data::FRACTIONS
        .binary_search_by(|(c, _)| c.cmp(&code))
        .ok()
        .and_then(|i| data::FRACTIONS.get(i))
        .map_or(2, |&(_, d)| d)
}

/// The nearest record of `code` in `loc` or a parent (symbols and own format).
fn currency(loc: &Loc, code: &[u8; 3]) -> Option<&'static data::Currency> {
    let mut at = Some(loc);
    while let Some(l) = at {
        let start = usize::from(l.currencies);
        let own = CURRENCIES
            .get(start..start + usize::from(l.currencies_len))
            .unwrap_or(&[]);
        if let Ok(i) = own.binary_search_by(|c| c.code.cmp(code)) {
            return own.get(i);
        }
        at = LOCALES.get(usize::from(l.parent));
    }
    None
}

/// The symbol of a currency record (`narrow`: the narrow one, else the symbol), else the code.
fn symbol<'a>(rec: Option<&data::Currency>, code: &'a [u8; 3], narrow: bool) -> &'a str {
    match rec {
        Some(c) if narrow && c.narrow != 0 => data::s(c.narrow),
        Some(c) if c.symbol != 0 => data::s(c.symbol),
        _ => code_str(code),
    }
}

/// The display name of `code` for category `cat` in `loc` or a parent (`None`: none).
#[cfg(feature = "currency-names")]
fn name(loc: &Loc, code: &[u8; 3], cat: u8) -> Option<&'static str> {
    let mut at = Some(loc);
    while let Some(l) = at {
        let start = l.names as usize;
        let own = data::NAMES
            .get(start..start + usize::from(l.names_len))
            .unwrap_or(&[]);
        if let Ok(i) = own.binary_search_by(|n| (n.code, n.cat).cmp(&(*code, cat)))
            && let Some(n) = own.get(i)
        {
            return (n.name != 0).then(|| data::name_str(n.name));
        }
        at = LOCALES.get(usize::from(l.parent));
    }
    None
}

/// Compact patterns of `loc` (sorted by magnitude, then category).
fn compact_list(loc: &Loc) -> &'static [data::Compact] {
    let start = usize::from(loc.compact);
    COMPACT
        .get(start..start + usize::from(loc.compact_len))
        .unwrap_or(&[])
}

/// ICU `CompactData::getMultiplier`: the power of ten a number of magnitude `mag` is
/// divided by (as a negative shift), 0 for no compact form.
fn multiplier(list: &[data::Compact], mag: i32) -> i32 {
    let Some(largest) = list.last().map(|c| i32::from(c.mag)) else {
        return 0;
    };
    if mag < 0 {
        return 0;
    }
    let mag = mag.min(largest);
    list.iter()
        .find(|c| i32::from(c.mag) == mag && c.cat == Category::Other as u8)
        .filter(|c| c.zeros > 0)
        .map_or(0, |c| i32::from(c.zeros) - mag - 1)
}

/// ICU `CompactData::getPattern`: the pattern for magnitude `mag` and category `cat`.
fn compact_pattern(
    list: &'static [data::Compact],
    mag: i32,
    cat: Category,
) -> Option<&'static data::Compact> {
    let largest = i32::from(list.last()?.mag);
    if mag < 0 {
        return None;
    }
    let mag = mag.min(largest);
    let at = |c: u8| list.iter().find(|p| i32::from(p.mag) == mag && p.cat == c);
    at(cat as u8)
        .or_else(|| at(Category::Other as u8))
        .filter(|p| p.zeros > 0)
}

/// A number ready to write: its affixes, rounded digits and shape.
struct Shown<'a> {
    pp: &'static str,
    ps: &'static str,
    cur: &'a str,
    decimal: &'static str,
    group: &'static str,
    /// `NaN` / `∞` instead of digits.
    special: Option<&'static str>,
    d: Decimal,
    int: i32,
    frac: i32,
    g1: i32,
    g2: i32,
}

/// Formats `v` with `o` in `loc` into `out`.
pub(crate) fn write(v: f64, o: &NumberOptions, loc: &Loc, out: &mut dyn fmt::Write) -> fmt::Result {
    let w = &mut W { out };
    let shown = prepare(v, o, loc);
    #[cfg(feature = "currency-names")]
    if o.style == Style::Currency && o.display == Display::Name {
        let cat = if shown.special.is_some() {
            Category::Other
        } else {
            plural::select_rule(loc.plural, &shown.d.operands(shown.frac))
        };
        let unit = |c: Category| loc.units.get(c as usize).copied().filter(|&u| u != 0);
        let pattern = data::s(unit(cat).or_else(|| unit(Category::Other)).unwrap_or(0));
        let pattern = if pattern.is_empty() {
            "{0} {1}"
        } else {
            pattern
        };
        let code = std::str::from_utf8(&o.currency).unwrap_or("");
        let label = name(loc, &o.currency, cat as u8)
            .or_else(|| name(loc, &o.currency, Category::Other as u8))
            .unwrap_or(code);
        let mut rest = pattern;
        while let Some(at) = rest.find('{') {
            w.s(rest.get(..at).unwrap_or(""))?;
            let tail = rest.get(at..).unwrap_or("");
            if let Some(after) = tail.strip_prefix("{0}") {
                emit(w, &shown, loc, o)?;
                rest = after;
            } else if let Some(after) = tail.strip_prefix("{1}") {
                w.s(label)?;
                rest = after;
            } else {
                w.s("{")?;
                rest = tail.get(1..).unwrap_or("");
            }
        }
        return w.s(rest);
    }
    emit(w, &shown, loc, o)
}

/// Rounds `v` and picks the affixes (`o.compact`: the compact pattern of the rounded value).
fn prepare<'a>(v: f64, o: &'a NumberOptions, loc: &Loc) -> Shown<'a> {
    let negative = v.is_sign_negative() && !v.is_nan();
    let rec = if o.style == Style::Currency {
        currency(loc, &o.currency)
    } else {
        None
    };
    let own_pat = rec.filter(|c| c.pattern != u16::MAX).map(|c| c.pattern);
    // `currencyDisplay: name` shows the number by the decimal pattern, unless the currency has
    // its own format (`EUR` in `en-150`) or the locale shows the symbol there (`en-DE`), as ICU
    let display = match o.display {
        #[cfg(feature = "currency-names")]
        Display::Name if own_pat.is_some() || loc.name_cur => Display::Symbol,
        d => d,
    };
    let pat: &Pat = PATTERNS
        .get(usize::from(match (o.style, display) {
            (Style::Percent, _) => loc.percent_pat,
            (Style::Currency, Display::Name) | (Style::Decimal, _) => loc.decimal_pat,
            (Style::Currency, _) => own_pat.unwrap_or(loc.currency_pat),
        }))
        .unwrap_or(&PATTERNS[0]);
    let cur = match (o.style, display) {
        (Style::Currency, Display::Symbol) => symbol(rec, &o.currency, false),
        (Style::Currency, Display::Narrow) => symbol(rec, &o.currency, true),
        (Style::Currency, Display::Code) => code_str(&o.currency),
        _ => "",
    };
    // monetary separators for currencies, the record's own first
    let (decimal, group) = if o.style == Style::Currency {
        let pick = |own: u16, locale: u16| data::s(if own != 0 { own } else { locale });
        (
            pick(rec.map_or(0, |c| c.decimal), loc.cur_decimal),
            pick(rec.map_or(0, |c| c.group), loc.cur_group),
        )
    } else {
        (data::s(loc.decimal), data::s(loc.group))
    };
    let (pp, ps) = if negative {
        (data::s(pat.np), data::s(pat.ns))
    } else {
        (data::s(pat.pp), data::s(pat.ps))
    };
    let mut shown = Shown {
        pp,
        ps,
        cur,
        decimal,
        group,
        special: None,
        d: Decimal::ZERO,
        int: 1,
        frac: 0,
        g1: i32::from(pat.g1),
        g2: i32::from(pat.g2).max(1),
    };
    if !v.is_finite() {
        shown.special = Some(data::s(if v.is_nan() { loc.nan } else { loc.infinity }));
        return shown;
    }
    let mut d = Decimal::of(v.abs());
    if o.style == Style::Percent && !d.is_zero() {
        d.n += 2;
    }
    if o.compact {
        let list = compact_list(loc);
        let round = |d: &mut Decimal| {
            if o.frac_given {
                d.round_frac(i32::from(o.max_frac));
            } else {
                // `Precision::integer().withMinDigits(2)`: whole, but two significant digits
                d.round_at(d.n.max(2));
            }
        };
        let mut mult = 0;
        if d.is_zero() {
            round(&mut d);
        } else {
            let mag = d.n - 1;
            mult = multiplier(list, mag);
            let original = d;
            d.n += mult;
            round(&mut d);
            if !d.is_zero() && d.n - 1 != mag + mult {
                // 999.9 → 1000: the next magnitude's form when it differs ("1K", not "1000")
                let next = multiplier(list, mag + 1);
                if next != mult {
                    d = original;
                    d.n += next;
                    round(&mut d);
                    mult = next;
                }
            }
        }
        let mag = if d.is_zero() { 0 } else { d.n - 1 - mult };
        let (_, frac) = d.shape(o.min_int, o.min_frac);
        let cat = plural::select_rule(loc.plural, &d.operands(frac));
        if let Some(c) = compact_pattern(list, mag, cat) {
            (shown.pp, shown.ps) = if negative {
                (data::s(c.np), data::s(c.ns))
            } else {
                (data::s(c.pp), data::s(c.ps))
            };
        }
    } else {
        d.round_frac(i32::from(o.max_frac));
    }
    (shown.int, shown.frac) = d.shape(o.min_int, o.min_frac);
    shown.d = d;
    shown
}

/// A currency code as text.
fn code_str(code: &[u8; 3]) -> &str {
    std::str::from_utf8(code).unwrap_or("")
}

/// Writes the prepared number: prefix, grouped integer digits, fraction, suffix.
fn emit(w: &mut W<'_>, s: &Shown<'_>, loc: &Loc, o: &NumberOptions) -> fmt::Result {
    if let Some(body) = s.special {
        affix(w, s.pp, s.cur, true, false)?;
        w.s(body)?;
        return affix(w, s.ps, s.cur, false, false);
    }
    affix(w, s.pp, s.cur, true, true)?;
    let digits = digit_table(loc);
    let group = s.group;
    let min_grouping = match o.grouping {
        Grouping::Auto => i32::from(loc.min_grouping),
        Grouping::Min2 => i32::from(loc.min_grouping).max(2),
        Grouping::Always => 1,
        Grouping::Off => i32::MAX,
    };
    let grouped = s.g1 > 0 && s.int - s.g1 >= min_grouping;
    let d = &s.d;
    for p in (0..s.int).rev() {
        w.s(digits
            .get(usize::from(d.digit(p) - b'0'))
            .copied()
            .unwrap_or("0"))?;
        if grouped && p >= s.g1 && (p - s.g1) % s.g2 == 0 {
            w.s(group)?;
        }
    }
    if s.frac > 0 {
        w.s(s.decimal)?;
        for k in 1..=s.frac {
            w.s(digits
                .get(usize::from(d.digit(-k) - b'0'))
                .copied()
                .unwrap_or("0"))?;
        }
    }
    affix(w, s.ps, s.cur, false, true)
}
