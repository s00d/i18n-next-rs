//! Names from the frontend (DESIGN 8b.2) into `set_locale` / `page` / `with_locale` /
//! `add_messages`: no panic, an unknown locale is refused without changing anything, and no
//! name becomes a path. The catalog comes from `files()`, the process runs in an empty
//! directory, and the directory is still empty after every input: a name like
//! `../../etc/passwd`, `en.json` or `pages/x` never reaches the file system.
#![no_main]

use i18n_next::I18n;
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use std::sync::OnceLock;

fn sandbox() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("i18n-next-fuzz-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).unwrap();
        dir
    })
}

fuzz_target!(|data: &[u8]| {
    let dir = sandbox();
    let text = String::from_utf8_lossy(data);
    let names: Vec<&str> = text.split('\0').take(8).collect();
    let i18n = I18n::builder()
        .locale("en")
        .fallback("en")
        .files([
            ("en.json", br#"{"a": "A", "n": {"x": "X"}}"#.as_slice()),
            ("de.json", br#"{"a": "Ae"}"#),
            ("pages/about/en.json", br#"{"a": "About"}"#),
        ])
        .build()
        .unwrap();
    for (i, name) in names.iter().enumerate() {
        let before = i18n.locale();
        let known = i18n.locales().iter().any(|l| l.code == *name);
        match i18n.set_locale(name) {
            Ok(()) => assert!(known, "set_locale accepted unknown {name:?}"),
            Err(_) => {
                assert!(!known, "set_locale refused known {name:?}");
                assert_eq!(i18n.locale(), before);
            }
        }
        let _ = i18n.page(name).t("a");
        let _ = i18n.with_locale(name).page(name).tc("a", 1);
        let _ = i18n.with_locale(name).has("n.x");
        let other = names.get(i + 1).copied();
        if i18n.add_messages(name, other, r#"{"added": "yes"}"#).is_ok() {
            assert_eq!(i18n.with_locale(name).page(other.unwrap_or("")).t("added"), "yes");
        }
        let _ = i18n.pages();
    }
    assert!(std::fs::read_dir(dir).unwrap().next().is_none(), "a name reached the file system");
});
