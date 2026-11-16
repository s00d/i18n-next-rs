# tauri-plugin-i18n-next

Tauri 2 plugin for [i18n-next](https://crates.io/crates/i18n-next): one locale catalog
for the Rust backend, native menus and tray, and the WebView (through the
`tauri-plugin-i18n-next-api` npm package).

```rust,no_run
use tauri_plugin_i18n_next::ManagerExt;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let i18n = i18n_next::I18n::builder()
    .config("i18n.json")
    .dir("locales")
    .missing(tauri_plugin_i18n_next::missing) // dev report of missing keys
    .build()?;
tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())
    .setup(|app| {
        let quit = tauri::menu::MenuItem::new(app, "Quit", true, None::<&str>)?;
        let i18n = app.i18n();
        i18n.bind(&quit, i18n.page("tray").text("quit")); // relabeled on every switch
        Ok(())
    })
#   ;
# Ok(()) }
```

- `t!` works everywhere (the plugin installs the catalog globally); `app.i18n()` derefs
  to `I18n`, `app.state::<I18n>()` works too.
- Data goes to the WebView over the `i18n-next://` protocol (`/v{version}/{locale}/{page}.json`,
  `/v{version}/bundle?locale=…&page=…`, cacheable, versioned URLs); commands `load`,
  `load_page`, `set_locale`, `get_locale`, `available_locales`, `pages` (and
  `report_missing` in debug builds); event `i18n-next://changed { locale, version }`.
- The start locale: the saved one (`i18n-next.json` in the app config dir), else the
  WebView's `navigator.languages`, else `defaultLocale`.
- Debug builds reload on file changes (`dir` / `lazy_dir`) and write
  `i18n-missing.json` to the app data dir.

Permissions: add `"i18n-next:default"` to your capability (and
`"i18n-next:allow-report-missing"` for the dev report).

License: MIT.
