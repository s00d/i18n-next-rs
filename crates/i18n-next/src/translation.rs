//! What [`I18n::t`](crate::I18n::t) returns: the stored text shared, or the key it was
//! asked for. No copy either way, so a lookup allocates nothing.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

/// The text of a key as [`I18n::t`](crate::I18n::t) found it: a shared reference to the
/// stored string, or, on a miss (and for a subtree or `""`), the key itself.
///
/// It reads as a `&str` (`Deref`, `AsRef`, `Display`, comparisons with `str` and
/// `String`); [`String::from`] or [`to_string`](ToString::to_string) makes an owned copy
/// when it has to outlive the key.
///
/// ```
/// use i18n_next::I18n;
/// let i18n = I18n::builder().messages("en", None, r#"{"hi": "Hello"}"#).build()?;
/// let hi = i18n.t("hi");
/// assert_eq!(hi, "Hello");
/// assert_eq!(hi.len(), 5);
/// assert_eq!(format!("{hi}!"), "Hello!");
/// let owned: String = i18n.t("nope").into();
/// assert_eq!(owned, "nope");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Clone)]
pub struct Translation<'k>(Repr<'k>);

#[derive(Clone)]
enum Repr<'k> {
    Shared(Arc<str>),
    Key(&'k str),
}

impl<'k> Translation<'k> {
    pub(crate) fn shared(text: Arc<str>) -> Self {
        Translation(Repr::Shared(text))
    }

    pub(crate) fn key(key: &'k str) -> Self {
        Translation(Repr::Key(key))
    }

    fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Shared(t) => t,
            Repr::Key(k) => k,
        }
    }
}

impl Deref for Translation<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Translation<'_> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Translation<'_> {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Translation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for Translation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

/// The text itself, as a miss would give it back.
impl<'k> From<&'k str> for Translation<'k> {
    fn from(text: &'k str) -> Self {
        Translation::key(text)
    }
}

impl From<Translation<'_>> for String {
    fn from(t: Translation<'_>) -> String {
        t.as_str().to_owned()
    }
}

impl From<Translation<'_>> for Arc<str> {
    fn from(t: Translation<'_>) -> Arc<str> {
        match t.0 {
            Repr::Shared(t) => t,
            Repr::Key(k) => Arc::from(k),
        }
    }
}

impl Eq for Translation<'_> {}

impl PartialEq for Translation<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl std::hash::Hash for Translation<'_> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.as_str().hash(h);
    }
}

macro_rules! eq_str {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Translation<'_> {
            fn eq(&self, other: &$t) -> bool {
                self.as_str() == AsRef::<str>::as_ref(other)
            }
        }
        impl PartialEq<Translation<'_>> for $t {
            fn eq(&self, other: &Translation<'_>) -> bool {
                AsRef::<str>::as_ref(self) == other.as_str()
            }
        }
    )*};
}
eq_str!(str, &str, String, std::borrow::Cow<'_, str>);

/// The text as a string: a command returning a `Translation` sends the text.
#[cfg(feature = "serde")]
impl serde::Serialize for Translation<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
