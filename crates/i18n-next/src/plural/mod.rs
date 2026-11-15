//! Plurals (DESIGN 5, 1.1): CLDR cardinal rules generated into [`rules`], a tag resolved the
//! way `Intl.PluralRules.supportedLocalesOf` resolves it, and the form selection of
//! `@i18n-micro/core` `pluralForm` (`core/src/text.ts`).
//!
//! - A tag is the locale's `iso`, else its code. A tag `Intl` would reject (`en_US`, a RangeError)
//!   or not support (`ua`, `xx`) has no rules: such a locale takes the index rule, never a host
//!   default. A supported tag resolves like `BestAvailableLocale`: aliases first (`iw` → `he`,
//!   `620` → `PT`), then region and script are dropped until a known locale is left
//!   (`de-AT` → `de`, `pt-Latn-PT` → `pt`, `pt-PT` stays).
//! - Per rule set, the facts `localePlural` keeps per locale (categories of the counts 0..199,
//!   which of them several counts share) are computed on first use from the rule itself and
//!   cached: 200 rule calls once, 0.46–1.1 µs (bench `plural/compute/*`), against 8 KB of
//!   tables (200 bytes × 40 rule sets) in every binary. A count under 200 then reads the
//!   table (0.6 ns), a larger one calls the rule (1.5–7.7 ns).
//!
//! Self-contained (no `crate::` paths): tests and benches include it with `#[path]`.

// ponytail: every rule set and locale is compiled in (~40 functions); DESIGN 2.4 wants only
// the locales of `I18N_NEXT_LOCALES`. That filter lands with the CLDR data stage (M5, the
// `build.rs` that selects locales), and the rule tables go through it then.
#[rustfmt::skip]
#[allow(
    clippy::nonminimal_bool,
    clippy::manual_range_patterns,
    reason = "generated: negations stay `!(..)` (spike S8), lists stay as CLDR writes them"
)]
mod rules;

use std::sync::OnceLock;

/// A CLDR plural category, in template order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Category {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

const CATEGORIES: [Category; 6] = [
    Category::Zero,
    Category::One,
    Category::Two,
    Category::Few,
    Category::Many,
    Category::Other,
];

/// The operands of UTS 35 plural rules: `n` absolute value, `i` integer digits, `v` / `w`
/// visible fraction digits with / without trailing zeros, `f` / `t` those digits as a
/// number, `e` / `c` the compact exponent (always 0 here: no compact notation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Operands {
    pub(crate) n: f64,
    pub(crate) i: u64,
    pub(crate) v: u64,
    pub(crate) w: u64,
    pub(crate) f: u64,
    pub(crate) t: u64,
    pub(crate) e: u64,
    pub(crate) c: u64,
}

/// ICU keeps the integer digits of a plural operand modulo 10^18 (`DecimalQuantity::toLong`
/// truncates), so `1e18` has `i = 0` and is `one` in French, as in every browser.
const I_MOD: u64 = 1_000_000_000_000_000_000;

impl Operands {
    /// A whole number up to 2^53 (a larger count is rounded to an `f64` first, as JS holds
    /// it, and read by [`Operands::whole_f64`]): `n` is exact, `i` is `n`.
    pub(crate) fn int(n: u64) -> Operands {
        Operands {
            n: n as f64,
            i: n,
            v: 0,
            w: 0,
            f: 0,
            t: 0,
            e: 0,
            c: 0,
        }
    }

    /// A whole `f64` past 2^53, as `Intl.PluralRules` (ICU) reads it: `n` is the double
    /// itself; `i` is the last 18 of its shortest decimal digits, turned into a double
    /// (`getPluralOperand` returns one) and taken modulo as that double. `2^61` is
    /// `2305843009213694000`: `i` = 305843009213694000 → 305843009213694016, so `i % 10`
    /// is 6, not the 2 of the exact value nor the 0 of the digits.
    pub(crate) fn whole_f64(x: f64) -> Operands {
        let x = x.abs();
        let last18 = match shortest_whole(x) {
            Some((digits, zeros)) => match 10u64.checked_pow(zeros) {
                Some(p) if zeros < 18 => (digits % (I_MOD / p)) * p,
                _ => 0,
            },
            None => digits_of_huge(x),
        };
        Operands {
            n: x,
            i: last18 as f64 as u64,
            ..Operands::int(0)
        }
    }

