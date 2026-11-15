#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

// Building blocks of the catalog (M1, DESIGN 2.3). Their tests and the `json` bench
// include these files with `#[path]`, so they stay crate-private and the public API
// does not change.
#[allow(dead_code, reason = "used by the catalog, which lands next in M1")]
mod hash;
#[allow(dead_code, reason = "used by the catalog, which lands next in M1")]
mod json;

/// The crate version, as published (`CARGO_PKG_VERSION`).
///
/// ```
/// assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
