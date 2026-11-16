//! Own hasher for the key maps (DESIGN 2.2, 2.3, spike S2): a fold over 16-byte blocks
//! with one folded multiply per block, as in `foldhash`. Keys are short and trusted (files
//! and code; names from the front end are checked against the known list, 8b.2), so the
//! seed is fixed, not DoS-resistant.
//!
//! Not the spike's Fx step (`(h.rotate_left(5) ^ w) * K`): on locale-shaped keys it gives
//! 18 full 64-bit collisions in 10k (`a.title185` / `a.title935`). A change in the top
//! byte of one word only reaches the top bits of the product, the rotate moves it into
//! the low byte, where the next word's difference cancels it. The folded multiply XORs
//! the high half of the 128-bit product into the low one, so every input bit reaches
//! every output bit; the scenario test below pins this.
//!
//! `str` hashing is `write(bytes)` then `write_u8(0xff)`: the bytes are folded, the
//! terminator only has to keep "a" + "b" apart from "ab" (in tuple keys), so `write_u8`
//! is a rotate and xor, no multiply. Integer keys are not what these maps hold: their
//! writes get one plain multiply (Fx).
//!
//! Self-contained (no `crate::` paths): the bench includes it with `#[path]`.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
/// Second multiplier for the other half of a block.
const K2: u64 = 0xa0_76_1d_64_78_bd_64_2f;
/// Start state: non-zero, so a run of zero words does not stay at zero.
const SEED: u64 = 0x24_3f_6a_88_85_a3_08_d3;

#[derive(Clone, Copy)]
pub(crate) struct FastHasher(u64);

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

impl Default for FastHasher {
    #[inline]
    fn default() -> Self {
        FastHasher(SEED)
    }
}

/// The 128-bit product of `a` and `b` with its high half folded into the low one.
#[inline]
fn fold(a: u64, b: u64) -> u64 {
    let full = u128::from(a) * u128::from(b);
    (full as u64) ^ ((full >> 64) as u64)
}

/// The 1..=7 bytes of `t` as a zero-padded little-endian word, from fixed-size reads: a
/// `copy_from_slice` of a variable length is a `memmove` call, which cost a short-key
/// lookup a fifth of its time. Two overlapping reads put each byte at its own position.
#[inline]
fn tail_word(t: &[u8]) -> u64 {
    let n = t.len();
    if n >= 4 {
        let lo = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
        let hi = u32::from_le_bytes([t[n - 4], t[n - 3], t[n - 2], t[n - 1]]);
        u64::from(lo) | u64::from(hi) << ((n - 4) * 8)
    } else {
        u64::from(t[0]) | u64::from(t[n / 2]) << (n / 2 * 8) | u64::from(t[n - 1]) << ((n - 1) * 8)
    }
}

impl FastHasher {
    #[inline]
    fn block(&mut self, a: u64, b: u64) {
        self.0 = fold(self.0 ^ a, b ^ K2);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, tail) = bytes.as_chunks::<8>();
        let (pairs, odd) = words.as_chunks::<2>();
        for [a, b] in pairs {
            self.block(u64::from_le_bytes(*a), u64::from_le_bytes(*b));
        }
        // The tail is zero-padded, its length (0x80 + len) in the free top byte: "a" and
        // "a\0" differ, and for `str` no tail word can equal a full word (0x8N would be a
        // continuation byte right after a complete character).
        let last = (!tail.is_empty()).then(|| tail_word(tail) | (0x80 + tail.len() as u64) << 56);
        match (odd.first(), last) {
            (Some(w), Some(t)) => self.block(u64::from_le_bytes(*w), t),
            (Some(w), None) => self.0 = fold(self.0 ^ u64::from_le_bytes(*w), K),
            (None, Some(t)) => self.0 = fold(self.0 ^ t, K),
            (None, None) => {}
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0 = self.0.rotate_left(8) ^ u64::from(i);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.write_u64(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0 ^ i).wrapping_mul(K);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    /// One step written out: 128-bit product, high half folded into the low.
    fn step(a: u64, b: u64) -> u64 {
        let p = a as u128 * b as u128;
        (p >> 64) as u64 ^ p as u64
    }

    fn write(bytes: &[u8]) -> u64 {
        let mut h = FastHasher::default();
        h.write(bytes);
        h.finish()
    }

    fn le(b: &[u8]) -> u64 {
        u64::from_le_bytes(b.try_into().unwrap())
    }

    fn hash<T: Hash + ?Sized>(v: &T) -> u64 {
        BuildHasherDefault::<FastHasher>::default().hash_one(v)
    }

    /// 10k keys shaped like flattened locale files.
    fn keys() -> Vec<String> {
        let sections = [
            "nav",
            "pages.home",
            "pages.about.team",
            "form",
            "errors",
            "a",
        ];
        let leaves = ["title", "item_", "label", "x", "description_long_text_"];
        let mut out = Vec::new();
        for i in 0..10_000 {
            let s = sections[i % sections.len()];
            let l = leaves[(i / sections.len()) % leaves.len()];
            out.push(format!("{s}.{l}{i}"));
        }
        out
    }

    /// Scenario: a map of 10k real-shaped keys spreads evenly over the low bits (bucket
    /// index) and the top 7 bits (hashbrown's control byte), with no 64-bit collisions.
    #[test]
    fn distribution_over_realistic_keys() {
        let keys = keys();
        let hashes: Vec<u64> = keys.iter().map(|k| hash(k.as_str())).collect();

        let mut uniq = hashes.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), keys.len(), "64-bit collisions");