    /// A decimal as written (`"1.50"`): `v`, `w`, `f`, `t` from its fraction digits.
    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "used by tests that include this module with #[path]"
    )]
    pub(crate) fn parse(s: &str) -> Operands {
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        let digits = |d: &str| d.bytes().fold(0u64, |a, b| a * 10 + u64::from(b - b'0'));
        let trimmed = frac.trim_end_matches('0');
        Operands {
            n: s.parse().unwrap_or(0.0),
            i: digits(int) % I_MOD,
            v: frac.len() as u64,
            w: trimmed.len() as u64,
            f: digits(frac),
            t: digits(trimmed),
            e: 0,
            c: 0,
        }
    }
}

/// The last 18 of the shortest digits of a whole `f64` from 2^126 up (a count only an
/// `f64` reaches): `{}` spells them padded with zeros, never with an exponent.
#[cfg(feature = "format")]
fn digits_of_huge(x: f64) -> u64 {
    struct Digits(u64);
    impl std::fmt::Write for Digits {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for b in s.bytes().filter(u8::is_ascii_digit) {
                self.0 = (self.0 % (I_MOD / 10)) * 10 + u64::from(b - b'0');
            }
            Ok(())
        }
    }
    let mut d = Digits(0);
    let _ = std::fmt::Write::write_fmt(&mut d, format_args!("{x}"));
    d.0
}

/// Without `format` a whole count past 2^53 comes from an integer (at most 2^64), never here.
#[cfg(not(feature = "format"))]
fn digits_of_huge(_: f64) -> u64 {
    0
}

/// The shortest round-trip digits of a whole `f64` `x` (2^53 ≤ x < 2^126), as JS
/// `Number::toString` and Rust `{}` choose them: `(digits, zeros)` for `digits · 10^zeros`,
/// with no trailing zero in `digits`. Below 2^53, `x` itself; `None` from 2^126 up.
///
/// Integer arithmetic, no `core::fmt`: `x` is `M · 2^e` (`e ≥ 1`), every value in its
/// rounding interval rounds to it (ties to even `M`), so the answer is the multiple of the
/// largest power of 10 inside the interval, the closer one to `x` when there are two
/// (ECMA-262: then the even one). Checked against `{}` (`tests/plural_props.rs`).
pub(crate) fn shortest_whole(x: f64) -> Option<(u64, u32)> {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    const MAX: f64 = 85_070_591_730_234_615_865_843_651_857_942_052_864.0; // 2^126
    if x.is_nan() || x >= MAX || x.fract() != 0.0 || x < 0.0 {
        return None;
    }
    if x < SAFE {
        return Some((x as u64, 0));
    }
    let bits = x.to_bits();
    let fraction = bits & ((1 << 52) - 1);
    let e = ((bits >> 52) & 0x7ff) as u32 - 1075;
    let v = x as u128;
    let up = 1u128 << e;
    // At a power of two the double below is closer: half the gap.
    let down = if fraction == 0 { up / 2 } else { up };
    let even = fraction & 1 == 0;
    // Doubled bounds: `y` rounds to `x` when `2v - down ≤ 2y ≤ 2v + up` (strict unless even).
    let (lo, hi) = (2 * v - down, 2 * v + up);
    let inside = |y: u128| {
        let d = 2 * y;
        if even {
            lo <= d && d <= hi
        } else {
            lo < d && d < hi
        }
    };
    let (mut best, mut zeros, mut p) = (v, 0u32, 1u128);
    loop {
        let next = p * 10;
        let below = v / next * next;
        let above = below + next;
        let pick = match (inside(below), inside(above)) {
            (false, false) => break,
            (true, false) => below,
            (false, true) => above,
            (true, true) => match (v - below).cmp(&(above - v)) {
                std::cmp::Ordering::Less => below,
                std::cmp::Ordering::Greater => above,
                std::cmp::Ordering::Equal if (below / next).is_multiple_of(2) => below,
                std::cmp::Ordering::Equal => above,
            },
        };
        best = pick;
        zeros += 1;
        p = next;
    }
    u64::try_from(best / p).ok().map(|d| (d, zeros))
}

/// A count `pluralForm` reads by category: whole and not negative. Past 2^53 a count is an
/// `f64` and keeps its own spelling (see [`Operands::whole_f64`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Whole {
    /// 0..=2^53.
    Int(u64),
    Big(f64),
}

