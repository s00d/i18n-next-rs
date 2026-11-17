//! Keys of the catalog's path maps with their hash computed once. A lookup walks a chain of
//! maps (page and root of each locale in the fallback chain); with [`Probe`] the key is
//! hashed once for the whole chain instead of once per map.
//!
//! `std`'s `HashMap` takes no precomputed hash, so the maps are keyed by [`Key`] (the text
//! and its hash, hashed as that `u64`) and looked up through `dyn Lookup`, which both a
//! stored [`Key`] and a borrowed [`Probe`] implement with the same hash and equality.

use crate::hash::{FastHasher, FastMap};
use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

/// The hash a [`Key`] and a [`Probe`] carry: the map hasher over the text, as `str` hashes.
fn hash_of(text: &str) -> u64 {
    let mut h = FastHasher::default();
    text.hash(&mut h);
    h.finish()
}

/// A stored path: the text and its hash.
#[derive(Clone)]
pub(crate) struct Key {
    hash: u64,
    text: Arc<str>,
}

impl Key {
    pub(crate) fn text(&self) -> &Arc<str> {
        &self.text
    }
}

impl From<Arc<str>> for Key {
    fn from(text: Arc<str>) -> Self {
        Key {
            hash: hash_of(&text),
            text,
        }
    }
}

impl From<&str> for Key {
    fn from(text: &str) -> Self {
        Key::from(Arc::<str>::from(text))
    }
}

impl Deref for Key {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.text, f)
    }
}

/// A key to look up, hashed once: build it once per lookup and pass it to every map.
#[derive(Clone, Copy)]
pub(crate) struct Probe<'a> {
    hash: u64,
    text: &'a str,
}

impl<'a> Probe<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Probe {
            hash: hash_of(text),
            text,
        }
    }

    pub(crate) fn text(&self) -> &'a str {
        self.text
    }
}

/// What a map lookup compares: the hash, then the text.
pub(crate) trait Lookup {
    fn hash(&self) -> u64;
    fn text(&self) -> &str;
}

impl Lookup for Key {
    fn hash(&self) -> u64 {
        self.hash
    }
    fn text(&self) -> &str {
        &self.text
    }
}

impl Lookup for Probe<'_> {
    fn hash(&self) -> u64 {
        self.hash
    }
    fn text(&self) -> &str {
        self.text
    }
}

impl Hash for dyn Lookup + '_ {
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_u64(Lookup::hash(self));
    }
}

impl PartialEq for dyn Lookup + '_ {
    fn eq(&self, other: &Self) -> bool {
        Lookup::hash(self) == Lookup::hash(other) && self.text() == other.text()
    }
}

impl Eq for dyn Lookup + '_ {}

// Same hash and equality as `dyn Lookup`, as `Borrow` requires.
impl Hash for Key {
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_u64(self.hash);
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.text == other.text
    }
}

impl Eq for Key {}

impl<'a> Borrow<dyn Lookup + 'a> for Key {
    fn borrow(&self) -> &(dyn Lookup + 'a) {
        self
    }
}

/// A path map: lookups by [`Probe`] (hashed once) or by text.
pub(crate) type PathMap<V> = FastMap<Key, V>;

/// Lookups on a [`PathMap`].
pub(crate) trait PathMapExt<V> {
    fn at(&self, key: &Probe<'_>) -> Option<&V>;
    fn at_str(&self, key: &str) -> Option<&V> {
        self.at(&Probe::new(key))
    }
    fn at_mut(&mut self, key: &str) -> Option<&mut V>;
    fn key_at(&self, key: &str) -> Option<&Key>;
    fn take(&mut self, key: &str) -> Option<V>;
}

impl<V> PathMapExt<V> for PathMap<V> {
    #[inline]
    fn at(&self, key: &Probe<'_>) -> Option<&V> {
        self.get(key as &dyn Lookup)
    }
    fn at_mut(&mut self, key: &str) -> Option<&mut V> {
        self.get_mut(&Probe::new(key) as &dyn Lookup)
    }
    fn key_at(&self, key: &str) -> Option<&Key> {
        self.get_key_value(&Probe::new(key) as &dyn Lookup)
            .map(|(k, _)| k)
    }
    fn take(&mut self, key: &str) -> Option<V> {
        self.remove(&Probe::new(key) as &dyn Lookup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scenario: a map keyed by `Key` finds every stored path through a probe, a probe
    /// built once reads several maps, and texts that differ never match (even if a hash
    /// did, equality compares the text).
    #[test]
    fn probe_reads_what_key_stored() {
        let mut a: PathMap<u32> = PathMap::default();
        let mut b: PathMap<u32> = PathMap::default();
        for (i, k) in ["nav.home", "nav", "a.b.c", ""].iter().enumerate() {
            a.insert(Key::from(*k), i as u32);
        }
        b.insert(Key::from("nav.home"), 9);
        let p = Probe::new("nav.home");
        assert_eq!((a.at(&p), b.at(&p)), (Some(&0), Some(&9)));
        assert_eq!(a.at_str("a.b.c"), Some(&2));
        assert_eq!(a.at_str(""), Some(&3));
        assert_eq!(a.at_str("nav.hom"), None);
        assert_eq!(&**a.key_at("nav").unwrap(), "nav");
        *a.at_mut("nav").unwrap() = 7;
        assert_eq!(a.take("nav"), Some(7));
        assert_eq!(a.at_str("nav"), None);
        // equality checks the text, not only the hash
        let fake = Key {
            hash: Probe::new("x").hash,
            text: Arc::from("y"),
        };
        assert!(Key::from("x") != fake);
    }
}
