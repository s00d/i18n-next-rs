[![Crates.io](https://img.shields.io/crates/v/i18n-next?style=for-the-badge)](https://crates.io/crates/i18n-next)
[![docs.rs](https://img.shields.io/docsrs/i18n-next?style=for-the-badge)](https://docs.rs/i18n-next)
[![Documentation](https://img.shields.io/badge/docs-site-blue?style=for-the-badge)](https://s00d.github.io/i18n-next-rs/)

# i18n-next

i18n for Rust on the same locale files as [nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro):
global and per-page translations, plurals and `Intl`-compatible formats, native and wasm,
with no runtime dependencies.

**Documentation: <https://s00d.github.io/i18n-next-rs/>**

**Status: pre-release (0.1).** Root and page chunks, fallback chains, `i18n.json`, plurals
(CLDR rules as in `Intl.PluralRules`), linked messages, `Text` (the Rust `<i18n-t>`), `t!` and
`scope`, numbers, dates and relative time as `Intl` shows them (own CLDR 48 data and time
zones), lazy and remote loading, and a Rust guest for the
[Tauri plugin](https://crates.io/crates/tauri-plugin-i18n-next). Not published yet.

```rust
use i18n_next::{I18n, t, tc};

let i18n = I18n::builder()
    .locale("ru")
    .fallback("en")
    .files([
        ("en.json", br#"{"nav": {"home": "Home"}, "hi": "Hi, {name}!"}"#.as_slice()),
        ("ru.json", r#"{"hi": "Привет, {name}!",
            "apples": "нет яблок | {count} яблоко | {count} яблока | {count} яблок"}"#.as_bytes()),
        ("pages/settings/ru.json", r#"{"title": "Настройки"}"#.as_bytes()),
    ])
    .build()?;
assert_eq!(t!(i18n => "hi", name = "Аня"), "Привет, Аня!");
assert_eq!(t!(i18n => "nav.home"), "Home");                    // fallback
assert_eq!(t!(i18n => "title", @page = "settings"), "Настройки");
assert_eq!(tc!(i18n => "apples", 21), "21 яблоко");            // CLDR plural rules

let hi = i18n.text("hi").arg("name", "Ann");                   // lazy: shown when displayed
i18n.set_locale("en")?;
assert_eq!(hi.to_string(), "Hi, Ann!");
# Ok::<(), i18n_next::Error>(())
```

Or `install` it once and write `t!("hi", name = "Ann")` anywhere; `scope()` sets the locale
of a request or a task. Default features: `std`, `format`, `tz`, `global`; `default-features =
false` is the micro build. `I18N_NEXT_LOCALES=en,de,ru` at build time keeps only the CLDR
data you ship.

Guides, platforms (desktop, Android, iOS, browser, Tauri guest) and the feature list:
<https://s00d.github.io/i18n-next-rs/>.

License: MIT.