/// A category list in template order.
#[derive(Debug, Clone, Copy)]
struct Cats {
    list: [Category; 6],
    len: usize,
}

impl Cats {
    fn from_mask(mask: u8) -> Cats {
        let mut cats = Cats {
            list: [Category::Other; 6],
            len: 0,
        };
        for c in CATEGORIES {
            if mask & (1 << c as u8) != 0
                && let Some(slot) = cats.list.get_mut(cats.len)
            {
                *slot = c;
                cats.len += 1;
            }
        }
        cats
    }

    fn position(&self, c: Category) -> Option<usize> {
        self.list.iter().take(self.len).position(|&x| x == c)
    }
}

/// `CATEGORY_PROBE` of `text.ts`: counts below it are read from a table.
const PROBE: usize = 200;

/// What `localePlural` of `text.ts` keeps per locale, here per rule set.
#[derive(Debug)]
pub(crate) struct Plural {
    select: fn(&Operands) -> Category,
    /// Every category of the rule set (`pluralCategories`).
    all: Cats,
    /// Categories counts under [`PROBE`] reach, without `zero`.
    named: Cats,
    /// Bit `k`: several counts under [`PROBE`] share `named[k]`, so its form must show `{count}`.
    shared: u8,
    small: [Category; PROBE],
}

impl Plural {
    fn new(rule: &rules::Rule) -> Plural {
        let mut small = [Category::Other; PROBE];
        let mut seen = [0usize; 6];
        for (n, slot) in small.iter_mut().enumerate() {
            *slot = (rule.select)(&Operands::int(n as u64));
            if let Some(k) = seen.get_mut(*slot as usize) {
                *k += 1;
            }
        }
        let reached = seen
            .iter()
            .enumerate()
            .filter(|&(c, &k)| k > 0 && c != Category::Zero as usize)
            .fold(0u8, |m, (c, _)| m | 1 << c);
        let named = Cats::from_mask(reached);
        let shared = (0..named.len)
            .filter(|&k| {
                let c = named.list.get(k).map_or(0, |&c| c as usize);
                seen.get(c).is_some_and(|&n| n > 1)
            })
            .fold(0u8, |m, k| m | 1 << k);
        Plural {
            select: rule.select,
            all: Cats::from_mask(rule.categories),
            named,
            shared,
            small,
        }
    }

    /// The category of a whole count.
    pub(crate) fn category(&self, count: Whole) -> Category {
        match count {
            Whole::Int(n) => match self.small.get(n as usize) {
                Some(&c) if n < PROBE as u64 => c,
                _ => (self.select)(&Operands::int(n)),
            },
            Whole::Big(x) => (self.select)(&Operands::whole_f64(x)),
        }
    }
}

static DATA: [OnceLock<Plural>; rules::RULES.len()] =
    [const { OnceLock::new() }; rules::RULES.len()];

/// The rules of a tag, `None` for one `Intl.PluralRules` rejects or does not support.
pub(crate) fn for_tag(tag: &str) -> Option<&'static Plural> {
    let i = rule_index(tag)?;
    let slot = DATA.get(i)?;
    if let Some(p) = slot.get() {
        return Some(p);
    }
    let p = compute(i)?;
    Some(slot.get_or_init(|| p))
}

/// The data of rule set `i`, computed afresh (what [`for_tag`] caches; benches time it).
pub(crate) fn compute(i: usize) -> Option<Plural> {
    rules::RULES.get(i).map(Plural::new)
}

/// The select function and category mask of rule set `i` (tests).
#[cfg(test)]
#[allow(
    dead_code,
    reason = "used by tests that include this module with #[path]"
)]
pub(crate) fn rule(i: usize) -> (fn(&Operands) -> Category, u8) {
    (rules::RULES[i].select, rules::RULES[i].categories)
}

/// The rule set of a locale `Intl` knows by this exact name (`pt-PT`, `de`).
pub(crate) fn locale_rule(name: &str) -> Option<usize> {
    let at = find(rules::LOCALES, &rules::LOCALES_ENDS, name)?;
    rules::LOCALE_RULES.get(at).map(|&r| usize::from(r))
}

