# Changelog

All notable changes to `i18n-next`, `tauri-plugin-i18n-next` and `tauri-plugin-i18n-next-api`
are documented here. The three packages are released together with one version.

## Unreleased

- `t()` returns a plain `&str` and `get()` an `Option<&str>` (on `I18n` and `View`): catalog
  texts are stored once for the life of the process (equal texts share one copy), so a
  lookup neither allocates nor touches a reference count, and the text stays valid through
  `set_locale`, `reload` and page eviction. The cost: texts are not freed until the process
  exits; a reload adds only texts not seen before. `try_t` is gone: `get` is the same lookup
  (`None` on a miss, `missing` not called).

- Project skeleton: workspace, both crates, CI (native matrix, MSRV 1.90, wasm, size,
  dependency, API, perf jobs), test and bench scaffolding, shared example locales.
