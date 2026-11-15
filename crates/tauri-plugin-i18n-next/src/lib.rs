#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

pub mod commands;
pub mod error;

pub use error::{Error, Result};

/// The plugin, registered as `i18n-next`.
///
/// ```rust,no_run
/// # fn main() {
/// tauri::Builder::default()
///     .plugin(tauri_plugin_i18n_next::init())
/// #   ;
/// # }
/// ```
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("i18n-next")
        .invoke_handler(tauri::generate_handler![commands::ping])
        .build()
}