/// Entry `i` of a concatenated string table.
fn nth(table: &'static str, ends: &[u16], i: usize) -> &'static str {
    let start = i
        .checked_sub(1)
        .and_then(|j| ends.get(j))
        .map_or(0, |&e| usize::from(e));
    let end = ends.get(i).map_or(start, |&e| usize::from(e));
    table.get(start..end).unwrap_or("")
}

/// Binary search of `key` in a sorted concatenated string table.
fn find(table: &'static str, ends: &[u16], key: &str) -> Option<usize> {
    let (mut lo, mut hi) = (0, ends.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match nth(table, ends, mid).as_bytes().cmp(key.as_bytes()) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return Some(mid),
        }
    }
    None
}

fn alpha(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn digit(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

fn alnum(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A short ASCII string on the stack.
#[derive(Clone, Copy)]
struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf {
            bytes: [0; N],
            len: 0,
        }
    }

    fn push(&mut self, s: &str, case: impl Fn(u8) -> u8) {
        for b in s.bytes() {
            if let Some(slot) = self.bytes.get_mut(self.len) {
                *slot = case(b);
                self.len += 1;
            }
        }
    }

    fn of(s: &str, case: impl Fn(u8) -> u8) -> Self {
        let mut buf = Self::new();
        buf.push(s, case);
        buf
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(self.bytes.get(..self.len).unwrap_or(&[])).unwrap_or("")
    }
}

fn lower(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

fn upper(b: u8) -> u8 {
    b.to_ascii_uppercase()
}

/// The parts of a well-formed tag that pick a rule.
struct Tag<'a> {
    language: Buf<8>,
    script: Option<&'a str>,
    region: Option<&'a str>,
    /// The replacement of the first variant with a `language-variant` alias
    /// (`aa-1996-saaho` → `ssy`), looked up as each variant is read.
    variant_alias: Option<&'static str>,
}

/// A variant subtag: 5..8 alphanumerics, or 4 starting with a digit.
fn variant(s: &str) -> bool {
    alnum(s)
        && (matches!(s.len(), 5..=8)
            || (s.len() == 4 && s.as_bytes().first().is_some_and(u8::is_ascii_digit)))
}

/// Whether `v` repeats (ASCII case aside) one of the `-`-separated subtags of `before`.
fn repeats(before: &str, v: &str) -> bool {
    before.split('-').any(|p| p.eq_ignore_ascii_case(v))
}

/// Byte offset of `part` (a slice of `tag`) in `tag`.
fn offset(tag: &str, part: &str) -> usize {
    part.as_ptr() as usize - tag.as_ptr() as usize
}

/// `unicode_locale_id` of UTS 35 as ECMA-402 `IsStructurallyValidLanguageTag` checks it:
/// `None` where `Intl` throws a RangeError (`en_US`, `root`, `i-klingon`, a repeated variant
/// or singleton). Extensions are checked by shape only (singleton, then subtags of 2..8),
/// without a buffer: a tag of any length is read in place, with no allocation.
///
/// Known difference: ICU stops checking a `-u-` extension after a repeated key
/// (`ru-u-rg-rg-a1` is accepted as `ru-u-rg`); this follows the grammar and rejects it
/// (`fixtures/compat/README.md`).
fn parse(tag: &str) -> Option<Tag<'_>> {
    let mut parts = tag.split('-').peekable();
    let language = parts.next()?;
    if !(alpha(language) && matches!(language.len(), 2 | 3 | 5..=8)) {
        return None;
    }
    let mut out = Tag {
        language: Buf::of(language, lower),
        script: None,
        region: None,
        variant_alias: None,
    };
    // Where the variants start in `tag`: a new one is checked against those before it.
    let mut variants_at = None;
    let mut stage = 0;
    while let Some(&s) = parts.peek() {
        if s.len() == 1 {
            break;
        }
        parts.next();
        if stage == 0 && s.len() == 4 && alpha(s) {
            out.script = Some(s);
            stage = 1;
        } else if stage <= 1 && ((s.len() == 2 && alpha(s)) || (s.len() == 3 && digit(s))) {
            out.region = Some(s);
            stage = 2;
        } else if variant(s) {
            let at = offset(tag, s);
            let start = *variants_at.get_or_insert(at);
            if repeats(tag.get(start..at).unwrap_or(""), s) {
                return None;
            }
            if out.variant_alias.is_none() {
                let mut key = Buf::<17>::of(out.language.as_str(), lower);
                key.push("-", lower);
                key.push(s, lower);
                out.variant_alias = language_alias(key.as_str(), tag);
            }
            stage = 2;
        } else {
            return None;
        }
    }
    // Extensions and private use; a singleton is a digit (0..9) or a letter (10..35).
    let mut singletons = 0u64;
    while let Some(s) = parts.next() {
        let c = *s.as_bytes().first()?;
        if s.len() != 1 || !c.is_ascii_alphanumeric() {
            return None;
        }
        let c = c.to_ascii_lowercase();
        if c == b'x' {
            let mut any = false;
            for p in parts.by_ref() {
                if p.is_empty() || p.len() > 8 || !alnum(p) {
                    return None;
                }
                any = true;
            }
            return any.then_some(out);
        }
        let slot = if c.is_ascii_digit() {
            c - b'0'
        } else {
            c - b'a' + 10
        };
        let bit = 1u64 << slot;
        if singletons & bit != 0 {
            return None;
        }
        singletons |= bit;
        // The subtags up to the next singleton, as one slice of `tag`.
        let (mut start, mut end) = (None, 0);
        while let Some(&p) = parts.peek() {
            if p.len() == 1 {
                break;
            }
            if !(matches!(p.len(), 2..=8) && alnum(p)) {
                return None;
            }
            let at = offset(tag, p);
            start.get_or_insert(at);
            end = at + p.len();
            parts.next();
        }
        let ext = start.and_then(|s| tag.get(s..end)).unwrap_or("");
        let valid = match c {
            b'u' => unicode_extension(ext),
            b't' => transformed_extension(ext),
            _ => !ext.is_empty(),
        };
        if !valid {
            return None;
        }
    }
    Some(out)
}

