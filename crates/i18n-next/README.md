# i18n-next

i18n for Rust on the same locale files as [nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro):
global and per-page translations, plurals and `Intl`-compatible formats, native and wasm,
with no runtime dependencies.

**Status: early.** The catalog (root and page chunks, fallback chains, `i18n.json`) and
plurals (`tc`, CLDR rules as in `Intl.PluralRules`) work; formats, `t!` and the Tauri plugin
come stage by stage. Do not depend on it yet.

```rust
use i18n_next::{Args, I18n};

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
assert_eq!(i18n.t_with("hi", &Args::new().str("name", "Аня")), "Привет, Аня!");
assert_eq!(i18n.t("nav.home"), "Home");                        // fallback
assert_eq!(i18n.page("settings").t("title"), "Настройки");
assert_eq!(i18n.tc("apples", 21), "21 яблоко");                // CLDR plural rules
# Ok::<(), i18n_next::Error>(())
```

License: MIT.
