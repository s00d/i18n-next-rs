//! Lazy and remote chunks (DESIGN 3.3) and page eviction (8b.16).
//!
//! A snapshot layer is either a ready chunk or a [`LazyChunk`]: the files of one
//! `(locale, page)` pair, read on first use. The parsed chunk lives in a `OnceLock` inside
//! the cell, shared by every snapshot that holds the cell, so a lazy load publishes no new
//! snapshot and two threads racing on one pair parse it once (the second waits). A remote
//! file is never read here: `add_file` hands its tree in and fills the cell when the last
//! file of the pair has arrived.
//!
//! Eviction swaps a loaded cell for a fresh empty one in a new snapshot; readers of the
//! old snapshot keep the loaded cell. The LRU stamp is the global [`TICK`] at the last
//! access: one relaxed load and, at most once per tick, one relaxed store.

use crate::catalog::{Chunk, Tree};
use crate::error::Error;
use crate::hash::FastMap;
use crate::trace;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Where the bytes of one lazy file come from.
#[derive(Debug)]
pub(crate) enum Origin {
    /// A file of a `lazy_dir` (its full path).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    Path(std::path::PathBuf),
    /// Embedded bytes of `lazy_files`.
    Bytes(Arc<[u8]>),
    /// A `remote` manifest entry: the app brings the bytes (`add_file`).
    Remote,
}

/// One file of a lazy pair.
#[derive(Debug)]
pub(crate) struct LazyFile {
    /// The path relative to the locales root (`pages/settings/de.json`): the manifest path.
    pub(crate) path: Box<str>,
    /// The path for errors (a `lazy_dir` file: joined to its directory).
    pub(crate) shown: Box<str>,
    pub(crate) origin: Origin,
    /// A root file goes under what came before it (folded pages, as in `build`).
    pub(crate) under: bool,
}

impl LazyFile {
    pub(crate) fn is_remote(&self) -> bool {
        matches!(self.origin, Origin::Remote)
    }

    /// Reads and parses a local file; `None` for a remote one.
    fn read(&self) -> Option<Result<(Tree, usize), Error>> {
        let parse = |bytes: &[u8]| Tree::parse(bytes, &self.shown).map(|t| (t, bytes.len()));
        match &self.origin {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            Origin::Path(full) => Some(
                std::fs::read(full)
                    .map_err(|source| Error::Io {
                        path: String::from(&*self.shown),
                        source,
                    })
                    .and_then(|b| parse(&b)),
            ),
            Origin::Bytes(b) => Some(parse(b)),
            Origin::Remote => None,
        }
    }
}

/// Merges one file's tree into the files part of a chunk (`under`: below what is there).
pub(crate) fn apply(acc: &mut Tree, tree: Tree, under: bool) {
    if under {
        let above = std::mem::replace(acc, tree);
        acc.merge(above);
    } else {
        acc.merge(tree);
    }
}

/// Bumped by every lazy load: the LRU clock (global, so no pointer chase on access).
static TICK: AtomicU64 = AtomicU64::new(1);

/// Per instance: the eviction limit, the load counter and the way back to the instance.
pub(crate) struct LazyCtl {
    /// Evictable pages kept loaded; `None`: no eviction.
    pub(crate) limit: Option<usize>,
    /// Successful lazy and remote loads (`Debug` of `I18n` shows it).
    pub(crate) loads: AtomicU64,
    /// Set right after the instance is built: a load past the limit evicts through it.
    pub(crate) owner: OnceLock<Weak<crate::i18n::Inner>>,
    /// Remote fetches in flight, by manifest path (`ensure_async` dedupe).
    pub(crate) flights: Mutex<FastMap<Box<str>, Arc<crate::remote::Flight>>>,
}

impl LazyCtl {
    pub(crate) fn new(limit: Option<usize>) -> LazyCtl {
        LazyCtl {
            limit,
            loads: AtomicU64::new(0),
            owner: OnceLock::new(),
            flights: Mutex::new(FastMap::default()),
        }
    }
}

/// Locks `m`, poisoned or not: every critical section here leaves its data consistent.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The files of one `(locale, page)` pair, parsed on first use.
pub(crate) struct LazyChunk {
    pub(crate) files: Arc<[LazyFile]>,
    /// `extend` layers and eager files of the pair: under the lazy files.
    pub(crate) base: Option<Arc<Tree>>,
    /// `messages` / `add_messages` while the pair was not loaded: over the files, in order.
    pub(crate) over: Arc<[Arc<Tree>]>,
    /// Trees of remote files that arrived, by index in `files`.
    pub(crate) arrived: Mutex<Vec<Option<Arc<Tree>>>>,
    pub(crate) remote: bool,
    /// What a remote pair answers before its files are in: base ⊕ over (library strings
    /// and messages are there at once). `None` for a local pair.
    partial: Option<Arc<Tree>>,
    cell: OnceLock<Tree>,
    stamp: AtomicU64,
    pub(crate) evictable: bool,
    pub(crate) ctl: Arc<LazyCtl>,
    /// The miss path ([`LazyChunk::load`]), by pointer: only [`LazyChunk::new`] names it, and
    /// only the builder's lazy sources reach `new`, so an app without them links none of
    /// the loading code (wasm size).
    miss: fn(&LazyChunk) -> Option<&Tree>,
    /// `linked_messages(true)`: how the pair resolves its own linked messages on load.
    pub(crate) linked: OnceLock<Arc<LinkCtx>>,
}

