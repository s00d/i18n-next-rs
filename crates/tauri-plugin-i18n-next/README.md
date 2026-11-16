[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-i18n-next?style=for-the-badge)](https://crates.io/crates/tauri-plugin-i18n-next)
[![docs.rs](https://img.shields.io/docsrs/tauri-plugin-i18n-next?style=for-the-badge)](https://docs.rs/tauri-plugin-i18n-next)
[![Documentation](https://img.shields.io/badge/docs-site-blue?style=for-the-badge)](https://s00d.github.io/i18n-next-rs/)

# tauri-plugin-i18n-next

Tauri 2 plugin for [i18n-next](https://crates.io/crates/i18n-next): one locale catalog
for the Rust backend, native menus and tray, and the WebView (through the
`tauri-plugin-i18n-next-api` npm package).

**Documentation: <https://s00d.github.io/i18n-next-rs/>**

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
- Data goes to the WebView over the `i18n-next://` protocol (`/{tag}/{locale}/{page}.json`,
  `/{tag}/bundle?locale=…&page=…`: content-tagged, cacheable URLs; the `meta` command lists
  the tags and the fallback chains); commands `meta`, `load`, `load_page`, `set_locale`,
  `get_locale`, `available_locales`, `pages` (and `report_missing` in debug builds); event
  `i18n-next://changed { locale, version }`, in version order. Pages from other origins
  (`tauri-plugin-localhost`, an external URL) need `Builder::origins`.
- The start locale: the saved one (`i18n-next.json` in the app config dir), else the
  WebView's `navigator.languages`, else `defaultLocale`. Dates show in the catalog's
  `time_zone`, else the WebView's zone (sent with the first `meta`), and the front end
  formats them in the same zone.
- An app with a Content Security Policy must allow the protocol:
  `"connect-src": "ipc: http://ipc.localhost i18n-next: http://i18n-next.localhost"` (plus its own
  sources) in `app.security.csp`; without it guest-js falls back to the raw `load` command.
- Debug builds reload on file changes (`dir` / `lazy_dir`) and write
  `i18n-missing.json` to the app data dir. Front-end misses arrive by themselves; Rust
  misses only when the catalog is built with
  `.missing(tauri_plugin_i18n_next::missing)` (the handler is set on the catalog, before
  the plugin exists).

Permissions: add `"i18n-next:default"` to your capability (and
`"i18n-next:allow-report-missing"` for the dev report).

Front end:

```ts
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'
app.use(await createTauriI18n({ router }))   // or connect(i18n) for React, Preact, Solid
```

Guide (protocol, IPC, `bind`, persist, CSP, origins) and the JS API:
<https://s00d.github.io/i18n-next-rs/guides/tauri>.

License: MIT.
