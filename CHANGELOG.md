# Changelog

All notable changes to `i18n-next`, `tauri-plugin-i18n-next` and `tauri-plugin-i18n-next-api`
are documented here. The three packages are released together with one version.

## Unreleased

- `t(key, f)` and `get(key, f)` (on `I18n` and `View`) lend the text to `f` as a `&str`
  borrowed from the catalog instead of returning a `String` / `Arc<str>`: no allocation, no
  reference count. `t(key, str::to_owned)` keeps a copy. `try_t` is gone: `get` is the same
  lookup (`None` on a miss, `missing` not called).

- Project skeleton: workspace, both crates, CI (native matrix, MSRV 1.90, wasm, size,
  dependency, API, perf jobs), test and bench scaffolding, shared example locales.
