//! The Tauri side of the Leptos playground (DESIGN 8a): the shared catalog
//! (`examples/locales`, `examples/i18n.json`) embedded and served by the plugin. The front end
//! is `../src`, the same Leptos app as in the browser: it finds `__TAURI_INTERNALS__` and
//! connects through `i18n_next::tauri_guest`.

use i18n_next::I18n;

/// The shared catalog, embedded: the same layout as `dir("locales")`, also on Android.
fn catalog() -> Result<I18n, i18n_next::Error> {
    macro_rules! file {
        ($path:literal) => {
            (
                $path,
                &include_bytes!(concat!("../../../locales/", $path))[..],
            )
        };
    }
    I18n::builder()
        .config_json(include_str!("../../../i18n.json"))
        .files([
            file!("en.json"),
            file!("ru.json"),
            file!("ar.json"),
            file!("pages/about/en.json"),
            file!("pages/about/ru.json"),
            file!("pages/about/ar.json"),
            file!("pages/settings/en.json"),
            file!("pages/settings/ru.json"),
            file!("pages/tray/en.json"),
            file!("pages/tray/ru.json"),
            file!("pages/tray/ar.json"),
        ])
        .linked_messages(true)
        .missing(tauri_plugin_i18n_next::missing)
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let i18n = catalog().expect("the example catalog builds");
    tauri::Builder::default()
        .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())
        .run(tauri::generate_context!())
        .expect("error while running the Leptos playground");
}
