//! Playground of i18n-next in a Rust front end (DESIGN 8a): one Leptos CSR app in two modes.
//!
//! - **Browser** (`trunk serve`): the shared catalog (`examples/locales`, copied to
//!   `dist/locales`) through `remote` + `ensure_async` with the platform's `fetch`; the chosen
//!   locale is kept in `localStorage`, the first one comes from `navigator.languages`.
//! - **Tauri** (`cargo tauri dev` here): with `__TAURI_INTERNALS__` present,
//!   `tauri_guest::connect` gets the messages from the plugin; switching the locale switches
//!   Rust, the native labels and every window, and changes from Rust arrive as events.
//!
//! Both install the instance, so components call `t!` / `tc!`, and subscribe to it to render
//! again after a change. Only `js-sys` talks to the browser here, as in i18n-next.

use i18n_next::{I18n, t, tauri_guest, tc};
use js_sys::futures::JsFuture;
use js_sys::wasm_bindgen::{JsCast, JsValue};
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The catalog's files, as the browser mode fetches them from `locales/`.
const FILES: [&str; 11] = [
    "en.json",
    "ru.json",
    "ar.json",
    "pages/about/en.json",
    "pages/about/ru.json",
    "pages/about/ar.json",
    "pages/settings/en.json",
    "pages/settings/ru.json",
    "pages/tray/en.json",
    "pages/tray/ru.json",
    "pages/tray/ar.json",
];
const PAGES: [&str; 3] = ["index", "settings", "about"];
/// The `localStorage` key of the browser mode's locale.
const SAVED: &str = "i18n-next.locale";

fn get(o: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(o, &JsValue::from_str(key))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

/// `o.name(args…)`, `None` when it is missing or throws.
fn call(o: &JsValue, name: &str, args: &[JsValue]) -> Option<JsValue> {
    let f: Function = get(o, name)?.dyn_into().ok()?;
    Reflect::apply(&f, o, &args.iter().collect::<Array>()).ok()
}

fn in_tauri() -> bool {
    get(&js_sys::global(), "__TAURI_INTERNALS__").is_some_and(|v| v.is_object())
}

/// `localStorage` (it throws in some private windows: then nothing is kept).
fn storage() -> Option<JsValue> {
    get(&js_sys::global(), "localStorage")
}

/// `fetch("locales/<path>")` with the platform's `fetch`: what `ensure_async` asks for.
async fn fetch(path: String) -> Result<Vec<u8>, String> {
    fetch_url(&format!("locales/{path}")).await
}

/// The body at `url`, relative to the page.
async fn fetch_url(path: &str) -> Result<Vec<u8>, String> {
    let global = js_sys::global();
    let url = JsValue::from_str(path);
    let res = call(&global, "fetch", &[url]).ok_or("no fetch")?;
    let res = JsFuture::from(Promise::resolve(&res))
        .await
        .map_err(|e| format!("{e:?}"))?;
    if get(&res, "ok").and_then(|o| o.as_bool()) != Some(true) {
        return Err(format!("{path}: HTTP {:?}", get(&res, "status")));
    }
    let body = call(&res, "arrayBuffer", &[]).ok_or("no body")?;
    let body = JsFuture::from(Promise::resolve(&body))
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(Uint8Array::new(&body).to_vec())
}

/// The browser mode's catalog: the saved locale, else the best of `navigator.languages`.
async fn browser() -> Result<I18n, i18n_next::Error> {
    let i18n = I18n::builder()
        .config_json(include_str!("../../i18n.json"))
        .remote(FILES)
        .linked_messages(true)
        .build()?;
    let saved = storage()
        .and_then(|s| call(&s, "getItem", &[JsValue::from_str(SAVED)]))
        .and_then(|v| v.as_string())
        .filter(|c| i18n.has_locale(c));
    let languages: Vec<String> = get(&js_sys::global(), "navigator")
        .and_then(|n| get(&n, "languages"))
        .map(|l| {
            Array::from(&l)
                .iter()
                .filter_map(|v| v.as_string())
                .collect()
        })
        .unwrap_or_default();
    let codes: Vec<String> = i18n.locales().into_iter().map(|l| l.code).collect();
    let start = saved.or_else(|| i18n_next::negotiate(&languages.join(","), &codes));
    if let Some(code) = start {
        i18n.set_locale(&code)?;
    }
    i18n.ensure_async(&i18n.locale(), None, fetch).await?;
    Ok(i18n)
}

/// Loads `page` in the active locale: from the plugin, or the files it lacks (again when
/// the locale switched while they loaded).
async fn show(page: &str) -> Result<(), i18n_next::Error> {
    if in_tauri() {
        return tauri_guest::set_page(page).await;
    }
    let i18n = i18n_next::global().ok_or(i18n_next::Error::NotInstalled)?;
    loop {
        let locale = i18n.locale();
        i18n.ensure_async(&locale, Some(page), fetch).await?;
        if *i18n.locale() == *locale {
            return Ok(());
        }
    }
}

/// Switches the locale: Rust and every window (the plugin loads the shown page), or this
/// page (its files first, so no key shows while they load) and `localStorage`; then the page
/// shown by now, which a navigation during the switch may have changed.
async fn switch(code: String, page: RwSignal<String>) -> Result<(), i18n_next::Error> {
    if in_tauri() {
        return tauri_guest::set_locale(&code).await;
    }
    let i18n = i18n_next::global().ok_or(i18n_next::Error::NotInstalled)?;
    i18n.ensure_async(&code, Some(&page.get_untracked()), fetch)
        .await?;
    i18n.set_locale(&code)?;
    if let Some(s) = storage() {
        call(
            &s,
            "setItem",
            &[JsValue::from_str(SAVED), JsValue::from_str(&code)],
        );
    }
    show(&page.get_untracked()).await
}

/// `wasm-size.txt` of the trunk hook (`<bytes> <gzip bytes>`), shown in the footer.
async fn wasm_size() -> String {
    let Ok(body) = fetch_url("wasm-size.txt").await else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&body);
    let kb = |s: &str| s.parse::<f64>().map(|n| n / 1024.0).unwrap_or_default();
    match text.split_whitespace().collect::<Vec<_>>()[..] {
        [raw, gzip] => format!(" · .wasm {:.0} KB (gzip {:.0} KB)", kb(raw), kb(gzip)),
        _ => String::new(),
    }
}

