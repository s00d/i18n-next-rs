//! The generated CLDR rules against `Intl.PluralRules` (DESIGN 7.3a, spike S8), without
//! Node: `fixtures/compat/__golden__/plural-categories.md` (written by `scripts/cldr.mjs`)
//! holds the category of 0..1000, of decimals and of whole `f64` past 2^53 for every locale
//! `Intl` supports, the category set of each, and where `Intl` resolves a few hundred tags
//! (aliases, regions, scripts, extensions, malformed ones). The plural module is
//! crate-private: it is included with `#[path]`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../src/plural/mod.rs"]
#[allow(dead_code)]
mod plural;

use plural::{Category, Operands};

const GOLDEN: &str = include_str!("../../../fixtures/compat/__golden__/plural-categories.md");

/// `call(arg) → "result"` lines as (name, first argument, result without quotes).
fn lines() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    GOLDEN.lines().filter(|l| l.contains(" → ")).map(|l| {
        let (call, result) = l.split_once(" → ").unwrap();
        let (name, arg) = call.split_once('(').unwrap();
        let arg = arg.strip_suffix(')').unwrap();
        (name, arg.trim_matches('"'), result.trim_matches('"'))
    })
}

fn letter(c: Category) -> char {
    match c {
        Category::Zero => 'z',
        Category::One => 'o',
        Category::Two => 't',
        Category::Few => 'f',
        Category::Many => 'm',
        Category::Other => 'x',
    }
}

fn list(name: &str) -> Vec<&'static str> {
    let (_, _, r) = lines().find(|(n, _, _)| *n == name).unwrap();
    r.split(' ').collect()
}

/// Given Intl's categories of every supported locale, when the generated rule of that
/// locale reads the same numbers, then each category matches.
#[test]
fn rules_match_intl_for_every_locale() {
    let decimals = list("decimal_list");
    let big = list("big_list");
    let mut locales = 0;
    let mut checked = 0;
    for (name, locale, expected) in lines() {
        if !matches!(name, "categories" | "integers" | "decimals" | "big") {
            continue;
        }
        // step 1: Intl knows the locale by this name → so does the table
        let rule = plural::locale_rule(locale).unwrap_or_else(|| panic!("{locale} not in LOCALES"));
        let (select, mask) = plural::rule(rule);
        // step 2: each number → the same category as Intl
        let got: String = match name {
            "categories" => {
                locales += 1;
                let names = ["zero", "one", "two", "few", "many", "other"];
                let got: Vec<&str> = (0..6)
                    .filter(|b| mask & (1 << b) != 0)
                    .map(|b| names[b])
                    .collect();
                assert_eq!(got.join(" "), expected, "categories of {locale}");
                continue;
            }
            "integers" => (0..=1000u64)
                .map(|n| letter(select(&Operands::int(n))))
                .collect(),
            "decimals" => decimals
                .iter()
                .map(|d| letter(select(&Operands::parse(d))))
                .collect(),
            _ => big
                .iter()
                .map(|b| letter(select(&Operands::whole_f64(b.parse().unwrap()))))
                .collect(),
        };
        checked += got.len();
        assert_eq!(got, expected, "{name} of {locale}");
    }
    assert!(locales > 200, "only {locales} locales");
    assert!(checked > 200_000, "only {checked} numbers");
}

/// Tags ICU accepts against the BCP 47 grammar, rejected here on purpose
/// (`fixtures/compat/README.md`): after a repeated `-u-` key ICU stops checking, so `a1`
/// (no valid key) passes.
const KNOWN_DIFFERENCES: &[&str] = &["ru-u-rg-rg-a1"];

/// Given whole doubles from 2^53 to 2^126, when their shortest digits are found with
/// integers ([`plural::shortest_whole`], what `Count` prints and the plural operand reads),
/// then they are the digits `{}` prints (shortest round-trip, padded with zeros); below
/// 2^53 the number itself, from 2^126 up `None`.
#[test]
fn shortest_whole_matches_display() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut checked = 0;
    for i in 0..400_000u32 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let x = match i % 4 {
            // any mantissa at any exponent of the range
            0 | 1 => ((state >> 11) | 1 << 52) as f64 * 2f64.powi(1 + (state % 73) as i32),
            // powers of two and their neighbours: the uneven rounding interval
            2 => {
                f64::from_bits((2f64.powi(53 + (state % 73) as i32)).to_bits() + (state >> 62) - 1)
            }
            // round decimals: many trailing zeros
            _ => ((state >> 40) as f64) * 10f64.powi(10 + (state % 20) as i32),
        };
        if !(9_007_199_254_740_992.0..2f64.powi(126)).contains(&x) || x.fract() != 0.0 {
            continue;
        }
        let (digits, zeros) = plural::shortest_whole(x).unwrap();
        assert_ne!(digits % 10, 0, "{x:e}");
        assert_eq!(
            format!("{digits}{}", "0".repeat(zeros as usize)),
            format!("{x}"),
            "{x:e}"
        );
        checked += 1;
    }
    assert!(checked > 350_000, "only {checked}");
    assert_eq!(plural::shortest_whole(12345.0), Some((12345, 0)));
    assert_eq!(plural::shortest_whole(2f64.powi(126)), None);
    // `u64::MAX` as JS holds it: 2^64, `18446744073709552000`
    assert_eq!(
        plural::shortest_whole(2f64.powi(64)),
        Some((18446744073709552, 3))
    );
}

