//! A BCP 47 tag → the CLDR data locale `Intl.NumberFormat` formats with, as
//! `@i18n-micro/core` `FormatService` hands it over (`intlTag`): underscores become hyphens, a
//! tag `Intl` rejects or does not support formats as `en`.
//!
//! Then as ICU picks its data: language aliases that lead elsewhere (`iw` → `he`, `sh` →
//! `sr-Latn`, `hy-arevmda` → `hyw`), region aliases (`620` → `PT`), the script a region implies
//! where ICU aliases it (`zh-TW` → `zh-Hant-TW`, `sr-ME` → `sr-Latn-ME`; `ku-IQ` → the root,
//! ICU lacking `ku-Arab`), then the longest selected locale of
//! `lang-Script-REGION`, `lang-Script`, `lang-REGION`, `lang`. A language with no selected
//! locale (`I18N_NEXT_LOCALES` left it out) formats as `en` too. `-u-nu-` and other
//! extensions are not read (known difference: `ar-EG-u-nu-latn` keeps Arabic digits).
//! `scripts/cldr.mjs` checks this against `Intl` on every locale and derived tag.

use super::data::{self, LOCALE_KEYS, LOCALES, Loc};
use crate::plural::{Buf, subtags};
use std::borrow::Cow;

/// Index into `LOCALES` of an exact data locale name (`und` excluded: never asked for).
fn index(name: &str) -> Option<usize> {
    let i = LOCALE_KEYS.binary_search(&data::key(name)?).ok()?;
    (i != data::UND).then_some(i)
}

/// The row of `key` in a table (`keys` sorted, `rows` beside them).
fn row<T: Copy>(keys: &[u128], rows: &[T], key: &str) -> Option<T> {
    let i = keys.binary_search(&data::key(key)?).ok()?;
    rows.get(i).copied()
}

/// The replacement of a language alias: `lang-variant` (`hy-arevmda` → `hyw`) for the first
/// variant that has one, else the language (`iw` → `he`), unless the tag is that language
/// bare and lowercase and the alias does not apply there (`bh`: V8 keeps it).
fn alias(lang: &str, variants: Option<&str>, tag: &str) -> Option<&'static str> {
    for v in variants.into_iter().flat_map(|v| v.split('-')) {
        let mut key = Buf::<17>::of(lang, |b| b);
        key.push("-", |b| b);
        key.push(v, |b| b.to_ascii_lowercase());
        if let Some((to, _)) = row(&data::ALIASES_KEYS, &data::ALIASES, key.as_str()) {
            return Some(data::s(to));
        }
    }
    let (to, lone) = row(&data::ALIASES_KEYS, &data::ALIASES, lang)?;
    (lone || tag != lang).then(|| data::s(to))
}

/// The data locale of `tag` (see the module docs); `en` when there is none.
pub(crate) fn resolve(tag: &str) -> &'static Loc {
    let i = find(tag).unwrap_or(data::EN);
    LOCALES.get(i).unwrap_or(&LOCALES[data::EN])
}

/// The data locale of `tag`, `None` when `Intl` would not support it here.
pub(crate) fn find(tag: &str) -> Option<usize> {
    let tag: Cow<'_, str> = if tag.contains('_') {
        Cow::Owned(tag.replace('_', "-"))
    } else {
        Cow::Borrowed(tag)
    };
    let t = subtags(&tag)?;
    let mut lang = t.language;
    let mut script = t.script.map(Buf::<4>::title);
    let mut region = t
        .region
        .map(|r| Buf::<3>::of(r, |b| b.to_ascii_uppercase()));
    if let Some(to) = alias(lang.as_str(), t.variants, &tag) {
        let mut parts = to.split('-');
        lang = Buf::of(parts.next().unwrap_or(""), |b| b);
        for p in parts {
            if p.len() == 4 && script.is_none() {
                script = Some(Buf::of(p, |b| b));
            } else if region.is_none() {
                region = Some(Buf::of(p, |b| b));
            }
        }
    }
    if let Some(r) = region
        && let Some(to) = row(&data::REGIONS_KEYS, &data::REGIONS, r.as_str())
    {
        region = Some(Buf::of(data::s(to), |b| b));
    }
    let mut cand = Buf::<17>::of(lang.as_str(), |b| b);
    let base = cand.len();
    if script.is_none()
        && let Some(r) = &region
    {
        cand.push("-", |b| b);
        cand.push(r.as_str(), |b| b);
        script = row(&data::SCRIPTS_KEYS, &data::SCRIPTS, cand.as_str())
            .map(|s| Buf::of(data::s(s), |b| b));
        cand.truncate(base);
        // ICU aliases the tag onto a locale it lacks, and formats with the root (`ku-IQ`)
        if script.as_ref().is_some_and(|s| s.as_str() == "-") {
            return Some(data::UND);
        }
    }
    if let Some(s) = &script {
        cand.push("-", |b| b);
        cand.push(s.as_str(), |b| b);
        let with_script = cand.len();
        if let Some(r) = &region {
            cand.push("-", |b| b);
            cand.push(r.as_str(), |b| b);
            if let Some(i) = index(cand.as_str()) {
                return Some(i);
            }
            cand.truncate(with_script);
        }
        if let Some(i) = index(cand.as_str()) {
            return Some(i);
        }
        cand.truncate(base);
    }
    if let Some(r) = &region {
        cand.push("-", |b| b);
        cand.push(r.as_str(), |b| b);
        if let Some(i) = index(cand.as_str()) {
            return Some(i);
        }
        cand.truncate(base);
    }
    index(cand.as_str())
}

/// The locale's own name (8b.14) of the data locale of `tag`; `None` for a tag with no
/// selected locale or a locale ICU has no name for.
pub(crate) fn autonym(tag: &str) -> Option<&'static str> {
    let loc = LOCALES.get(find(tag)?)?;
    let name = data::s(loc.autonym);
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name of data locale `i`, from its key.
    fn name(i: usize) -> String {
        let bytes = LOCALE_KEYS[i].to_be_bytes();
        String::from_utf8(bytes.iter().copied().take_while(|&b| b != 0).collect()).unwrap()
    }

    /// Given every data locale of this build, when its name is looked up, then the key search
    /// finds it (the keys sort like the names), `und` aside.
    #[test]
    fn keys_find_every_locale() {
        assert!(LOCALE_KEYS.windows(2).all(|w| w[0] < w[1]));
        for i in 0..LOCALES.len() {
            let want = (i != data::UND).then_some(i);
            assert_eq!(index(&name(i)), want, "{}", name(i));
        }
        assert_eq!(name(data::UND), "und");
        assert_eq!(index("a-name-over-sixteen-bytes"), None);
    }

    /// Given every data locale, when `scripts/cldr.mjs` resolved its plural rule set once, then
    /// it is the one `Intl.PluralRules` gives the name (`plural::rule_index`), as compact
    /// patterns and currency names used to resolve it on every call.
    #[test]
    fn plural_rule_of_each_locale() {
        for (i, loc) in LOCALES.iter().enumerate() {
            let want = crate::plural::rule_index(&name(i)).map_or(u8::MAX, |r| r as u8);
            assert_eq!(loc.plural, want, "{}", name(i));
        }
    }
}
