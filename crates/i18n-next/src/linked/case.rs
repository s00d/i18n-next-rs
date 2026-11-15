//! `toLocaleUpperCase(tag)` / `toLocaleLowerCase(tag)` as V8 runs them for the case modifiers
//! of linked messages. Every tag gets the default Unicode mapping (`str::to_uppercase` /
//! `to_lowercase`) except those whose language ICU special-cases, after canonicalization
//! (`tur` → `tr`, `gre` → `el`):
//!
//! - `tr`, `az`: `i` → `İ`; `I` → `ı` unless a U+0307 follows (past other accents), which is
//!   then dropped; `İ` → `i`;
//! - `lt`: `I` / `J` / `Į` gain a U+0307 before an accent above, `Ì Í Ĩ` always; upper case
//!   drops a U+0307 after a soft-dotted letter;
//! - `el`: upper case drops accents and breathings, keeps or adds a dialytika, writes an iota
//!   subscript as `Ι`, and keeps the tonos of a standalone `ή` (ICU `GreekUpper`).
//!
//! The Unicode properties these rules read come from Node's ICU (`case_data.rs`, written by
//! `scripts/casing.mjs`); the compat goldens (`linked.md`) pin the results.

use super::case_data as data;

/// The case rules of a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Default,
    Turkic,
    Lithuanian,
    Greek,
}

impl Lang {
    /// The rules `toLocale*Case(tag)` applies; `None` (no tag, or one `Intl` rejects:
    /// `caseTag` of `linked-messages` drops it) is the default mapping.
    pub(crate) fn of(tag: Option<&str>) -> Lang {
        let Some(tag) = tag.filter(|t| !t.is_empty()) else {
            return Lang::Default;
        };
        crate::plural::canonical_language(tag, |lang| match lang {
            "tr" | "tur" | "az" | "aze" => Lang::Turkic,
            "lt" | "lit" => Lang::Lithuanian,
            "el" | "ell" => Lang::Greek,
            _ => Lang::Default,
        })
        .unwrap_or(Lang::Default)
    }
}

fn in_table(table: &[u32], c: char) -> bool {
    let c = c as u32;
    let at = table.partition_point(|&p| p >> 11 <= c);
    at.checked_sub(1)
        .and_then(|i| table.get(i))
        .is_some_and(|&p| c - (p >> 11) <= p & 0x7ff)
}

fn ignorable(c: char) -> bool {
    in_table(&data::CASE_IGNORABLE, c)
}

fn cased(c: char) -> bool {
    in_table(&data::CASED, c)
}

/// ICU `ucase_getDotType`.
#[derive(PartialEq, Eq)]
enum Dot {
    SoftDotted,
    Above,
    OtherAccent,
    None,
}

fn dot(c: char) -> Dot {
    if in_table(&data::SOFT_DOTTED, c) {
        Dot::SoftDotted
    } else if in_table(&data::ABOVE, c) {
        Dot::Above
    } else if in_table(&data::OTHER_ACCENT, c) {
        Dot::OtherAccent
    } else {
        Dot::None
    }
}

/// Past case-ignorable characters, a cased one (ICU `isFollowedByCasedLetter`).
fn cased_next(mut chars: impl Iterator<Item = char>) -> bool {
    chars.find(|&c| !ignorable(c)).is_some_and(cased)
}

/// `Σ` at the end of a word (Unicode `Final_Sigma`).
fn final_sigma(before: &str, after: &str) -> bool {
    cased_next(before.chars().rev()) && !cased_next(after.chars())
}

/// Past accents of other classes, the first character is `want`.
fn past_accents(chars: impl Iterator<Item = char>, want: impl Fn(char) -> bool) -> bool {
    for c in chars {
        if want(c) {
            return true;
        }
        if dot(c) != Dot::OtherAccent {
            return false;
        }
    }
    false
}

const DOT_ABOVE: char = '\u{307}';