/// Given the tags Intl resolved (or rejected), when Rust resolves the same tag, then it gets
/// the rule of the locale Intl resolved to, and no rule where Intl threw or knew nothing.
#[test]
fn tags_resolve_like_supported_locales_of() {
    let mut tags = 0;
    for (name, tag, expected) in lines().filter(|(n, _, _)| *n == "tag") {
        let _ = name;
        tags += 1;
        if KNOWN_DIFFERENCES.contains(&tag) {
            // ICU accepts it, the grammar does not: no rules here (the index rule).
            assert!(
                !expected.starts_with('<'),
                "{tag}: Intl agrees now ({expected})"
            );
            assert_eq!(plural::rule_index(tag), None, "{tag}");
            continue;
        }
        // `JSON.stringify` spelling: none of the tags needs an escape
        let want = if expected.starts_with('<') {
            None
        } else {
            // `en-u-va-posix` → `en`: extensions pick no rule
            let base = expected.split("-u-").next().unwrap();
            Some(
                plural::locale_rule(base)
                    .or_else(|| plural::locale_rule(base.split('-').next().unwrap()))
                    .unwrap_or_else(|| panic!("{expected} has no rule")),
            )
        };
        // step 1: same rule (or the index rule) as Intl
        assert_eq!(
            plural::rule_index(tag),
            want,
            "tag {tag:?} (Intl: {expected})"
        );
        // step 2: the cached locale data agrees on whether there are rules
        assert_eq!(
            plural::for_tag(tag).is_some(),
            want.is_some(),
            "for_tag {tag:?}"
        );
    }
    assert!(tags > 600, "only {tags} tags");
}

/// Given a whole count, when it is read through the cached table (under 200) or the rule
/// (above), then both agree with the rule itself.
#[test]
fn small_table_agrees_with_the_rule() {
    for tag in [
        "ru", "ar", "fr", "cy", "lv", "en", "ja", "pt-PT", "kok-Latn",
    ] {
        let data = plural::for_tag(tag).unwrap();
        let (select, _) = plural::rule(plural::rule_index(tag).unwrap());
        for n in 0..1000u64 {
            assert_eq!(
                data.category(plural::Whole::Int(n)),
                select(&Operands::int(n)),
                "{tag} {n}"
            );
        }
    }
}

/// Given tags at the edges of the UTS 35 grammar (M10 mutants), when `Intl` would check
/// them, then the same ones are rejected: a 4-letter language, a singleton that is not
/// alphanumeric, a private-use or extension subtag too long or not alphanumeric, a repeated
/// singleton, a `-u-` key whose second character is a digit, a `-t-` field without a value
/// or with a key that is not letter + digit.
#[test]
fn tag_grammar_edges() {
    let valid = |t: &str| plural::canonical_language(t, |_| ()).is_some();
    for t in [
        "en-x-abcdefgh",
        "en-a-bc-b-cd",
        "en-u-abc",
        "en-u-ca-gregory",
        "en-t-de-419",
        "en-t-de-at-h0-abc",
        "en-t-h0-abc-defg",
    ] {
        assert!(valid(t), "{t} is valid");
    }
    for t in [
        "abcd",
        "en-$-ab",
        "en-x-abcdefghi",
        "en-x-a$",
        "en-a-bc-a-cd",
        "en-a-b$c",
        "en-u-a1",
        "en-t-de-at-aa-abc",
        "en-t-h0",
        "en-t-h0-ab",
    ] {
        assert!(!valid(t), "{t} is not valid");
    }
}

/// Given the locales whose rules depend on a script or a region (`kok-Latn`, `pt-PT`), when
/// a tag names them (the region through its numeric alias too), then their own rule set is
/// picked, not the language's.
#[test]
fn script_and_region_pick_their_rule_set() {
    assert_eq!(
        plural::rule_index("kok-Latn"),
        plural::locale_rule("kok-Latn")
    );
    assert_eq!(plural::rule_index("pt-620"), plural::locale_rule("pt-PT"));
    assert_ne!(plural::locale_rule("pt-PT"), plural::locale_rule("pt"));
}

/// `shortest_whole` takes whole, non-negative values only: a fraction or a negative number
/// is `None`, zero is itself.
#[test]
fn shortest_whole_domain() {
    assert_eq!(plural::shortest_whole(0.0), Some((0, 0)));
    assert_eq!(plural::shortest_whole(0.5), None);
    assert_eq!(plural::shortest_whole(-5.0), None);
}

/// Given counts the golden table does not reach (Cornish `two` for thousands, Latvian
/// decimals with two fraction digits), when the generated rules read them, then they pick
/// the category `Intl.PluralRules` does (Node, ICU 78).
#[test]
fn generated_rules_past_the_golden() {
    let select = |tag: &str, o: &Operands| (plural::rule(plural::rule_index(tag).unwrap()).0)(o);
    for (n, want) in [
        (22, Category::Two),
        (2000, Category::Two),
        (21_000, Category::Other),
        (30_000, Category::Other),
        (40_000, Category::Two),
        (60_000, Category::Two),
        (80_000, Category::Two),
        (120_000, Category::Two),
        (1_100_000, Category::Two),
    ] {
        assert_eq!(select("kw", &Operands::int(n)), want, "kw {n}");
    }
    for (s, want) in [
        ("1", Category::One),
        ("10", Category::Zero),
        ("0.21", Category::One),
        ("0.11", Category::Zero),
        ("1.31", Category::One),
        ("10.01", Category::One),
    ] {
        assert_eq!(select("lv", &Operands::parse(s)), want, "lv {s}");
    }
}
