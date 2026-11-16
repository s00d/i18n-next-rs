# i18n-next-rs

i18n for Rust and Tauri on the same locale files as
[nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro): one catalog (global and
per-page translations) for a Rust backend, native menus and tray, a Vue WebView and
Rust frontends in wasm.

Documentation: **https://s00d.github.io/i18n-next-rs/**

**Status: pre-release (0.1).** The catalog (root and page chunks, fallback chains,
`i18n.json`), plurals, linked messages, `Text`, `t!` and `scope`, number and date formats
(CLDR 48, matched line by line with `Intl`), lazy and remote loading, the Tauri plugin, the
npm package and the Rust guest for Tauri work. Nothing is published yet.

```rust
I18n::builder().dir("locales").locale("ru").fallback("en").install()?;
println!("{}", t!("greeting", name = "Ann"));
```

| Package | What |
| --- | --- |
| [`i18n-next`](crates/i18n-next) | the core crate: catalog, pages, plurals, `Intl`-compatible formats; no runtime dependencies, native and wasm |
| [`tauri-plugin-i18n-next`](crates/tauri-plugin-i18n-next) | Tauri 2 plugin: hands the catalog to the WebView and native UI |
| `tauri-plugin-i18n-next-api` | npm package (guest-js) for the WebView side: `connect` for any `@i18n-micro` adapter, `/vue` with a router guard |

Playgrounds, all on the shared catalog in [`examples/locales`](examples/locales):

- `cargo run -p playground-rust` — a console tour (`--check` fails on any missing key);
- [`examples/tauri-app`](examples/tauri-app) — Vue + vue-router, native menu and tray, two windows (`pnpm playground:tauri`);
- [`examples/leptos-app`](examples/leptos-app) — Leptos in the browser (`trunk serve`) and inside Tauri (`cargo tauri dev`);
- [`examples/nuxt-site`](examples/nuxt-site) — nuxt-i18n-micro on the same files.

Design and work plan: [DESIGN.md](DESIGN.md), [PLAN.md](PLAN.md) (Russian). Changes: [CHANGELOG.md](CHANGELOG.md).

## Development

```sh
cargo nextest run --workspace        # tests (profiles in .config/nextest.toml)
cargo test --doc --workspace         # doctests
cargo bench -p i18n-next             # criterion benches
cargo run -p playground-rust -- --check
tools/size-check.sh                  # .wasm size of the micro, full and guest builds
pnpm install && pnpm typecheck && pnpm test && pnpm build   # guest-js
pnpm -C scripts compat               # regenerate the compat goldens (JS writes them)
pnpm docs:generate                   # the docs site
```

The whole gate and the test rules: [Contributing](https://s00d.github.io/i18n-next-rs/contributing).

MSRV: Rust 1.90. License: MIT.
