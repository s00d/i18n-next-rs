//! The CLDR number data `build.rs` selected (DESIGN 2.4, 5.2): `&'static` tables, strings
//! interned in one text (`STR`, `ENDS`), indices as `u16`. See `build.rs` for the layout.

/// One locale (its name: the key at the same index of `LOCALE_KEYS`): symbols, patterns
/// (indices into [`PATTERNS`]), its compact list and its own currency symbols (the ones that
/// differ from `parent`'s).
#[derive(Debug)]
pub(crate) struct Loc {
    /// Index into [`LOCALES`], `u16::MAX` for `und`.
    pub(crate) parent: u16,
    /// The ten digits of a non-Latin default numbering system, `""` for `latn`.
    pub(crate) digits: u16,
    pub(crate) decimal: u16,
    pub(crate) group: u16,
    /// ICU's monetary separators (`currencyDecimal` / `currencyGroup`, else the plain ones).
    pub(crate) cur_decimal: u16,
    pub(crate) cur_group: u16,
    pub(crate) nan: u16,
    pub(crate) infinity: u16,
    pub(crate) decimal_pat: u16,
    pub(crate) percent_pat: u16,
    pub(crate) currency_pat: u16,
    pub(crate) min_grouping: u8,
    pub(crate) compact: u16,
    pub(crate) compact_len: u8,
    pub(crate) currencies: u16,
    pub(crate) currencies_len: u16,
    /// The locale's own name (8b.14), `""` when ICU has none.
    pub(crate) autonym: u16,
    /// The plural rule set of the locale (`plural::RULES`, `u8::MAX`: none, `other`): compact
    /// patterns and currency names by plural form, resolved once by `scripts/cldr.mjs`.
    pub(crate) plural: u8,
    /// `unitPattern-count-*` (`{0} {1}`) by category, `0` when absent.
    #[cfg(feature = "currency-names")]
    pub(crate) units: [u16; 6],
    #[cfg(feature = "currency-names")]
    pub(crate) names: u32,
    #[cfg(feature = "currency-names")]
    pub(crate) names_len: u16,
    /// `currencyDisplay: name` shows the number as `symbol` does (`en-DE`: `US$1,234.50 US
    /// dollars`), not by the decimal pattern.
    #[cfg(feature = "currency-names")]
    pub(crate) name_cur: bool,
    /// Date data (M6): the rows of [`DATE_PATS`], [`DATE_TIMES`], [`DATE_NAMES`] and [`DATE_REL`], the
    /// row of [`DATE_B`] (`u16::MAX`: no flexible day periods), the overrides
    /// `DATE_OVER[dover..dover + dover_len]` and the hour letters of `hour12: true` /
    /// `false` (`h` or `K`, `H` or `k`).
    pub(crate) dpats: u16,
    pub(crate) dtimes: u16,
    pub(crate) dnames: u16,
    pub(crate) drel: u16,
    pub(crate) db: u16,
    pub(crate) dover: u32,
    pub(crate) dover_len: u16,
    pub(crate) hc12: u8,
    pub(crate) hc24: u8,
}

/// A parsed pattern: positive / negative prefix and suffix (signs in place, `¤` for the
/// currency), primary / secondary grouping (0: none).
#[derive(Debug)]
pub(crate) struct Pat {
    pub(crate) pp: u16,
    pub(crate) ps: u16,
    pub(crate) np: u16,
    pub(crate) ns: u16,
    pub(crate) g1: u8,
    pub(crate) g2: u8,
}

/// A compact short pattern for 10^`mag` and plural category `cat` (`Category` order);
/// `zeros` 0: no compact form there.
#[derive(Debug)]
pub(crate) struct Compact {
    pub(crate) mag: u8,
    pub(crate) cat: u8,
    pub(crate) pp: u16,
    pub(crate) ps: u16,
    pub(crate) np: u16,
    pub(crate) ns: u16,
    pub(crate) zeros: u8,
}

/// A currency of a locale: symbols (`0`, `""`: none, the ISO code shows) and its own format
/// (`pattern` into [`PATTERNS`], `u16::MAX`: none; `decimal` / `group`: `0` for the locale's).
#[derive(Debug)]
pub(crate) struct Currency {
    pub(crate) code: [u8; 3],
    pub(crate) symbol: u16,
    pub(crate) narrow: u16,
    pub(crate) pattern: u16,
    pub(crate) decimal: u16,
    pub(crate) group: u16,
}

/// A currency display name for a plural category; `0` (`""`): none here.
#[cfg(feature = "currency-names")]
#[derive(Debug)]
pub(crate) struct Name {
    pub(crate) code: [u8; 3],
    pub(crate) cat: u8,
    pub(crate) name: u32,
}

include!(concat!(env!("OUT_DIR"), "/numbers.rs"));

/// `name` (at most 16 bytes) as `build.rs` keys it: big-endian, zero-padded, so keys sort like
/// the names. `LOCALE_KEYS`, `ALIASES`, `REGIONS` and `SCRIPTS` are sorted by it: one integer
/// compare per binary-search step.
#[inline]
pub(crate) fn key(name: &str) -> Option<u128> {
    let b = name.as_bytes();
    let mut a = [0u8; 16];
    a.get_mut(..b.len())?.copy_from_slice(b);
    Some(u128::from_be_bytes(a))
}

/// The name of the numbering system of `digits` (a `Loc::digits`), `latn` for `""`.
pub(crate) fn numbering(digits: u16) -> &'static str {
    if digits == 0 {
        return "latn";
    }
    NUMBERING
        .iter()
        .find(|&&(d, _)| d == digits)
        .map_or("", |&(_, name)| s(name))
}

/// String `i` of an interned table (`text`, `ends`).
#[inline]
fn at<E: Copy + Into<u32>>(text: &'static str, ends: &[E], i: usize) -> &'static str {
    let start = i
        .checked_sub(1)
        .and_then(|j| ends.get(j))
        .map_or(0, |&e| e.into() as usize);
    let end = ends.get(i).map_or(start, |&e| e.into() as usize);
    text.get(start..end).unwrap_or("")
}

/// Interned string `i`.
#[inline]
pub(crate) fn s(i: u16) -> &'static str {
    at(STR, &ENDS, usize::from(i))
}

/// Compiled date pattern `i` (`build.rs`: `\x01`, letter, width digit per field).
#[inline]
pub(crate) fn dpat(i: u16) -> &'static str {
    at(DPAT_STR, &DPAT_ENDS, usize::from(i))
}

/// Date name or relative-time string `i`.
#[inline]
pub(crate) fn dname(i: u16) -> &'static str {
    at(DNAME_STR, &DNAME_ENDS, usize::from(i))
}

/// Currency name string `i`.
#[cfg(feature = "currency-names")]
pub(crate) fn name_str(i: u32) -> &'static str {
    at(NAME_STR, &NAME_ENDS, i as usize)
}