/// One layer of a lazy pair's view, for its linked messages (M3, `linked.rs`).
pub(crate) enum LinkLayer {
    Ready(Arc<Chunk>),
    /// Weak: views of two locales that fall back to each other would hold each other. `true`:
    /// the same locale's root under a page, which the load may read first (no cycle: a root's
    /// view holds no page); another locale's pair is read only when it is loaded already.
    Lazy(Weak<LazyChunk>, bool),
}

/// What a lazy pair needs to resolve its own linked messages when it loads: the other layers
/// of its view, strongest first, and the tag of its locale. Set at `build` / `reload`.
pub(crate) struct LinkCtx {
    pub(crate) layers: Vec<LinkLayer>,
    pub(crate) tag: Arc<str>,
    pub(crate) resolve: fn(&mut Tree, &[&Chunk], &str),
}

impl LinkCtx {
    fn apply(&self, tree: &mut Tree) {
        // the upgraded cells live as long as the borrowed chunks
        let cells: Vec<Option<Arc<LazyChunk>>> = self
            .layers
            .iter()
            .map(|l| match l {
                LinkLayer::Ready(_) => None,
                LinkLayer::Lazy(w, _) => w.upgrade(),
            })
            .collect();
        let layers: Vec<&Chunk> = self
            .layers
            .iter()
            .zip(&cells)
            .filter_map(|(l, cell)| match (l, cell) {
                (LinkLayer::Ready(c), _) => Some(&**c),
                (LinkLayer::Lazy(_, true), Some(cell)) => cell.get(),
                (LinkLayer::Lazy(_, false), Some(cell)) => cell.loaded(),
                (LinkLayer::Lazy(..), None) => None,
            })
            .collect();
        (self.resolve)(tree, &layers, &self.tag);
    }
}

impl std::fmt::Debug for LazyChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyChunk")
            .field("files", &self.files.len())
            .field("loaded", &self.cell.get().is_some())
            .finish_non_exhaustive()
    }
}

impl LazyChunk {
    pub(crate) fn new(
        files: Arc<[LazyFile]>,
        base: Option<Arc<Tree>>,
        over: Arc<[Arc<Tree>]>,
        evictable: bool,
        ctl: Arc<LazyCtl>,
    ) -> LazyChunk {
        LazyChunk::parts(files, base, over, evictable, ctl, LazyChunk::load)
    }

    fn parts(
        files: Arc<[LazyFile]>,
        base: Option<Arc<Tree>>,
        over: Arc<[Arc<Tree>]>,
        evictable: bool,
        ctl: Arc<LazyCtl>,
        miss: fn(&LazyChunk) -> Option<&Tree>,
    ) -> LazyChunk {
        let remote = files.iter().any(LazyFile::is_remote);
        let partial = match (remote, over.is_empty()) {
            (false, _) => None,
            (true, true) => base.clone(),
            (true, false) => {
                let mut t = base.as_deref().cloned().unwrap_or_default();
                for o in over.iter() {
                    t.merge(Tree::clone(o));
                }
                Some(Arc::new(t))
            }
        };
        LazyChunk {
            partial,
            arrived: Mutex::new(files.iter().map(|_| None).collect()),
            files,
            base,
            over,
            remote,
            cell: OnceLock::new(),
            stamp: AtomicU64::new(0),
            evictable,
            ctl,
            miss,
            linked: OnceLock::new(),
        }
    }

    /// The same link context in a new cell of this pair.
    fn keep_links(&self, next: LazyChunk) -> LazyChunk {
        if let Some(ctx) = self.linked.get() {
            let _ = next.linked.set(ctx.clone());
        }
        next
    }

    /// The same pair, not loaded: what eviction puts in its place.
    pub(crate) fn fresh(&self) -> LazyChunk {
        self.keep_links(LazyChunk::parts(
            self.files.clone(),
            self.base.clone(),
            self.over.clone(),
            self.evictable,
            self.ctl.clone(),
            self.miss,
        ))
    }

