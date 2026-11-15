//! A BCP 47 tag → the CLDR data locale `Intl.NumberFormat` formats with, as
//! `@i18n-micro/core` `FormatService` hands it over (`intlTag`): underscores become hyphens, a
//! tag `Intl` rejects or does not support formats as `en`.
//!
//! Then as ICU picks its data: language aliases that lead elsewhere (`iw` → `he`, `sh` →
//! `sr-Latn`), region aliases (`620` → `PT`), the script a region implies (`zh-TW` →
//! `zh-Hant-TW`, `sr-ME` → `sr-Latn-ME`), then the longest selected locale of
//! `lang-Script-REGION`, `lang-Script`, `lang-REGION`, `lang`. A language with no selected
//! locale (`I18N_NEXT_LOCALES` left it out) formats as `en` too. `-u-nu-` and other
//! extensions are not read (known difference: `ar-EG-u-nu-latn` keeps Arabic digits).
//! `scripts/cldr.mjs` checks this against `Intl` on every locale and derived tag.

use super::data::{self, LOCALES, Loc};
use crate::plural::{Buf, subtags};
use std::borrow::Cow;

/// Index into `LOCALES` of an exact data locale name (`und` excluded: never asked for).
fn index(name: &str) -> Option<usize> {
    let i = LOCALES
        .binary_search_by(|l| data::s(l.name).as_bytes().cmp(name.as_bytes()))
        .ok()?;
    (name != "und").then_some(i)
}

/// The replacement of `key` in a sorted pair table of interned strings.
fn lookup(table: &[(u16, u16)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by(|&(k, _)| data::s(k).as_bytes().cmp(key.as_bytes()))
        .ok()
        .and_then(|i| table.get(i))
        .map(|&(_, v)| data::s(v))
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
    let (language, script, region) = subtags(&tag)?;
    let mut lang = language;
    let mut script = script.map(Buf::<4>::title);
    let mut region = region.map(|r| Buf::<3>::of(r, |b| b.to_ascii_uppercase()));
    if let Some(to) = lookup(&data::ALIASES, lang.as_str()) {
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
        && let Some(to) = lookup(&data::REGIONS, r.as_str())
    {
        region = Some(Buf::of(to, |b| b));
    }
    let mut cand = Buf::<17>::of(lang.as_str(), |b| b);
    let base = cand.len();
    if script.is_none()
        && let Some(r) = &region
    {
        cand.push("-", |b| b);
        cand.push(r.as_str(), |b| b);
        script = lookup(&data::SCRIPTS, cand.as_str()).map(|s| Buf::of(s, |b| b));
        cand.truncate(base);
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
