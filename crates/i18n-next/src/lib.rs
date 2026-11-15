#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod args;
mod catalog;
mod check;
mod error;
mod hash;
mod i18n;
mod json;
mod linked;
mod loader;
mod plural;

pub use args::{Arg, Args, Count};
pub use check::{CheckItem, CheckReport, LinkError};
pub use error::Error;
pub use i18n::{Builder, I18n, LocaleInfo, View};

/// The crate version, as published (`CARGO_PKG_VERSION`).
///
/// ```
/// assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
