//! The bundled tzdb (feature `tzdb`; always on Windows, which has no TZif files and whose
//! zone API needs `unsafe` FFI). `tzdb.bin` is written by `scripts/tzdb.mjs` from the IANA
//! release named in its first line (with the tarball's sha256): only transitions since
//! 1970 and the POSIX rule after them, so instants before 1970 get the 1970 offset.
//!
//! Layout after the header line, every number a LEB128 varint (signed ones zigzag):
//! `count`, then `count` × (name length, name bytes, blob offset) sorted by name (links
//! point at the blob of their target, identical zones share one); then the blobs, each:
//! rule length, rule bytes (POSIX `TZ`, empty for none), initial offset, type count,
//! the type offsets, transition count, transitions as (seconds since the previous one,
//! from 0; type index).

use super::Table;
use super::posix::Posix;

/// The data compiled into the binary.
#[cfg(any(feature = "tzdb", windows))]
pub(crate) static DATA: &[u8] = include_bytes!("tzdb.bin");

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn uint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self.d.get(self.pos)?;
            self.pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Some(v);
            }
        }
        None
    }

    fn int(&mut self) -> Option<i64> {
        let v = self.uint()?;
        Some((v >> 1) as i64 ^ -((v & 1) as i64))
    }

    fn len(&mut self) -> Option<usize> {
        usize::try_from(self.uint()?).ok()
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.d.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(b)
    }
}

/// The header line: `i18n-next tzdb <version> sha256:<hex> <url>`.
pub(crate) fn header(d: &[u8]) -> Option<&str> {
    let n = d.iter().position(|&b| b == b'\n')?;
    std::str::from_utf8(&d[..n]).ok()
}

/// Calls `f(name, blob offset)` for every name until it returns `Some`.
fn scan<'a, T>(d: &'a [u8], mut f: impl FnMut(&'a str, usize) -> Option<T>) -> Option<(T, usize)> {
    let mut r = Reader {
        d,
        pos: header(d)?.len() + 1,
    };
    let count = r.len()?;
    let mut hit = None;
    for _ in 0..count {
        let n = r.len()?;
        let name = std::str::from_utf8(r.bytes(n)?).ok()?;
        let blob = r.len()?;
        if hit.is_none() {
            hit = f(name, blob);
        }
    }
    hit.map(|h| (h, r.pos))
}

/// Every zone name, sorted.
pub(crate) fn names(d: &[u8]) -> Vec<&str> {
    let mut out = Vec::new();
    scan(d, |name, _| {
        out.push(name);
        None::<()>
    });
    out
}

/// The zone `name` (exact, case-sensitive). A linear scan of ~600 names: a cold path,
/// `Zone::named` caches the result.
pub(crate) fn find(d: &[u8], name: &str) -> Option<Table> {
    let (blob, blobs) = scan(d, |n, blob| (n == name).then_some(blob))?;
    let mut r = Reader {
        d,
        pos: blobs.checked_add(blob)?,
    };
    let n = r.len()?;
    let rule = match r.bytes(n)? {
        [] => None,
        s => Some(Posix::parse(std::str::from_utf8(s).ok()?)?),
    };
    let initial = i32::try_from(r.int()?).ok()?;
    let ntypes = r.len()?;
    let mut types = Vec::with_capacity(ntypes.min(256));
    for _ in 0..ntypes {
        types.push(i32::try_from(r.int()?).ok()?);
    }
    let count = r.len()?;
    let mut times = Vec::with_capacity(count.min(4096));
    let mut offsets = Vec::with_capacity(count.min(4096));
    let mut t = 0i64;
    for _ in 0..count {
        t = t.checked_add(i64::try_from(r.uint()?).ok()?)?;
        times.push(t);
        offsets.push(*types.get(r.len()?)?);
    }
    Some(Table {
        times: times.into(),
        offsets: offsets.into(),
        initial,
        rule,
    })
}
