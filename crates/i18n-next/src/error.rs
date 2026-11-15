//! The crate's error (DESIGN 4.5): hand-written `Display` and `Error`, no `thiserror` (2.3).

use std::fmt;

/// What went wrong while building the catalog or switching it.
///
/// A broken or non-object locale file is an error with its path. The JS loader logs it and
/// skips the file; Rust refuses to start with half a catalog instead.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file or directory could not be read.
    Io {
        /// The path as given (a `dir` entry is joined to its directory).
        path: String,
        /// What the OS said.
        source: std::io::Error,
    },
    /// A locale file, `messages` text or `i18n.json` is not valid JSON.
    Json {
        /// The file path, or `messages:<locale>/<page>` for text passed in code.
        path: String,
        /// Byte offset of the problem (a BOM counts).
        offset: usize,
        /// What is wrong there.
        kind: &'static str,
    },
    /// Valid JSON, but not an object (an array, `null`, a string…).
    NotObject {
        /// The file path, or `messages:<locale>/<page>`.
        path: String,
    },
    /// A locale that no file, `i18n.json` entry or builder call names.
    UnknownLocale(String),
    /// A locale `i18n.json` marks `"disabled": true`.
    DisabledLocale(String),
    /// A page name that is not allowed or not known (`__proto__`, an alias to nothing).
    UnknownPage(String),
    /// `i18n.json` has the wrong shape.
    Config {
        /// The config path, or `i18n.json` for `config_json`.
        path: String,
        /// Which field is wrong.
        reason: &'static str,
    },
    /// A path `add_file` was given that the `remote` manifest does not list (8b.2: names
    /// only from the catalog).
    UnknownFile(String),
    /// The fetch closure of `ensure_async` failed for one file; the catalog is unchanged.
    Fetch {
        /// The manifest path that was being fetched.
        path: String,
        /// What the closure's error said (its `Display`).
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "cannot read {path}: {source}"),
            Error::Json { path, offset, kind } => {
                write!(f, "{path}: invalid JSON at byte {offset}: {kind}")
            }
            Error::NotObject { path } => write!(f, "{path}: expected a JSON object"),
            Error::UnknownLocale(code) => write!(f, "unknown locale \"{code}\""),
            Error::DisabledLocale(code) => write!(f, "locale \"{code}\" is disabled"),
            Error::UnknownPage(name) => write!(f, "unknown page \"{name}\""),
            Error::Config { path, reason } => write!(f, "{path}: {reason}"),
            Error::UnknownFile(path) => write!(f, "unknown file \"{path}\""),
            Error::Fetch { path, message } => write!(f, "cannot fetch {path}: {message}"),
        }
    }
}

impl Error {
    /// A copy for every waiter of one shared fetch (`Error` is not `Clone`: `Io` holds an
    /// `io::Error`, which `add_file` never returns; anything else becomes `Fetch` with its
    /// text).
    pub(crate) fn duplicate(&self, file: &str) -> Error {
        match self {
            Error::Json { path, offset, kind } => Error::Json {
                path: path.clone(),
                offset: *offset,
                kind,
            },
            Error::NotObject { path } => Error::NotObject { path: path.clone() },
            Error::Fetch { path, message } => Error::Fetch {
                path: path.clone(),
                message: message.clone(),
            },
            other => Error::Fetch {
                path: file.to_owned(),
                message: other.to_string(),
            },
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
