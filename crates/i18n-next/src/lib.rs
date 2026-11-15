#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

/// The crate version, as published (`CARGO_PKG_VERSION`).
///
/// ```
/// assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
