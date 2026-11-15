//! Errors returned to the frontend. Serialized as `{ "kind": "...", "message": "..." }`
//! so guest-js can branch on `kind` without parsing text.
//!
//! `Display` and `Error` are written by hand (no `thiserror`, DESIGN 2.3).

use serde::Serialize;
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum Error {
    /// A page name the catalog does not know (DESIGN 8b.2). Arrives with M7.
    UnknownPage(String),
    /// A locale code the catalog does not know (DESIGN 8b.2). Arrives with M7.
    UnknownLocale(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPage(name) => write!(f, "unknown page: {name}"),
            Self::UnknownLocale(code) => write!(f, "unknown locale: {code}"),
        }
    }
}

impl std::error::Error for Error {}
