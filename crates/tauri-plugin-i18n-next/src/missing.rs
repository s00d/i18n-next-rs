//! The dev missing-key report (DESIGN 8b.7): Rust misses (through [`missing`] as the
//! catalog's `missing` handler) and front-end misses (`report_missing`) deduplicated by
//! `(locale, page, key)` with a count. Each new key rewrites `i18n-missing.json` and
//! emits [`crate::EVENT_MISSING`]. Release builds record nothing.
//!
//! The report is one per process: the handler is set on the catalog before the plugin
//! exists, so the plugin attaches to it at start; misses before that are kept.

use crate::models::MissingEntry;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Runtime};

type Notify = Arc<dyn Fn(&MissingEntry, &[u8]) + Send + Sync>;

struct Report {
    counts: BTreeMap<(String, String, String), u64>,
    notify: Option<Notify>,
}

static REPORT: Mutex<Report> = Mutex::new(Report {
    counts: BTreeMap::new(),
    notify: None,
});

/// The catalog's `missing` handler that feeds the dev report:
/// `I18n::builder().missing(tauri_plugin_i18n_next::missing)`. A no-op in release builds.
///
/// ```
/// let i18n = i18n_next::I18n::builder()
///     .missing(tauri_plugin_i18n_next::missing)
///     .build()?;
/// assert_eq!(i18n.t("nope"), "nope");
/// # Ok::<(), i18n_next::Error>(())
/// ```
pub fn missing(locale: &str, key: &str, page: &str) {
    record(std::iter::once((locale, page, key)));
}

/// Counts `(locale, page, key)` misses; new keys are written and announced.
pub(crate) fn record<'a>(misses: impl Iterator<Item = (&'a str, &'a str, &'a str)>) {
    if !cfg!(debug_assertions) {
        return;
    }
    let (news, file, notify) = {
        let mut r = REPORT.lock().unwrap_or_else(PoisonError::into_inner);
        let mut news = Vec::new();
        for (locale, page, key) in misses {
            let count = r
                .counts
                .entry((locale.to_owned(), page.to_owned(), key.to_owned()))
                .or_insert(0);
            *count += 1;
            if *count == 1 {
                news.push(entry(locale, page, key, 1));
            }
        }
        if news.is_empty() || r.notify.is_none() {
            return;
        }
        (news, file_bytes(&r), r.notify.clone())
    };
    if let Some(notify) = notify {
        for e in &news {
            notify(e, &file);
        }
    }
}

fn entry(locale: &str, page: &str, key: &str, count: u64) -> MissingEntry {
    MissingEntry {
        locale: locale.to_owned(),
        page: page.to_owned(),
        key: key.to_owned(),
        count,
    }
}

/// The whole report as the file holds it: a JSON array, sorted.
fn file_bytes(r: &Report) -> Vec<u8> {
    let all: Vec<MissingEntry> = r
        .counts
        .iter()
        .map(|((l, p, k), c)| entry(l, p, k, *c))
        .collect();
    serde_json::to_vec_pretty(&all).unwrap_or_default()
}

/// Sends new keys of the report to `app` and `file` from now on (dev, at plugin start),
/// and the ones collected before.
pub(crate) fn attach<R: Runtime>(app: AppHandle<R>, file: PathBuf) {
    let notify: Notify = Arc::new(move |e, bytes| {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, bytes);
        let _ = app.emit(crate::EVENT_MISSING, e);
    });
    let (before, bytes) = {
        let mut r = REPORT.lock().unwrap_or_else(PoisonError::into_inner);
        r.notify = Some(notify.clone());
        let before: Vec<MissingEntry> = r
            .counts
            .iter()
            .map(|((l, p, k), c)| entry(l, p, k, *c))
            .collect();
        (before, file_bytes(&r))
    };
    for e in &before {
        notify(e, &bytes);
    }
}
