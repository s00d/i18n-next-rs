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

/// Given the tags Intl resolved (or rejected), when Rust resolves the same tag, then it gets
/// the rule of the locale Intl resolved to, and no rule where Intl threw or knew nothing.
#[test]
fn tags_resolve_like_supported_locales_of() {
    let mut tags = 0;
    for (name, tag, expected) in lines().filter(|(n, _, _)| *n == "tag") {
        let _ = name;
        tags += 1;
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