        // 1024 buckets by the low bits: ~9.8 keys each (Poisson: max ~25 is already rare).
        let mut low = [0u32; 1024];
        for h in &hashes {
            low[(h & 1023) as usize] += 1;
        }
        let (min, max) = (low.iter().min().unwrap(), low.iter().max().unwrap());
        assert!(*min >= 1 && *max <= 30, "low-bit buckets {min}..{max}");

        // 128 control-byte values: ~78 each.
        let mut top = [0u32; 128];
        for h in &hashes {
            top[(h >> 57) as usize] += 1;
        }
        let (min, max) = (top.iter().min().unwrap(), top.iter().max().unwrap());
        assert!(*min >= 40 && *max <= 120, "top-7-bit buckets {min}..{max}");

        // And the map built on it finds every key.
        let map: FastMap<&str, usize> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(map.get(k.as_str()), Some(&i));
        }
        assert_eq!(map.get("nav.missing"), None);
    }

    /// Scenario: the hash is a pure function of the input (no seed), and every byte,
    /// word and tail position matters.
    #[test]
    fn tail_word_is_the_zero_padded_bytes() {
        let bytes = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        for n in 1..=7 {
            let mut b = [0u8; 8];
            b[..n].copy_from_slice(&bytes[..n]);
            assert_eq!(tail_word(&bytes[..n]), u64::from_le_bytes(b), "{n} bytes");
        }
    }

    #[test]
    fn deterministic_and_input_sensitive() {
        assert_eq!(hash("nav.title"), hash("nav.title"));
        assert_eq!(hash(&String::from("nav.title")), hash("nav.title"));
        // Fixed values, one per path of `write`: a change of the algorithm shows here.
        assert_eq!(write(b""), SEED);
        assert_eq!(write(b"a"), step(SEED ^ 0x8100_0000_0000_0061, K));
        assert_eq!(write(b"abcdefgh"), step(SEED ^ le(b"abcdefgh"), K));
        let t9 = le(b"i\0\0\0\0\0\0\x81");
        assert_eq!(write(b"abcdefghi"), step(SEED ^ le(b"abcdefgh"), t9 ^ K2));
        let s16 = step(SEED ^ le(b"abcdefgh"), le(b"ijklmnop") ^ K2);
        assert_eq!(write(b"abcdefghijklmnop"), s16);
        let t2 = le(b"qr\0\0\0\0\0\x82");
        assert_eq!(write(b"abcdefghijklmnopqr"), step(s16 ^ t2, K));
        assert_eq!(
            write(b"abcdefghijklmnopqrstuvwx"),
            step(s16 ^ le(b"qrstuvwx"), K)
        );
        let t7 = le(b"yz01234\x87");
        assert_eq!(
            write(b"abcdefghijklmnopqrstuvwxyz01234"),
            step(s16 ^ le(b"qrstuvwx"), t7 ^ K2)
        );
        // Two blocks: the loop folds each.
        let s32 = step(s16 ^ le(b"qrstuvwx"), le(b"yz012345") ^ K2);
        assert_eq!(write(b"abcdefghijklmnopqrstuvwxyz012345"), s32);

        let base = "pages.home.title_12";
        let mut seen = vec![hash(base)];
        for i in 0..base.len() {
            let mut b = base.as_bytes().to_vec();
            b[i] ^= 1;
            seen.push(hash(std::str::from_utf8(&b).unwrap()));
        }
        for len in 0..base.len() {
            seen.push(hash(&base[..len]));
        }
        seen.push(hash("a\0"));
        seen.push(hash("a"));
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n, "a one-byte change or a prefix collided");

        let int = |f: &dyn Fn(&mut FastHasher)| {
            let mut h = FastHasher::default();
            f(&mut h);
            h.finish()
        };
        let x = 0x0123_4567_89ab_cdefu64;
        let w = int(&|h| h.write_u64(x));
        assert_eq!(w, (SEED ^ x).wrapping_mul(K));
        // `usize` is 32 bits on wasm32: the truncated value, widened.
        assert_eq!(
            int(&|h| h.write_usize(x as usize)),
            int(&|h| h.write_u64(x as usize as u64))
        );
        assert_eq!(int(&|h| h.write_u32(7)), (SEED ^ 7).wrapping_mul(K));
        assert_eq!(int(&|h| h.write_u8(7)), SEED.rotate_left(8) ^ 7);
        // `str`: the bytes, then the 0xff terminator.
        assert_eq!(hash("ab"), write(b"ab").rotate_left(8) ^ 0xff);
        assert_ne!(int(&|h| h.write_u32(7)), int(&|h| h.write_u32(8)));
        assert_ne!(
            int(&|h| h.write_u8(0)),
            int(&|h| {
                h.write_u8(0);
                h.write_u8(0)
            })
        );
        // The fold order matters.
        assert_ne!(
            int(&|h| {
                h.write_u8(1);
                h.write_u8(2)
            }),
            int(&|h| {
                h.write_u8(2);
                h.write_u8(1)
            })
        );
    }
}
