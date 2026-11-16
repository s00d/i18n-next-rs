//! What commands and events carry besides raw payloads (DESIGN 6.1).

use serde::{Deserialize, Serialize};

/// One available locale (`available_locales`): `Locale` of `@i18n-micro/types`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Locale {
    /// The locale code, as in file names (`de-at`).
    pub code: String,
    /// The BCP 47 tag (`de-AT`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iso: Option<String>,
    /// Text direction (`rtl`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// `displayName` of `i18n.json`, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl From<i18n_next::LocaleInfo> for Locale {
    fn from(l: i18n_next::LocaleInfo) -> Self {
        Self {
            code: l.code,
            iso: l.iso,
            dir: l.dir,
            display_name: l.display_name,
        }
    }
}

/// The payload of [`crate::EVENT_CHANGED`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Changed {
    /// The active locale after the change.
    pub locale: String,
    /// The catalog version the change produced: the `v{version}` of protocol URLs.
    pub version: u64,
}

/// A key the front end missed (`report_missing`, dev only, DESIGN 8b.7).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MissingKey {
    /// The locale it was looked up in.
    pub locale: String,
    /// The page (`index` for the root).
    pub page: String,
    /// The key.
    pub key: String,
}

/// One line of the missing-key report: a key and how often it was missed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingEntry {
    /// The locale it was looked up in.
    pub locale: String,
    /// The page (`index` for the root).
    pub page: String,
    /// The key.
    pub key: String,
    /// Misses so far (Rust and front end together).
    pub count: u64,
}
