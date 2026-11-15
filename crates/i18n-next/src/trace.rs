//! Diagnostics (DESIGN 8b.17). With the `tracing` feature: misses are `warn!` with `locale`,
//! `page` and `key`; loads and `reload` are `debug!` events with the path, the bytes and
//! the time taken; `ensure_async` runs each fetch in a `debug` span. Without it every
//! function here is empty, except [`broken`]: a lazy file that cannot be read is reported
//! once to stderr in debug builds (there is no caller to hand the error to).

use crate::error::Error;

/// A miss of `t` / `tc` in `locale` (`page` is `index` for the root).
#[inline]
pub(crate) fn missing(locale: &str, page: &str, key: &str) {
    #[cfg(feature = "tracing")]
    tracing::warn!(locale, page, key, "i18n-next: missing translation");
    #[cfg(not(feature = "tracing"))]
    let _ = (locale, page, key);
}

/// Whether misses are traced (the lookup then keeps the active locale for the report).
pub(crate) const MISSES: bool = cfg!(feature = "tracing");

/// A load nobody can be told about failed: a lazy file at first use (its chunk stays
/// without it) or a `watch` reload (the old catalog stays).
#[cold]
pub(crate) fn broken(error: &Error) {
    #[cfg(feature = "tracing")]
    tracing::warn!(%error, "i18n-next: load failed");
    #[cfg(all(not(feature = "tracing"), debug_assertions))]
    eprintln!("i18n-next: load failed: {error}");
    #[cfg(all(not(feature = "tracing"), not(debug_assertions)))]
    let _ = error;
}

/// Time of one load or reload. `Instant::now` panics on `wasm32-unknown-unknown`, so the
/// duration is left out there.
pub(crate) struct Timer {
    #[cfg(all(
        feature = "tracing",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ))]
    start: std::time::Instant,
}

impl Timer {
    #[inline]
    pub(crate) fn start() -> Timer {
        Timer {
            #[cfg(all(
                feature = "tracing",
                not(all(target_arch = "wasm32", target_os = "unknown"))
            ))]
            start: std::time::Instant::now(),
        }
    }

    /// `what` (`load`, `reload`, `add_file`) of `path` with `bytes` read is done.
    #[inline]
    pub(crate) fn done(self, what: &'static str, path: &str, bytes: usize) {
        #[cfg(feature = "tracing")]
        {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            let micros = self.start.elapsed().as_micros() as u64;
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            let micros = 0u64;
            tracing::debug!(path, bytes, micros, "i18n-next: {what}");
        }
        #[cfg(not(feature = "tracing"))]
        let _ = (self, what, path, bytes);
    }
}
