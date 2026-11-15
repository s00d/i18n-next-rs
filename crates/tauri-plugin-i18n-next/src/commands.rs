//! IPC commands of the plugin (`plugin:i18n-next|<name>`). Commands live here, not in
//! the crate root, so tests can register them on a mock app by path.

use crate::error::Result;

/// M0 placeholder: answers with the `i18n-next` version (the same release as the
/// plugin, DESIGN 8b.13), so the IPC path, permissions and tests have something real
/// to call until M7.
#[tauri::command]
pub fn ping() -> Result<String> {
    Ok(i18n_next::version().to_owned())
}
