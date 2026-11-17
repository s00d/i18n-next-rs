//! The dev missing-key report (DESIGN 8b.7): Rust misses (through [`missing`] as the
//! catalog's `missing` handler) and front-end misses (`report_missing`) deduplicated by
//! `(locale, page, key)` with a count. A call that brings new keys rewrites
//! `i18n-missing.json` once and emits [`crate::EVENT_MISSING`] per new key, in the order
//! the report changed. At most 10 000 keys of up to 512 bytes are kept. Release builds
//! record nothing.
//!
//! The report is one per process: the handler is set on the catalog before the plugin
//! exists, so the plugin attaches to it at start; misses before that are kept.

use crate::models::MissingEntry;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Runtime};

/// What a batch of new keys does: one write of the whole report, one event per new key.
type Notify = Arc<dyn Fn(&[MissingEntry], &[u8]) + Send + Sync>;

/// Keys kept in the report; misses of further new keys are dropped (a runaway loop over
/// generated keys must not grow the dev process without bound).
const MAX_ENTRIES: usize = 10_000;
/// Longest locale, page or key kept, in bytes; longer ones are dropped (the front end
/// sends any string).
const MAX_LEN: usize = 512;

struct Report {
    counts: BTreeMap<(String, String, String), u64>,
    notify: Option<Notify>,
}

static REPORT: Mutex<Report> = Mutex::new(Report {
    counts: BTreeMap::new(),
    notify: None,
});

/// Held from a change of the report to the end of its write, so the file and the events
/// follow the report's order: a later batch never writes an older report over a newer one.
static WRITE: Mutex<()> = Mutex::new(());

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

/// Counts `(locale, page, key)` misses; the new keys of one call are written once and
/// announced.
pub(crate) fn record<'a>(misses: impl Iterator<Item = (&'a str, &'a str, &'a str)>) {
    if !cfg!(debug_assertions) {
        return;
    }
    let _write = WRITE.lock().unwrap_or_else(PoisonError::into_inner);
    let (news, file, notify) = {
        let mut r = REPORT.lock().unwrap_or_else(PoisonError::into_inner);
        let mut news = Vec::new();
        for (locale, page, key) in misses {
            if [locale, page, key].iter().any(|s| s.len() > MAX_LEN) {
                continue;
            }
            let at = (locale.to_owned(), page.to_owned(), key.to_owned());
            if let Some(count) = r.counts.get_mut(&at) {
                *count += 1;
            } else if r.counts.len() < MAX_ENTRIES {
                r.counts.insert(at, 1);
                news.push(entry(locale, page, key, 1));
            }
        }
        if news.is_empty() {
            return;
        }
        let Some(notify) = r.notify.clone() else {
            return;
        };
        (news, file_bytes(&r), notify)
    };
    notify(&news, &file);
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
    set_notify(Arc::new(move |news, bytes| {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, bytes);
        for e in news {
            let _ = app.emit(crate::EVENT_MISSING, e);
        }
    }));
}

/// Makes `notify` the report's output and hands it what was collected before.
fn set_notify(notify: Notify) {
    let _write = WRITE.lock().unwrap_or_else(PoisonError::into_inner);
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
    if !before.is_empty() {
        notify(&before, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given the report, when one call brings many new keys, then the report is written
    /// once with all of them (not once per key); over-long names and keys past the cap are
    /// dropped, known ones still count.
    #[test]
    fn a_batch_is_written_once_and_the_report_is_bounded() {
        let writes = Arc::new(Mutex::new(Vec::<(usize, usize)>::new()));
        let log = writes.clone();
        set_notify(Arc::new(move |news, bytes| {
            let all: Vec<serde_json::Value> = serde_json::from_slice(bytes).unwrap();
            log.lock().unwrap().push((news.len(), all.len()));
        }));
        // step 1: 1000 new keys in one call → one write
        let keys: Vec<String> = (0..1000).map(|i| format!("k{i}")).collect();
        record(keys.iter().map(|k| ("en", "index", k.as_str())));
        assert_eq!(*writes.lock().unwrap(), [(1000, 1000)]);
        // step 2: an over-long key is dropped, one of the longest allowed is kept
        let long = "x".repeat(MAX_LEN + 1);
        record(std::iter::once(("en", "index", long.as_str())));
        assert_eq!(writes.lock().unwrap().len(), 1);
        let longest = "x".repeat(MAX_LEN);
        record(std::iter::once(("en", "index", longest.as_str())));
        assert_eq!(writes.lock().unwrap().last(), Some(&(1, 1001)));
        // step 3: past the cap new keys are dropped, a known one still counts
        let more: Vec<String> = (0..MAX_ENTRIES).map(|i| format!("m{i}")).collect();
        record(more.iter().map(|k| ("en", "index", k.as_str())));
        assert_eq!(
            writes.lock().unwrap().last(),
            Some(&(MAX_ENTRIES - 1001, MAX_ENTRIES))
        );
        record(std::iter::once(("en", "index", "k1")));
        record(std::iter::once(("en", "index", "brand-new")));
        assert_eq!(writes.lock().unwrap().len(), 3);
        let r = REPORT.lock().unwrap();
        assert_eq!(r.counts[&("en".into(), "index".into(), "k1".into())], 2);
    }
}