/// `unicode_locale_extensions` after `-u` (`ext`: its subtags joined by `-`): attributes
/// (3..8), then keywords: a key (alphanumeric + letter) and its type subtags (3..8).
fn unicode_extension(ext: &str) -> bool {
    let mut keys = false;
    for p in ext.split('-') {
        let b = p.as_bytes();
        if b.len() == 2 {
            if !b.get(1).is_some_and(u8::is_ascii_alphabetic) {
                return false;
            }
            keys = true;
        } else if !keys && !(3..=8).contains(&b.len()) {
            return false;
        }
    }
    !ext.is_empty()
}

/// `transformed_extensions` after `-t` (`ext`: its subtags joined by `-`): an optional
/// language id (its variants unique, as in the tag itself), then fields: a key (letter +
/// digit) and one or more values (3..8).
fn transformed_extension(ext: &str) -> bool {
    if ext.is_empty() {
        return false;
    }
    let mut it = ext.split('-').peekable();
    let mut lang = false;
    if it
        .next_if(|l| alpha(l) && matches!(l.len(), 2 | 3 | 5..=8))
        .is_some()
    {
        lang = true;
        let _ = it.next_if(|s| s.len() == 4 && alpha(s));
        let _ = it.next_if(|s| (s.len() == 2 && alpha(s)) || (s.len() == 3 && digit(s)));
        let mut variants_at = None;
        while let Some(v) = it.next_if(|s| variant(s)) {
            let at = offset(ext, v);
            let start = *variants_at.get_or_insert(at);
            if repeats(ext.get(start..at).unwrap_or(""), v) {
                return false;
            }
        }
    }
    let mut fields = false;
    while let Some(key) = it.next() {
        let b = key.as_bytes();
        if !(b.len() == 2 && b[0].is_ascii_alphabetic() && b[1].is_ascii_digit()) {
            return false;
        }
        let mut values = 0;
        while it.next_if(|s| (3..=8).contains(&s.len())).is_some() {
            values += 1;
        }
        if values == 0 {
            return false;
        }
        fields = true;
    }
    lang || fields
}

/// Looks `key` up in the sorted region alias table.
fn region_alias(key: &str) -> Option<&'static str> {
    rules::REGION_ALIASES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, to)| *to)
}

/// The replacement of a language alias key, unless `tag` is that key itself, bare and
/// lowercase, and V8 passes such a tag through (`bh`).
fn language_alias(key: &str, tag: &str) -> Option<&'static str> {
    let at = find(rules::ALIAS_KEYS, &rules::ALIAS_KEYS_ENDS, key)?;
    let bare = rules::ALIAS_BARE.get(at).copied().unwrap_or(true);
    (bare || key != tag).then(|| nth(rules::ALIAS_VALUES, &rules::ALIAS_VALUES_ENDS, at))
}