/// Upper case of `s` by the rules of `lang`, appended to `out`.
pub(crate) fn upper(s: &str, lang: Lang, out: &mut String) {
    match lang {
        Lang::Default => out.push_str(&s.to_uppercase()),
        Lang::Turkic => {
            for c in s.chars() {
                if c == 'i' {
                    out.push('\u{130}');
                } else {
                    out.extend(c.to_uppercase());
                }
            }
        }
        Lang::Lithuanian => {
            for (i, c) in s.char_indices() {
                let before = s.get(..i).unwrap_or("");
                if c == DOT_ABOVE
                    && past_accents(before.chars().rev(), |p| dot(p) == Dot::SoftDotted)
                {
                    continue;
                }
                out.extend(c.to_uppercase());
            }
        }
        Lang::Greek => greek_upper(s, out),
    }
}

/// Lower case of `s` by the rules of `lang`, appended to `out`.
pub(crate) fn lower(s: &str, lang: Lang, out: &mut String) {
    if matches!(lang, Lang::Default | Lang::Greek) {
        out.push_str(&s.to_lowercase());
        return;
    }
    for (i, c) in s.char_indices() {
        let before = s.get(..i).unwrap_or("");
        let after = s.get(i + c.len_utf8()..).unwrap_or("");
        match (lang, c) {
            (Lang::Turkic, '\u{130}') => out.push('i'),
            (Lang::Turkic, 'I') => {
                out.push(if past_accents(after.chars(), |n| n == DOT_ABOVE) {
                    'i'
                } else {
                    '\u{131}'
                });
            }
            (Lang::Turkic, DOT_ABOVE) if past_accents(before.chars().rev(), |p| p == 'I') => {}
            (Lang::Lithuanian, 'I' | 'J' | '\u{12e}')
                if past_accents(after.chars(), |n| dot(n) == Dot::Above) =>
            {
                out.extend(c.to_lowercase());
                out.push(DOT_ABOVE);
            }
            (Lang::Lithuanian, '\u{cc}') => out.push_str("i\u{307}\u{300}"),
            (Lang::Lithuanian, '\u{cd}') => out.push_str("i\u{307}\u{301}"),
            (Lang::Lithuanian, '\u{128}') => out.push_str("i\u{307}\u{303}"),
            (_, '\u{3a3}') => out.push(if final_sigma(before, after) {
                '\u{3c2}'
            } else {
                '\u{3c3}'
            }),
            _ => out.extend(c.to_lowercase()),
        }
    }
}

// ICU `GreekUpper` letter flags, as `scripts/casing.mjs` writes them.
const UPPER_MASK: u32 = 0x3ff;
const HAS_VOWEL: u32 = 0x400;
const HAS_YPOGEGRAMMENI: u32 = 0x800;
const HAS_ACCENT: u32 = 0x1000;
const HAS_DIALYTIKA: u32 = 0x2000;
const HAS_OTHER_GREEK_DIACRITIC: u32 = 0x4000;
const HAS_COMBINING_DIALYTIKA: u32 = 0x8000;
const HAS_EITHER_DIALYTIKA: u32 = HAS_DIALYTIKA | HAS_COMBINING_DIALYTIKA;
const HAS_VOWEL_AND_ACCENT: u32 = HAS_VOWEL | HAS_ACCENT;
const HAS_VOWEL_AND_ACCENT_AND_DIALYTIKA: u32 = HAS_VOWEL_AND_ACCENT | HAS_DIALYTIKA;

// States between letters.
const AFTER_CASED: u32 = 1;
const AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT: u32 = 2;
const AFTER_VOWEL_WITH_COMBINING_ACCENT: u32 = 4;

fn letter_data(c: char) -> u32 {
    let c = c as u32;
    let entry = match c {
        0x370..=0x3ff => data::GREEK_0370.get((c - 0x370) as usize),
        0x1f00..=0x1fff => data::GREEK_1F00.get((c - 0x1f00) as usize),
        0x2126 => return 0x3a9 | HAS_VOWEL,
        _ => None,
    };
    entry.map_or(0, |&d| u32::from(d))
}

