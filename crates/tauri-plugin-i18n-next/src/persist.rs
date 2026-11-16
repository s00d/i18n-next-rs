//! The remembered locale (DESIGN 8b.3): `{"locale": "de"}`, written atomically.

use std::path::Path;

/// The saved locale, if the file is there and well-formed.
pub(crate) fn read(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("locale")?.as_str().map(str::to_owned)
}

/// Writes the file through a temporary one and a rename: a crash never leaves half a file.
pub(crate) fn write(path: &Path, locale: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::json!({ "locale": locale }).to_string())?;
    std::fs::rename(&tmp, path)
}