/// The rule set `Intl.PluralRules` uses for `tag`, `None` for the index rule.
pub(crate) fn rule_index(tag: &str) -> Option<usize> {
    let t = parse(tag)?;
    let mut language = t.language;
    let mut script = t.script.map(|s| {
        let mut b = Buf::<4>::of(s, lower);
        if let Some(first) = b.bytes.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        b
    });
    let mut region = t.region.map(|r| Buf::<3>::of(r, upper));
    // A `language-variant` alias first (`art-lojban`), then the language alone (`iw`).
    let replacement = t
        .variant_alias
        .or_else(|| language_alias(language.as_str(), tag));
    if let Some(r) = replacement {
        let mut parts = r.split('-');
        language = Buf::of(parts.next().unwrap_or(""), lower);
        for p in parts {
            if p.len() == 4 && alpha(p) && script.is_none() {
                script = Some(Buf::of(p, |b| b));
            } else if (p.len() == 2 || p.len() == 3) && region.is_none() {
                region = Some(Buf::of(p, upper));
            }
        }
    }
    if let Some(r) = &region
        && let Some(to) = region_alias(r.as_str())
    {
        region = Some(Buf::of(to, upper));
    }
    // BestAvailableLocale: the longest known prefix of language[-script][-region].
    let mut candidate = Buf::<17>::of(language.as_str(), |b| b);
    let base = candidate.len;
    if let Some(s) = &script {
        candidate.push("-", |b| b);
        candidate.push(s.as_str(), |b| b);
    }
    let with_script = candidate.len;
    if let Some(r) = &region {
        candidate.push("-", |b| b);
        candidate.push(r.as_str(), |b| b);
        if let Some(i) = locale_rule(candidate.as_str()) {
            return Some(i);
        }
    }
    candidate.len = with_script;
    if with_script > base
        && let Some(i) = locale_rule(candidate.as_str())
    {
        return Some(i);
    }
    candidate.len = base;
    locale_rule(candidate.as_str())
}

/// What `pluralForm` reads from a template's forms besides their text, found once at load.
#[derive(Debug, Clone)]
pub(crate) struct Shape {
    len: usize,
    /// The first form has a `{`: no categories (`unmarkedIndex`).
    first_brace: bool,
    /// Bit `k`: form `k` has `{count}` (forms past 63 never matter: a locale has ≤ 6).
    counts: u64,
}

impl Shape {
    /// From the forms as split (trimmed or not: neither changes `{` or `{count}`).
    pub(crate) fn new<'a>(forms: impl Iterator<Item = &'a str>) -> Shape {
        let mut shape = Shape {
            len: 0,
            first_brace: false,
            counts: 0,
        };
        for (k, form) in forms.enumerate() {
            if k == 0 {
                shape.first_brace = form.contains('{');
            }
            if k < 64 && form.contains("{count}") {
                shape.counts |= 1 << k;
            }
            shape.len += 1;
        }
        shape
    }

    /// The index of the form `count` picks (`pluralIndex` / `markedIndex` of `text.ts`).
    /// `count` is `None` when negative, fractional or not a number: the last form.
    pub(crate) fn index(
        &self,
        marked: bool,
        count: Option<Whole>,
        plural: Option<&Plural>,
    ) -> usize {
        let last = self.len.saturating_sub(1);
        let Some(count) = count else {
            return last;
        };
        let by_index = || match count {
            Whole::Int(n) => usize::try_from(n).map_or(last, |n| n.min(last)),
            Whole::Big(_) => last,
        };
        let zero = matches!(count, Whole::Int(0));
        if marked {
            if zero {
                return 0;
            }
            let Some(p) = plural else {
                return by_index();
            };
            let cats = if last > p.named.len { p.all } else { p.named };
            return match cats.position(p.category(count)) {
                None => last,
                Some(k) => (k + 1).min(last),
            };
        }
        match plural {
            Some(p) if self.unmarked(p) => {
                if zero {
                    return 0;
                }
                match p.named.position(p.category(count)) {
                    None => last,
                    Some(k) => k + 1,
                }
            }
            _ => by_index(),
        }
    }

    /// `unmarkedIndex` applies: a zero form without `{`, then one form per named category,
    /// each shared one showing `{count}`.
    fn unmarked(&self, p: &Plural) -> bool {
        p.named.len >= 3
            && self.len == p.named.len + 1
            && !self.first_brace
            && (0..p.named.len)
                .all(|k| p.shared & (1 << k) == 0 || self.counts & (1 << (k + 1)) != 0)
    }
}
