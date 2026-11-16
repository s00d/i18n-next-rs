# Changelog

All notable changes to `i18n-next`, `tauri-plugin-i18n-next` and `tauri-plugin-i18n-next-api`
are documented here. The three packages are released together with one version.

## Unreleased

- `t()` (on `I18n` and `View`) returns `Translation<'k>` instead of `String`: the stored
  text shared or the key borrowed, no allocation. It reads as `&str` (`Deref`, `Display`,
  comparisons); `String::from(..)` / `.into()` makes an owned copy.

- Project skeleton: workspace, both crates, CI (native matrix, MSRV 1.90, wasm, size,
  dependency, API, perf jobs), test and bench scaffolding, shared example locales.
