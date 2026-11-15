//! `I18N_NEXT_LOCALES` (DESIGN 2.4): `build.rs` matches each tag of the list as the run time
//! resolves it (`format/locale.rs`), aliases included: a tag that formats at run time is
//! accepted at build time. `build.rs` is read in as a module, on the data it reads.

#[allow(dead_code)]
#[path = "../build.rs"]
mod build;

/// Given the tags of a locale list, when `build.rs` picks their data locales, then language
/// aliases (`iw`, `sh`, `mo`, `tl`, `BH`; not the bare `bh`), variant aliases (`hy-arevmda`:
/// no data), region aliases and ICU's script aliases (`ku-IQ`: the root) apply as at run time.
#[test]
fn locale_list_tags_resolve_like_the_run_time() {
    let f = build::parse(include_str!("../src/format/data/numbers.txt"), None);
    for (tag, want) in [
        ("iw", Some("he")),
        ("iw-IL", Some("he")),
        ("sh", Some("sr-Latn")),
        ("mo", Some("ro")),
        ("tl", Some("fil")),
        ("in", Some("id")),
        ("BH", Some("bho")),
        ("bh-US", Some("bho")),
        ("bh", None),
        ("hy-arevmda", None),
        ("ku-IQ", Some("und")),
        ("zh-TW", Some("zh-Hant")),
        ("zh-HK", Some("zh-Hant-HK")),
        ("sr-ME", Some("sr-Latn-ME")),
        ("pt-620", Some("pt-PT")),
        ("de-XX", Some("de")),
        ("de_CH", Some("de-CH")),
        ("xx", None),
    ] {
        assert_eq!(build::find(&f, tag).as_deref(), want, "{tag}");
    }
}
