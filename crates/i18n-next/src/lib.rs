#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[macro_use]
mod macros;
mod args;
mod catalog;
mod change;
mod check;
mod error;
mod evict;
#[cfg(feature = "format")]
mod format;
mod hash;
mod i18n;
mod json;
mod lazy;
mod linked;
mod loader;
#[cfg(feature = "payload")]
pub mod payload;
mod plural;
mod remote;
mod scope;
mod source;
mod text;
mod trace;
#[cfg(feature = "tz")]
#[allow(
    dead_code,
    reason = "backends of other platforms (Android, browser) and items only tests read"
)]
mod tz;
#[cfg(all(
    feature = "watch",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
mod watch;

pub use args::{Arg, Args, Count};
pub use change::{Change, Subscription};
pub use check::{CheckItem, CheckReport, LinkError};
pub use error::Error;
#[cfg(feature = "format")]
pub use format::api::{td, td_as, tdr, tn, tn_as};
#[cfg(feature = "format")]
pub use format::{DateArg, Number, RelativeOptions, Timestamp};
pub use i18n::{Builder, I18n, LocaleInfo, View};
#[doc(hidden)]
pub use macros::__ident;
pub use scope::{Scope, Scoped, locale, negotiate, page, scope, set_locale, t};
#[cfg(feature = "global")]
pub use scope::{global, install};
pub use text::{Key, Part, PluralCtx, Text};
#[cfg(all(
    feature = "watch",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
pub use watch::WatchGuard;

/// The crate version, as published (`CARGO_PKG_VERSION`).
///
/// ```
/// assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
