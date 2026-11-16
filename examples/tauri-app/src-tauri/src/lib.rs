//! Playground of tauri-plugin-i18n-next (DESIGN 8a), M7 skeleton: the shared catalog
//! (`examples/locales`, `examples/i18n.json`) embedded, a native menu and tray whose labels
//! follow the locale (`bind`), two windows (titles bound, both get `changed`), and a command
//! that returns a `Text`. The front end is a static page for now; M8 brings guest-js.

use i18n_next::{I18n, Text};
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_i18n_next::ManagerExt;

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

/// A command that returns a `Text`: the front end gets it translated in the active locale.
#[tauri::command]
fn greeting(app: tauri::AppHandle, name: String) -> Text {
    app.i18n().text("greeting").arg("name", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let i18n = catalog().expect("the example catalog builds");
    tauri::Builder::default()
        .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())
        .invoke_handler(tauri::generate_handler![greeting])
        .setup(|app| {
            let i18n = app.i18n();
            let tray = i18n.page("tray");

            // Language submenu: one item per locale, named in its own language
            let mut items = Vec::new();
            for l in i18n.locales() {
                let name = l.display_name.unwrap_or_else(|| l.code.clone());
                items.push(MenuItem::with_id(
                    app,
                    format!("locale:{}", l.code),
                    name,
                    true,
                    None::<&str>,
                )?);
            }
            let refs: Vec<&dyn tauri::menu::IsMenuItem<_>> = items
                .iter()
                .map(|i| i as &dyn tauri::menu::IsMenuItem<_>)
                .collect();
            let language = Submenu::with_items(app, "Language", true, &refs)?;
            let second = MenuItem::with_id(app, "second", "Second window", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            i18n.bind(&language, tray.text("language"));
            i18n.bind(&second, tray.text("second"));
            i18n.bind(&quit, tray.text("quit"));
            let menu = Menu::with_items(app, &[&language, &second, &quit])?;
            app.set_menu(menu.clone())?;

            let mut icon = TrayIconBuilder::with_id("main").menu(&menu);
            if let Some(image) = app.default_window_icon() {
                icon = icon.icon(image.clone());
            }
            let icon = icon.build(app)?;
            i18n.bind(&icon, tray.text("tooltip"));

            let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .inner_size(720.0, 520.0)
                .build()?;
            i18n.bind(&main, tray.text("title"));
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if let Some(code) = id.strip_prefix("locale:") {
                // the switch reaches Rust, both windows (`changed`) and every bound label
                let _ = app.i18n().set_locale(code);
            } else if id == "second" {
                if let Some(w) = app.get_webview_window("second") {
                    let _ = w.set_focus();
                } else if let Ok(w) =
                    WebviewWindowBuilder::new(app, "second", WebviewUrl::default())
                        .inner_size(480.0, 360.0)
                        .build()
                {
                    let i18n = app.i18n();
                    i18n.bind(&w, i18n.page("tray").text("second"));
                }
            } else if id == "quit" {
                app.exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the i18n-next demo");
}