fn diacritic_data(c: char) -> u32 {
    match c {
        // varia, tonos = oxia, perispomeni, and what can look like it
        '\u{300}' | '\u{301}' | '\u{342}' | '\u{302}' | '\u{303}' | '\u{311}' => HAS_ACCENT,
        '\u{308}' => HAS_COMBINING_DIALYTIKA,
        '\u{344}' => HAS_COMBINING_DIALYTIKA | HAS_ACCENT,
        '\u{345}' => HAS_YPOGEGRAMMENI,
        '\u{304}' | '\u{306}' | '\u{313}' | '\u{314}' | '\u{343}' => HAS_OTHER_GREEK_DIACRITIC,
        _ => 0,
    }
}

fn push_code(out: &mut String, c: u32) {
    if let Some(c) = char::from_u32(c) {
        out.push(c);
    }
}

/// ICU `GreekUpper::toUpper`.
fn greek_upper(s: &str, out: &mut String) {
    let mut state = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        let mut next_state = 0;
        if ignorable(c) {
            next_state |= state & AFTER_CASED;
        } else if cased(c) {
            next_state |= AFTER_CASED;
        }
        let mut data = letter_data(c);
        if data == 0 {
            out.extend(c.to_uppercase());
            state = next_state;
            continue;
        }
        let mut upper = data & UPPER_MASK;
        // A dialytika on this iota or upsilon when the vowel before lost its tonos.
        if data & HAS_VOWEL != 0
            && state & (AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT | AFTER_VOWEL_WITH_COMBINING_ACCENT)
                != 0
            && (upper == 0x399 || upper == 0x3a5)
        {
            data |= if state & AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT != 0 {
                HAS_DIALYTIKA
            } else {
                HAS_COMBINING_DIALYTIKA
            };
        }
        let mut ypogegrammeni = usize::from(data & HAS_YPOGEGRAMMENI != 0);
        let precomposed_accent = data & HAS_ACCENT != 0;
        // The combining diacritics after the letter belong to it.
        while let Some(&(_, d)) = chars.peek() {
            let extra = diacritic_data(d);
            if extra == 0 {
                break;
            }
            data |= extra;
            if extra & HAS_YPOGEGRAMMENI != 0 {
                ypogegrammeni += 1;
            }
            chars.next();
        }
        if data & HAS_VOWEL_AND_ACCENT_AND_DIALYTIKA == HAS_VOWEL_AND_ACCENT {
            next_state |= if precomposed_accent {
                AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT
            } else {
                AFTER_VOWEL_WITH_COMBINING_ACCENT
            };
        }
        let mut add_tonos = false;
        let rest = chars.peek().map_or("", |&(i, _)| s.get(i..).unwrap_or(""));
        if upper == 0x397
            && data & HAS_ACCENT != 0
            && ypogegrammeni == 0
            && state & AFTER_CASED == 0
            && !cased_next(rest.chars())
        {
            // The disjunctive "or" keeps its tonos.
            if precomposed_accent {
                upper = 0x389;
            } else {
                add_tonos = true;
            }
        } else if data & HAS_DIALYTIKA != 0 {
            // A vowel with dialytika stays precomposed when it can.
            if upper == 0x399 {
                upper = 0x3aa;
                data &= !HAS_EITHER_DIALYTIKA;
            } else if upper == 0x3a5 {
                upper = 0x3ab;
                data &= !HAS_EITHER_DIALYTIKA;
            }
        }
        push_code(out, upper);
        if data & HAS_EITHER_DIALYTIKA != 0 {
            out.push('\u{308}');
        }
        if add_tonos {
            out.push('\u{301}');
        }
        for _ in 0..ypogegrammeni {
            out.push('\u{399}');
        }
        state = next_state;
    }
}