/// `<html lang dir>` of the active locale.
fn html_lang(i18n: &I18n) {
    let code = i18n.locale();
    let info = i18n.locales().into_iter().find(|l| *l.code == *code);
    let Some(html) = get(&js_sys::global(), "document").and_then(|d| get(&d, "documentElement"))
    else {
        return;
    };
    let lang = info
        .as_ref()
        .and_then(|l| l.iso.clone())
        .unwrap_or(code.to_string());
    let dir = info.and_then(|l| l.dir).unwrap_or_else(|| "ltr".to_owned());
    let _ = Reflect::set(&html, &"lang".into(), &lang.into());
    let _ = Reflect::set(&html, &"dir".into(), &dir.into());
}

fn page_view(page: &str) -> AnyView {
    // 2024-01-05 04:24 UTC; the zone is the platform's (Intl), as in the WebView
    let at = 1_704_428_640_000i64;
    let now = js_sys::Date::now() as i64;
    match page {
        "settings" => view! {
            <h1>{t!("title", @page = "settings").to_string()}</h1>
            <p>{t!("language", @page = "settings").to_string()}</p>
        }
        .into_any(),
        "about" => view! {
            <h1>{t!("title", @page = "about").to_string()}</h1>
            <p>{t!("body", @page = "about", brand = "i18n-next").to_string()}</p>
            <p>{t!("source", @page = "about").to_string()}</p>
        }
        .into_any(),
        _ => view! {
            <h1>{t!("greeting", name = "Leptos").to_string()}</h1>
            <p>{tc!("apples", 3).to_string()}</p>
            <p>{tc!("cart", 0).to_string()}</p>
            <p>{t!("demo.price", price = i18n_next::tn(1234.5)).to_string()}</p>
            <p>{t!("demo.today", @date = at).to_string()}</p>
            <p>{t!("demo.updated", @relative = now - 90_000).to_string()}</p>
            <p>{t!("shout").to_string()}</p>
        }
        .into_any(),
    }
}

#[component]
fn App(i18n: I18n) -> impl IntoView {
    // every change of the catalog (a switch here, in Rust or in another window) renders again
    let version = RwSignal::new(i18n.version());
    let sub = i18n.subscribe(move |c| version.set(c.version));
    std::mem::forget(sub); // the app's lifetime
    let page = RwSignal::new("index".to_owned());
    let error = RwSignal::new(String::new());
    let report = move |r: Result<(), i18n_next::Error>| {
        error.set(r.err().map(|e| e.to_string()).unwrap_or_default());
    };
    let go = move |p: &'static str| {
        spawn_local(async move {
            let r = show(p).await;
            if r.is_ok() {
                page.set(p.to_owned());
            }
            report(r);
        });
    };
    let to = move |code: String| {
        spawn_local(async move { report(switch(code, page).await) });
    };
    let locales = i18n.locales();
    let shown = i18n.clone();
    Effect::new(move || {
        version.track();
        html_lang(&shown);
    });
    let current = i18n.clone();
    let size = RwSignal::new(String::new());
    spawn_local(async move { size.set(wasm_size().await) });
    view! {
        <nav>
            {PAGES
                .map(|p| {
                    let key = match p {
                        "index" => "nav.home",
                        other => if other == "settings" { "nav.settings" } else { "nav.about" },
                    };
                    view! {
                        <button
                            aria-pressed=move || (page.get() == p).to_string()
                            on:click=move |_| go(p)
                        >
                            {move || {
                                version.track();
                                t!(key).to_string()
                            }}
                        </button>
                    }
                })
                .collect_view()}
        </nav>
        <div class="locales">
            {locales
                .into_iter()
                .map(|l| {
                    let code = l.code.clone();
                    let name = l.display_name.unwrap_or_else(|| l.code.clone());
                    let i18n = current.clone();
                    let active = move || {
                        version.track();
                        (*i18n.locale() == *l.code).to_string()
                    };
                    view! {
                        <button aria-pressed=active on:click=move |_| to(code.clone())>
                            {name}
                        </button>
                    }
                })
                .collect_view()}
        </div>
        <p class="error">{move || error.get()}</p>
        <main>
            {move || {
                version.track();
                page_view(&page.get())
            }}
        </main>
        <footer>
            {move || {
                version.track();
                t!("footer").to_string()
            }}
            " · "
            {if in_tauri() { "Tauri (tauri-guest)" } else { "browser (remote + localStorage)" }}
            {move || size.get()}
        </footer>
    }
}

fn main() {
    spawn_local(async {
        let started = if in_tauri() {
            tauri_guest::connect("index").await
        } else {
            browser().await
        };
        match started {
            Ok(i18n) => {
                let _ = i18n_next::install(i18n.clone());
                leptos::mount::mount_to_body(move || view! { <App i18n /> });
            }
            Err(e) => {
                let text = e.to_string();
                leptos::mount::mount_to_body(move || view! { <p class="error">{text}</p> });
            }
        }
    });
}
