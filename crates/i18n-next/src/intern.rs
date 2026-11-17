//! The texts of every catalog in the process, each stored once and never freed: a lookup
//! hands out a plain `&'static str` (no reference count, no copy), whatever `reload`,
//! `set_locale` or an evicted page does to the catalog afterwards.
//!
//! The cost is memory: a text stays for the life of the process. Equal texts share one
//! copy, so a reload of unchanged files adds nothing; only texts never seen before grow it.

use crate::hash::FastHasher;
use std::collections::HashSet;
use std::hash::BuildHasherDefault;
use std::sync::Mutex;

static TEXTS: Mutex<HashSet<&'static str, BuildHasherDefault<FastHasher>>> =
    Mutex::new(HashSet::with_hasher(BuildHasherDefault::new()));

/// `s` stored for good: the copy already there, else a new one.
pub(crate) fn intern(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    let mut texts = TEXTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&t) = texts.get(s) {
        return t;
    }
    let t: &'static str = Box::leak(Box::from(s));
    texts.insert(t);
    t
}

#[cfg(test)]
mod tests {
    use super::intern;

    /// Equal texts come back as one copy, different texts as their own.
    #[test]
    fn one_copy_per_text() {
        let a = intern(&String::from("intern-test-a"));
        let b = intern(&String::from("intern-test-a"));
        assert!(std::ptr::eq(a, b));
        assert_eq!(intern("intern-test-b"), "intern-test-b");
        assert!(!std::ptr::eq(a, intern("intern-test-b")));
        assert_eq!(intern(""), "");
    }
}
