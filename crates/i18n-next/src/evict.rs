//! Page eviction (DESIGN 8b.16): which loaded pages leave the next snapshot.

use crate::i18n::{DataMap, LocaleData};
use crate::lazy::Layer;
use std::sync::Arc;

/// The data without the oldest loaded evictable pages, when more than `limit` are loaded;
/// `None` when nothing has to go. An evicted page gets a fresh, empty cell of the same
/// files: a local one loads again on the next access, a remote one waits for the next
/// `ensure_async` / `add_file`.
pub(crate) fn plan(data: &DataMap, limit: usize) -> Option<DataMap> {
    let mut loaded: Vec<(u64, &Arc<str>, &Arc<str>)> = Vec::new();
    for (locale, d) in data {
        for (page, layer) in &d.pages {
            if let Layer::Lazy(cell) = layer
                && cell.evictable
                && cell.loaded().is_some()
            {
                loaded.push((cell.stamp(), locale, page));
            }
        }
    }
    let over = loaded.len().checked_sub(limit).filter(|n| *n > 0)?;
    // ponytail: a scan per evicted page (loads go one at a time, so `over` is 1 but after
    // a reload); loads are rare next to reads. No sort: its code would cost ~4 KB of wasm.
    let mut oldest = Vec::with_capacity(over);
    for _ in 0..over {
        let Some(at) = (0..loaded.len()).min_by_key(|&i| loaded[i].0) else {
            break;
        };
        oldest.push(loaded.swap_remove(at));
    }
    let mut out = data.clone();
    for (_, locale, page) in &oldest {
        let Some(slot) = out.get_mut(*locale) else {
            continue;
        };
        let slot: &mut LocaleData = Arc::make_mut(slot);
        let fresh = match slot.pages.get(&***page) {
            Some(Layer::Lazy(cell)) => Layer::Lazy(Arc::new(cell.fresh())),
            _ => continue,
        };
        slot.pages.insert((*page).clone(), fresh);
    }
    Some(out)
}
