//! Errors returned to the frontend. Serialized as `{ "kind": "...", "message": "..." }`
//! so guest-js can branch on `kind` without parsing text; `message` is the `Display`
//! text, ready to show or log.
//!
//! `Display`, `Error` and `Serialize` are written by hand (no `thiserror`, DESIGN 2.3).

use serde::ser::{Serialize, SerializeStruct, Serializer};
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let kind = match self {
            Self::UnknownPage(_) => "UnknownPage",
            Self::UnknownLocale(_) => "UnknownLocale",
        };
        let mut s = serializer.serialize_struct("Error", 2)?;
        s.serialize_field("kind", kind)?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}