    /// The same pair with one more tree over its files (not loaded yet, so the tree waits
    /// for them): arrived remote files carry over. Never evicted: nothing restores it.
    pub(crate) fn with_over(&self, tree: Tree) -> LazyChunk {
        let over: Arc<[Arc<Tree>]> = self
            .over
            .iter()
            .cloned()
            .chain(std::iter::once(Arc::new(tree)))
            .collect();
        let next = LazyChunk::parts(
            self.files.clone(),
            self.base.clone(),
            over,
            false,
            self.ctl.clone(),
            self.miss,
        );
        *lock(&next.arrived) = lock(&self.arrived).clone();
        self.keep_links(next)
    }

    pub(crate) fn loaded(&self) -> Option<&Tree> {
        self.cell.get()
    }

    pub(crate) fn stamp(&self) -> u64 {
        self.stamp.load(Ordering::Relaxed)
    }

    /// The chunk, loaded on first use (local files only). The hit path: one acquire load
    /// of the cell and, for an evictable page, the LRU stamp.
    #[inline]
    pub(crate) fn get(&self) -> Option<&Tree> {
        match self.cell.get() {
            Some(t) => {
                if self.evictable {
                    let now = TICK.load(Ordering::Relaxed);
                    if self.stamp.load(Ordering::Relaxed) != now {
                        self.stamp.store(now, Ordering::Relaxed);
                    }
                }
                Some(t)
            }
            None => (self.miss)(self),
        }
    }

    #[cold]
    #[inline(never)]
    fn load(&self) -> Option<&Tree> {
        // A remote pair is filled by `add_file` only.
        if self.remote {
            return self.partial.as_deref();
        }
        let mut fresh = None;
        let tree = self.cell.get_or_init(|| {
            let (tree, ok) = self.build(Vec::new());
            fresh = Some(ok);
            tree
        });
        if let Some(ok) = fresh
            && self.mark_loaded(ok)
        {
            // Past the limit: evict through the instance, unless a writer is busy (then a
            // later load does it). The writer may be this very thread.
            if let Some(inner) = self.ctl.owner.get().and_then(Weak::upgrade) {
                inner.try_evict();
            }
        }
        Some(tree)
    }

    /// Counts a load (`ok`: every file read) and stamps the cell; whether eviction may be
    /// due.
    fn mark_loaded(&self, ok: bool) -> bool {
        if ok {
            self.ctl.loads.fetch_add(1, Ordering::Relaxed);
        }
        let now = TICK.fetch_add(1, Ordering::Relaxed) + 1;
        self.stamp.store(now, Ordering::Relaxed);
        self.evictable
    }

    /// Fills a remote pair once its last file arrived (`add_file`, under the writer);
    /// `Some(evictable)` when it was filled now.
    pub(crate) fn fill(&self) -> Option<bool> {
        let arrived = {
            let mut arrived = lock(&self.arrived);
            let complete = self
                .files
                .iter()
                .zip(arrived.iter())
                .all(|(f, t)| !f.is_remote() || t.is_some());
            if !complete || self.cell.get().is_some() {
                return None;
            }
            // The trees move into the chunk: no second copy stays behind.
            std::mem::take(&mut *arrived)
        };
        let mut fresh = None;
        self.cell.get_or_init(|| {
            let (tree, ok) = self.build(arrived);
            fresh = Some(ok);
            tree
        });
        fresh.map(|ok| self.mark_loaded(ok))
    }

    /// base ⊕ files (in order, roots under) ⊕ over. A local file that fails is reported
    /// and left out: a lookup has no caller to return the error to. Whether every file
    /// was read.
    fn build(&self, mut arrived: Vec<Option<Arc<Tree>>>) -> (Tree, bool) {
        let mut ok = true;
        let mut files = Tree::default();
        for (i, f) in self.files.iter().enumerate() {
            let timer = trace::Timer::start();
            let tree = match f.read() {
                Some(Ok((tree, bytes))) => {
                    timer.done("load", &f.shown, bytes);
                    tree
                }
                Some(Err(e)) => {
                    trace::broken(&e);
                    ok = false;
                    continue;
                }
                None => match arrived.get_mut(i).and_then(Option::take) {
                    Some(t) => Arc::unwrap_or_clone(t),
                    None => continue,
                },
            };
            apply(&mut files, tree, f.under);
        }
        let mut tree = self.base.as_deref().cloned().unwrap_or_default();
        tree.merge(files);
        for o in self.over.iter() {
            tree.merge(Tree::clone(o));
        }
        if let Some(ctx) = self.linked.get() {
            ctx.apply(&mut tree);
        }
        (tree, ok)
    }
}

/// One chunk slot of a locale: ready, or lazy.
#[derive(Debug, Clone)]
pub(crate) enum Layer {
    Ready(Arc<Chunk>),
    Lazy(Arc<LazyChunk>),
}

impl Layer {
    #[inline]
    pub(crate) fn chunk(&self) -> Option<&Chunk> {
        match self {
            Layer::Ready(c) => Some(c),
            Layer::Lazy(l) => l.get(),
        }
    }
}
