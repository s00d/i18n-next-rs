# i18n-next

i18n for Rust on the same locale files as [nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro):
global and per-page translations, plurals and `Intl`-compatible formats, native and wasm,
with no runtime dependencies.

**Status: early.** The catalog (root and page chunks, fallback chains, `i18n.json`),
plurals (CLDR rules as in `Intl.PluralRules`), `Text`, `t!` and global access work; formats
and the Tauri plugin come stage by stage. Do not depend on it yet.

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
of a request or a task.

License: MIT.
