# Changelog

All notable changes to `i18n-next`, `tauri-plugin-i18n-next` and `tauri-plugin-i18n-next-api`
are documented here. The three packages are released together with one version.

## 0.1.1 (2026-11-17)

- `tauri-plugin-i18n-next-api` on npm has its own README (install, Vue, `connect`) instead of
  the Rust crate's.
- The crates ship only what a build needs: sources, `build.rs`, permissions, README, LICENSE
  and CHANGELOG, no tests or benchmarks.
- `tauri-plugin-i18n-next-api` exports the `Tree` type (what `I18nInstance` takes) and the
  event payload types `ChangedEvent` and `MissingEvent`.

## 0.1.0 (2026-11-17)

First release of `i18n-next`, `tauri-plugin-i18n-next` and `tauri-plugin-i18n-next-api`.

### i18n-next

- Catalog in the nuxt-i18n-micro format: JSON trees, root and `pages/` chunks, `i18n.json`,
  fallback chains (`de-at` → `de` → `en`), the `missing` handler.
- Lookups: `t()` returns a plain `&str` (the key on a miss), `get()` an `Option<&str>`, `has()`.
  Catalog texts are stored once for the life of the process, so a lookup allocates nothing.
- `t_with` parameters, plurals (`tc`, CLDR rules), linked messages (`@:key` and modifiers),
  `Text` for lazy and rich text.
- Numbers, dates and relative time (`tn`, `td`, `tdr`) on CLDR 48 data with IANA time zones;
  `I18N_NEXT_LOCALES` picks the locales built in.
- Loading from a folder, embedded files, lazy pages or the network (`remote` + `ensure_async`);
  `reload`, `subscribe`, `watch` in debug builds.
- The global instance with `t!` / `tc!` / `key!`, `scope()` per request or task.
- `check()` for missing and broken keys, `tracing` events.
- Desktop, Android, iOS, the browser (wasm) and a Rust frontend inside Tauri (`tauri-guest`).

### tauri-plugin-i18n-next

- Serves the catalog to the WebView over a custom protocol; one locale for Rust and every
  window; relabels native menus, windows and the tray; persists the chosen locale; reports
  missing keys from the frontend in debug builds.

### tauri-plugin-i18n-next-api

- `connect` for any `@i18n-micro` instance, the `/vue` entry with a router guard, React,
  Preact and Solid adapters.
