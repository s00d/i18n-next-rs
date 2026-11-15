# tauri-plugin-i18n-next

Tauri 2 plugin for [i18n-next](https://crates.io/crates/i18n-next): one locale catalog
for the Rust backend, native menus and tray, and the WebView (through the
`tauri-plugin-i18n-next-api` npm package).

**Status: early.** Only the skeleton exists: the plugin registers as `i18n-next` and
exposes a placeholder `ping` command. Do not depend on it yet.

```rust,no_run
# fn main() {
tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::init())
#   ;
# }
```

Permissions: add `"i18n-next:default"` to your capability.

License: MIT.
