# i18n-next-rs

i18n for Rust and Tauri on the same locale files as
[nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro): one catalog (global and
per-page translations) for a Rust backend, native menus and tray, a Vue WebView and
Rust frontends in wasm.

**Status: early.** The repository holds the project skeleton; the API is being built
stage by stage. Nothing is published yet.

| Package | What |
| --- | --- |
| [`i18n-next`](crates/i18n-next) | the core crate: catalog, pages, plurals, `Intl`-compatible formats; no runtime dependencies, native and wasm |
| [`tauri-plugin-i18n-next`](crates/tauri-plugin-i18n-next) | Tauri 2 plugin: hands the catalog to the WebView and native UI |
| `tauri-plugin-i18n-next-api` | npm package (guest-js) for the WebView side |

- Design and work plan: [DESIGN.md](DESIGN.md), [PLAN.md](PLAN.md) (Russian).
- Playground: `cargo run -p playground-rust` (shared catalog in [`examples/locales`](examples/locales)).
- Changes: [CHANGELOG.md](CHANGELOG.md).

## Development

```sh
cargo nextest run --workspace        # tests (profiles in .config/nextest.toml)
cargo test --doc --workspace         # doctests
cargo bench -p i18n-next             # criterion benches
cargo run -p playground-rust -- --check
tools/size-check.sh                  # .wasm size of the micro and full builds
```

MSRV: Rust 1.90. License: MIT.
