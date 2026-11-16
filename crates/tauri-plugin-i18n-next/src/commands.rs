//! IPC commands of the plugin (`plugin:i18n-next|<name>`, DESIGN 6.1). They live here, not
//! in the crate root, so tests can register them on a mock app by path. Every name from
//! the front end is checked against the catalog first (8b.2).

use crate::Handle;
use crate::error::{Error, Result};
use crate::models::{Locale, MissingKey};
use i18n_next::{I18n, payload};
use tauri::State;
use tauri::ipc::Response;

/// `Ok` for a locale that is known and not disabled.
pub(crate) fn known_locale(i18n: &I18n, code: &str) -> Result<()> {
    if i18n.has_locale(code) {
        Ok(())
    } else {
        Err(Error::UnknownLocale(code.to_owned()))
    }
}

/// `Ok` for `index` (the root) and every page the catalog knows.
pub(crate) fn known_page(i18n: &I18n, page: &str) -> Result<()> {
    if page == "index" || i18n.has_page(page) {
        Ok(())
    } else {
        Err(Error::UnknownPage(page.to_owned()))
    }
}

/// Start: the active chain's roots and, with `page`, its page chunks, plus `locales`,
/// `defaultLocale` and the named formats (the payload of `i18n_next::payload`), as raw
/// bytes. `languages` (`navigator.languages`) picks the start locale on the first call
/// when no saved one applies (8b.3). The fallback path when the protocol is unavailable.
#[tauri::command]
pub fn load(
    state: State<'_, Handle>,
    page: Option<String>,
    languages: Option<Vec<String>>,
) -> Result<Response> {
    if let Some(p) = &page {
        known_page(&state, p)?;
    }
    state.first_load(languages.as_deref());
    let (_, bytes) = payload::encode(&state, None, page.as_deref(), true);
    Ok(Response::new(bytes))
}

/// What the front end needs to start and to build protocol URLs, without the chunks:
/// `version`, `locale`, `defaultLocale`, `fallbackLocale`, `locales`, `chains` (the
/// fallback chain of every locale, which the front end premerges in order, DESIGN 6.2),
/// the named formats, `tags` (the content tag of each chunk of the chain of `locale`, the
/// active one when not given, roots and `page`, by `"<locale>/<page>"`) and `bundle` (the
/// tag of its protocol bundle). As raw bytes, like `load`. `languages`
/// (`navigator.languages`) picks the start locale like the first `load` does (8b.3), so a
/// front end that reads data over the protocol never needs `load`.
#[tauri::command]
pub fn meta(
    state: State<'_, Handle>,
    locale: Option<String>,
    page: Option<String>,
    languages: Option<Vec<String>>,
) -> Result<Response> {
    if let Some(l) = &locale {
        known_locale(&state, l)?;
    }
    if let Some(p) = &page {
        known_page(&state, p)?;
    }
    if languages.is_some() {
        state.first_load(languages.as_deref());
    }
    Ok(Response::new(payload::meta(
        &state,
        locale.as_deref(),
        page.as_deref(),
    )))
}

/// Navigation: the page chunks of the active chain, as raw bytes.
#[tauri::command]
pub fn load_page(state: State<'_, Handle>, page: String) -> Result<Response> {
    known_page(&state, &page)?;
    let (_, bytes) = payload::encode_page(&state, &page);
    Ok(Response::new(bytes))
}

/// Switches the locale for Rust, native labels and every window (they get
/// [`crate::EVENT_CHANGED`]); returns the new catalog version.
#[tauri::command]
pub fn set_locale(state: State<'_, Handle>, locale: String) -> Result<u64> {
    known_locale(&state, &locale)?;
    state
        .set_locale(&locale)
        .map_err(|_| Error::UnknownLocale(locale))?;
    Ok(state.version())
}

/// The active locale.
#[tauri::command]
pub fn get_locale(state: State<'_, Handle>) -> String {
    state.locale().to_string()
}

/// Locales that are not disabled: `i18n.json` order first, then the rest by code.
#[tauri::command]
pub fn available_locales(state: State<'_, Handle>) -> Vec<Locale> {
    state.locales().into_iter().map(Locale::from).collect()
}

/// Known page names, sorted.
#[tauri::command]
pub fn pages(state: State<'_, Handle>) -> Vec<String> {
    state.i18n.pages()
}

/// Keys the front end missed (dev builds only: the plugin registers it in debug, DESIGN
/// 8b.7). They join the Rust misses in the report.
#[tauri::command]
pub fn report_missing(keys: Vec<MissingKey>) {
    crate::missing::record(keys.iter().map(|k| (&*k.locale, &*k.page, &*k.key)));
}
