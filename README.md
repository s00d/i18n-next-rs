[![Crates.io](https://img.shields.io/crates/v/i18n-next?style=for-the-badge)](https://crates.io/crates/i18n-next)
[![npm version](https://img.shields.io/npm/v/tauri-plugin-i18n-next-api/latest?style=for-the-badge)](https://www.npmjs.com/package/tauri-plugin-i18n-next-api)
[![Documentation](https://img.shields.io/badge/docs-site-blue?style=for-the-badge)](https://s00d.github.io/i18n-next-rs/)
[![CI](https://img.shields.io/github/actions/workflow/status/s00d/i18n-next-rs/ci.yml?branch=main&style=for-the-badge&label=CI)](https://github.com/s00d/i18n-next-rs/actions/workflows/ci.yml)
[![GitHub issues](https://img.shields.io/github/issues/s00d/i18n-next-rs?style=for-the-badge)](https://github.com/s00d/i18n-next-rs/issues)
[![GitHub stars](https://img.shields.io/github/stars/s00d/i18n-next-rs?style=for-the-badge)](https://github.com/s00d/i18n-next-rs/stargazers)
[![Donate](https://img.shields.io/badge/Donate-Donationalerts-ff4081?style=for-the-badge)](https://www.donationalerts.com/r/s00d88)

# i18n-next

i18n for Rust and Tauri 2 on the same locale files as
[nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro): one catalog of global and per-page
translations for a Rust backend, native menus and tray, a Vue WebView and Rust frontends in wasm.

**Documentation: https://s00d.github.io/i18n-next-rs/**

- **Same files as the web:** `locales/{locale}.json`, `locales/pages/{page}/{locale}.json` and
  `i18n.json` of nuxt-i18n-micro, fallback chains, linked messages (`@:key`), `<i18n-t>` as `Text`.
- **`Intl`-compatible formats:** plurals by CLDR rules, numbers, currencies, dates and relative time
  as `Intl` prints them (own CLDR 48 data and time zones, matched line by line with `Intl`).
- **Small core:** no runtime dependencies, native and wasm; `default-features = false` is a ~120 KB
  micro build, `I18N_NEXT_LOCALES=en,de,ru` keeps only the CLDR data you ship.
- **Tauri plugin:** the catalog reaches the WebView over a cacheable `i18n-next://` protocol;
  `setLocale` switches Rust, native labels and every window, the saved locale survives restarts.
- **Frontends:** `tauri-plugin-i18n-next-api` for any `@i18n-micro` adapter (Vue with a router guard,
  React, Preact, Solid), and a Rust guest (`tauri-guest`) for Leptos and other wasm apps.

| Platform | Supported |
|----------|-----------|
| Windows · macOS · Linux | ✅ |
| Android · iOS (Tauri) | ✅ (locales embedded with `files()`) |
| Browser (wasm32) | ✅ |
| Server (axum, any async runtime) | ✅ (`scope()` per request) |

**Status: pre-release (0.1), not published yet.** Until the first release, use the git dependency:
`i18n-next = { git = "https://github.com/s00d/i18n-next-rs" }`.

## Quick start

```rust
use i18n_next::{I18n, t};

I18n::builder().dir("locales").locale("ru").fallback("en").install()?;
println!("{}", t!("greeting", name = "Ann"));
println!("{}", t!("title", @page = "settings"));
i18n_next::set_locale("en")?;
```

Tauri:

```rust
// src-tauri/src/lib.rs
tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())   // t! works in Rust too
    .run(tauri::generate_context!())?;
```

```ts
// src/main.ts
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'

app.use(await createTauriI18n({ router }))
```

Full setup (capability, embedded locales for bundles): [Quick start](https://s00d.github.io/i18n-next-rs/getting-started/quick-start).

## Packages

| Package | Purpose | Docs |
|---|---|---|
| [`i18n-next`](crates/i18n-next) | Core: catalog, pages, plurals, `Intl`-compatible formats | [guide](https://s00d.github.io/i18n-next-rs/getting-started/introduction) · [api](https://s00d.github.io/i18n-next-rs/reference/rust-api) |
| [`tauri-plugin-i18n-next`](crates/tauri-plugin-i18n-next) | Tauri 2 plugin: catalog for the WebView and native UI | [guide](https://s00d.github.io/i18n-next-rs/guides/tauri) |
| `tauri-plugin-i18n-next-api` | npm package for the WebView (`connect`, `/vue`, React, Preact, Solid) | [guide](https://s00d.github.io/i18n-next-rs/guides/tauri) · [api](https://s00d.github.io/i18n-next-rs/reference/javascript-api) |

## Examples

All on the shared catalog in [`examples/locales`](examples/locales):

- [`examples/rust`](examples/rust): console tour and an axum server (`cargo run -p playground-rust -- --check`);
- [`examples/tauri-app`](examples/tauri-app): Vue + vue-router, native menu and tray, two windows, Android;
- [`examples/leptos-app`](examples/leptos-app): Leptos in the browser and inside Tauri;
- [`examples/nuxt-site`](examples/nuxt-site): nuxt-i18n-micro on the same files.

## Contributing

Issues and pull requests are welcome. Build, test and benchmark commands and the CI gate:
[CONTRIBUTING.md](CONTRIBUTING.md). Design notes: [DESIGN.md](DESIGN.md).
Changes: [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE). MSRV: Rust 1.90.
