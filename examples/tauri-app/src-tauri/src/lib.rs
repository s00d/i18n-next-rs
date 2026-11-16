//! Playground of tauri-plugin-i18n-next (DESIGN 8a): the shared catalog (`examples/locales`,
//! `examples/i18n.json`) embedded, a native menu and tray whose labels follow the locale
//! (`bind`), two windows (titles bound, both get `changed`), and a command that returns a
//! `Text`. The front end (`../src`, Vue + vue-router + guest-js) loads each route's page
//! through the router guard and lists missed keys in its inspector, with a transport bench
//! (protocol against invoke, DESIGN 6.1). The menu, tray and second window are desktop only;
//! on Android (`pnpm tauri android build`) the same app runs in one window.

use i18n_next::{I18n, Text};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex};
use tauri::{WebviewUrl, WebviewWindowBuilder};
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

/// A command that returns a `Text`: the front end gets it translated in the active locale.
#[tauri::command]
fn greeting(app: tauri::AppHandle, name: String) -> Text {
    app.i18n().text("greeting").arg("name", name)
}

/// The sizes of the transport bench (DESIGN 6.1: 1k, 10k, 100k keys, ~80 bytes each).
const BENCH_KEYS: [u32; 3] = [1_000, 10_000, 100_000];

/// The bench pages made so far, as `serde_json::Value` for the `Value` row.
#[derive(Default)]
struct Bench(Mutex<HashMap<u32, Arc<serde_json::Value>>>);

/// A shared `Value`, serialized without a copy.
struct Shared(Arc<serde_json::Value>);

impl serde::Serialize for Shared {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

/// The transport bench page of `keys` keys (`bench-1k`…), added to the `en` catalog once:
/// the inspector then reads it through the plugin's own paths (protocol chunk, raw
/// `load_page`). Returns its name.
#[tauri::command]
fn bench_page(
    app: tauri::AppHandle,
    bench: tauri::State<'_, Bench>,
    keys: u32,
) -> Result<String, String> {
    if !BENCH_KEYS.contains(&keys) {
        return Err(format!("keys: one of {BENCH_KEYS:?}"));
    }
    let page = format!("bench-{}k", keys / 1000);
    let mut made = bench.0.lock().map_err(|e| e.to_string())?;
    if let Entry::Vacant(slot) = made.entry(keys) {
        let mut json = String::from("{");
        for i in 0..keys {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "\"k{i:06}\":\"Transport bench value {i:06}: the quick brown fox jumps over\""
            ));
        }
        json.push('}');
        app.i18n()
            .add_messages("en", Some(&page), &json)
            .map_err(|e| e.to_string())?;
        let value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        slot.insert(Arc::new(value));
    }
    Ok(page)
}

/// The bench page as a `serde_json::Value` (the slow row of DESIGN 6.1).
#[tauri::command]
fn bench_value(bench: tauri::State<'_, Bench>, keys: u32) -> Result<Shared, String> {
    let made = bench.0.lock().map_err(|e| e.to_string())?;
    made.get(&keys)
        .map(|v| Shared(v.clone()))
        .ok_or_else(|| "bench_page first".to_owned())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let i18n = catalog().expect("the example catalog builds");
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())
        .manage(Bench::default())
        .invoke_handler(tauri::generate_handler![greeting, bench_page, bench_value])
        .setup(|app| {
            #[cfg(desktop)]
            desktop::menu_and_tray(app)?;
            let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .inner_size(720.0, 520.0)
                .build()?;
            let i18n = app.i18n();
            i18n.bind(&main, i18n.page("tray").text("title"));
            Ok(())
        });
    #[cfg(desktop)]
    let builder = builder.on_menu_event(desktop::on_menu);
    builder
        .run(tauri::generate_context!())
        .expect("error while running the i18n-next demo");
}

/// The native menu and tray (desktop only), their labels bound to the catalog.
#[cfg(desktop)]
mod desktop {
    use tauri::menu::{Menu, MenuEvent, MenuItem, Submenu};
    use tauri::tray::TrayIconBuilder;
    use tauri::{App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
    use tauri_plugin_i18n_next::ManagerExt;

    pub fn menu_and_tray(app: &App) -> tauri::Result<()> {
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
        Ok(())
    }

    pub fn on_menu(app: &AppHandle, event: MenuEvent) {
        let id = event.id().as_ref();
        if let Some(code) = id.strip_prefix("locale:") {
            // the switch reaches Rust, both windows (`changed`) and every bound label
            let _ = app.i18n().set_locale(code);
        } else if id == "second" {
            if let Some(w) = app.get_webview_window("second") {
                let _ = w.set_focus();
            } else if let Ok(w) = WebviewWindowBuilder::new(app, "second", WebviewUrl::default())
                .inner_size(480.0, 360.0)
                .build()
            {
                // a new window, a new binding: the plugin drops the old window's
                // binding when that window is destroyed
                let i18n = app.i18n();
                i18n.bind(&w, i18n.page("tray").text("second"));
            }
        } else if id == "quit" {
            app.exit(0);
        }
    }
}
