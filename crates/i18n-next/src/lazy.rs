//! Lazy and remote chunks (DESIGN 3.3) and page eviction (8b.16).
//!
//! A snapshot layer is either a ready chunk or a [`LazyChunk`]: the files of one
//! `(locale, page)` pair, read on first use. The parsed chunk lives in a `OnceLock` inside
//! the cell, shared by every snapshot that holds the cell, so a lazy load publishes no new
//! snapshot and two threads racing on one pair parse it once (the second waits). A remote
//! file is never read here: `add_file` / `ensure_async` hand its tree in, and the cell is
//! filled when its view is shown (`remote.rs`), the weakest pair of the view first.
//!
//! With linked messages, a pair resolves its links when it loads, over the other layers of
//! its view found in the current snapshot ([`LinkCtx`]); those are loaded first, before
//! this cell is initialized, so a load never waits on a cell whose loader waits for it.
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

/// Bumped by every lazy load: the LRU clock (global, so no pointer chase on access). A load
/// takes two ticks and stamps the first: a hit after it stamps the second, so a page read
/// again after the latest load is newer than that load (an LRU tie would evict it).
static TICK: AtomicU64 = AtomicU64::new(1);

/// `(locale, page)` of a lazy pair; `None` is the root.
pub(crate) type Pair = (Arc<str>, Option<Arc<str>>);

/// `(locale, page)` of a page (only pages are evicted).
pub(crate) type PagePair = (Arc<str>, Arc<str>);

/// Per instance: the eviction limit, the load counter and the way back to the instance.
pub(crate) struct LazyCtl {
    /// Evictable pages kept loaded; `None`: no eviction.
    pub(crate) limit: Option<usize>,
    /// Successful lazy and remote loads (`Debug` of `I18n` shows it).
    pub(crate) loads: AtomicU64,
    /// Set right after the instance is built: a load past the limit evicts through it, and a
    /// pair resolving its links reads the other layers of its view in the current snapshot.
    pub(crate) owner: OnceLock<Weak<crate::i18n::Inner>>,
    /// Remote fetches in flight, by manifest path (`ensure_async` dedupe).
    pub(crate) flights: Mutex<FastMap<Box<str>, Arc<crate::remote::Flight>>>,
    /// Evictable pages loaded since they were last checked (`evict.rs`): eviction looks at
    /// these instead of every page of the catalog. Entries whose current cell is not loaded
    /// any more (evicted, reloaded) are dropped there.
    pub(crate) loaded: Mutex<FastMap<PagePair, ()>>,
}

impl LazyCtl {
    pub(crate) fn new(limit: Option<usize>) -> LazyCtl {
        LazyCtl {
            limit,
            loads: AtomicU64::new(0),
            owner: OnceLock::new(),
            flights: Mutex::new(FastMap::default()),
            loaded: Mutex::new(FastMap::default()),
        }
    }
}

/// Locks `m`, poisoned or not: every critical section here leaves its data consistent.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The files of one `(locale, page)` pair, parsed on first use.
pub(crate) struct LazyChunk {
    pub(crate) pair: Pair,
    pub(crate) files: Arc<[LazyFile]>,
    /// `extend` layers and eager files of the pair: under the lazy files.
    pub(crate) base: Option<Arc<Tree>>,
    /// The builder's `messages` for the pair: over the files, in order, resolved with them.
    pub(crate) over: Arc<[Arc<Tree>]>,
    /// `add_messages` while a remote pair was not in: over everything, in order, never
    /// resolved (as in Nuxt).
    pub(crate) added: Arc<[Arc<Tree>]>,
    /// Trees of remote files that arrived, by index in `files`.
    pub(crate) arrived: Mutex<Vec<Option<Arc<Tree>>>>,
    pub(crate) remote: bool,
    /// What a remote pair answers before its files are in: base ⊕ over, links resolved, ⊕
    /// added (library strings and messages are there at once); made on first use.
    partial: OnceLock<Option<Arc<Tree>>>,
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

/// What a lazy pair needs to resolve its own linked messages when it loads: the other layers
/// of its view by `(locale, page)`, strongest first, read in the snapshot current at load
/// time (a cell an eviction or `add_messages` replaced is never read), and the tag of its
/// locale. Set at `build` / `reload`.
pub(crate) struct LinkCtx {
    pub(crate) layers: Vec<Pair>,
    pub(crate) tag: Arc<str>,
    pub(crate) resolve: fn(&mut Tree, &[&Chunk], &str),
}

thread_local! {
    /// Lazy pairs this thread is loading the view of (by address): a pair of a view that
    /// leads back to one of them (two locales falling back to each other) is not loaded
    /// for it again.
    #[allow(
        clippy::missing_const_for_thread_local,
        reason = "already `const`: clippy misreads the expansion on targets without native TLS (Android)"
    )]
    static LOADING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Marks a pair as loading its view on this thread until dropped.
struct Loading(usize);

impl Loading {
    fn has(cell: &LazyChunk) -> bool {
        let at = std::ptr::from_ref(cell) as usize;
        LOADING
            .try_with(|l| l.try_borrow().is_ok_and(|l| l.contains(&at)))
            .unwrap_or(false)
    }

    fn push(cell: &LazyChunk) -> Loading {
        let at = std::ptr::from_ref(cell) as usize;
        let _ = LOADING.try_with(|l| {
            if let Ok(mut l) = l.try_borrow_mut() {
                l.push(at);
            }
        });
        Loading(at)
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        let _ = LOADING.try_with(|l| {
            if let Ok(mut l) = l.try_borrow_mut()
                && let Some(i) = l.iter().rposition(|x| *x == self.0)
            {
                l.remove(i);
            }
        });
    }
}

impl LinkCtx {
    /// The other layers of the view in the current snapshot, strongest first.
    fn deps(&self, ctl: &LazyCtl) -> Vec<Layer> {
        let Some(inner) = ctl.owner.get().and_then(Weak::upgrade) else {
            return Vec::new();
        };
        let snap = inner.current();
        self.layers
            .iter()
            .filter_map(|(l, p)| snap.data.get(l)?.layer(p.as_deref()).cloned())
            .collect()
    }

    /// Resolves `tree`'s own links over `deps` as they answer now (a lazy layer that is
    /// not in counts as nothing, a remote one that is not in as its partial view).
    fn apply(&self, tree: &mut Tree, deps: &[Layer]) {
        let layers: Vec<&Chunk> = deps
            .iter()
            .filter_map(|l| match l {
                Layer::Ready(c) => Some(&**c),
                Layer::Lazy(c) => c
                    .loaded()
                    .or_else(|| c.partial.get().and_then(|p| p.as_deref())),
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
        pair: Pair,
        files: Arc<[LazyFile]>,
        base: Option<Arc<Tree>>,
        over: Arc<[Arc<Tree>]>,
        evictable: bool,
        ctl: Arc<LazyCtl>,
    ) -> LazyChunk {
        LazyChunk::parts(
            pair,
            files,
            base,
            over,
            Arc::new([]),
            evictable,
            ctl,
            LazyChunk::load,
        )
    }

    #[allow(clippy::too_many_arguments, reason = "the fields of one cell")]
    fn parts(
        pair: Pair,
        files: Arc<[LazyFile]>,
        base: Option<Arc<Tree>>,
        over: Arc<[Arc<Tree>]>,
        added: Arc<[Arc<Tree>]>,
        evictable: bool,
        ctl: Arc<LazyCtl>,
        miss: fn(&LazyChunk) -> Option<&Tree>,
    ) -> LazyChunk {
        LazyChunk {
            pair,
            remote: files.iter().any(LazyFile::is_remote),
            partial: OnceLock::new(),
            arrived: Mutex::new(files.iter().map(|_| None).collect()),
            files,
            base,
            over,
            added,
            cell: OnceLock::new(),
            stamp: AtomicU64::new(0),
            evictable,
            ctl,
            miss,
            linked: OnceLock::new(),
        }
    }

    /// The same pair, not loaded, with `added` and the link context: arrived remote files
    /// are not carried over.
    fn again(&self, added: Arc<[Arc<Tree>]>, evictable: bool) -> LazyChunk {
        let next = LazyChunk::parts(
            self.pair.clone(),
            self.files.clone(),
            self.base.clone(),
            self.over.clone(),
            added,
            evictable,
            self.ctl.clone(),
            self.miss,
        );
        if let Some(ctx) = self.linked.get() {
            let _ = next.linked.set(ctx.clone());
        }
        next
    }

    /// The same pair, not loaded: what eviction puts in its place.
    pub(crate) fn fresh(&self) -> LazyChunk {
        self.again(self.added.clone(), self.evictable)
    }

    /// The same remote pair, not in yet, with its arrived files: its partial view is made
    /// again on next use (a layer it links into came in).
    pub(crate) fn respawn(&self) -> LazyChunk {
        let next = self.again(self.added.clone(), self.evictable);
        *lock(&next.arrived) = lock(&self.arrived).clone();
        next
    }

    /// Whether its partial view was made (and may link a layer that came in since).
    pub(crate) fn has_partial(&self) -> bool {
        self.partial.get().is_some()
    }

    /// The same pair with one more `add_messages` tree over everything (not loaded yet, so
    /// the tree waits for its files): arrived remote files carry over. Never evicted:
    /// nothing restores it.
    pub(crate) fn with_over(&self, tree: Tree) -> LazyChunk {
        let added: Arc<[Arc<Tree>]> = self
            .added
            .iter()
            .cloned()
            .chain(std::iter::once(Arc::new(tree)))
            .collect();
        let next = self.again(added, false);
        *lock(&next.arrived) = lock(&self.arrived).clone();
        next
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

    /// The other layers of the view, each loaded first (unless this thread is already
    /// loading its view: a fallback cycle), so the links of this pair resolve over the
    /// whole view whatever was read before. No cell is being initialized here, so loading
    /// another pair never waits on one that waits for this one.
    fn prepare(&self) -> Vec<Layer> {
        let Some(ctx) = self.linked.get() else {
            return Vec::new();
        };
        let deps = ctx.deps(&self.ctl);
        let _me = Loading::push(self);
        for d in &deps {
            if let Layer::Lazy(c) = d
                && !Loading::has(c)
            {
                let _ = c.get();
            }
        }
        deps
    }

    #[cold]
    #[inline(never)]
    fn load(&self) -> Option<&Tree> {
        // A remote pair is filled by `add_file` / `ensure_async` only.
        if self.remote {
            return self.partial();
        }
        let deps = self.prepare();
        let mut fresh = None;
        let tree = self.cell.get_or_init(|| {
            let (tree, ok) = self.build(Vec::new(), &deps);
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

    /// A remote pair's answer before its files are in.
    fn partial(&self) -> Option<&Tree> {
        if let Some(p) = self.partial.get() {
            return p.as_deref();
        }
        let deps = self.prepare();
        self.partial
            .get_or_init(|| {
                let ctx = self.linked.get();
                if ctx.is_none() && self.over.is_empty() && self.added.is_empty() {
                    return self.base.clone();
                }
                let mut t = self.base.as_deref().cloned().unwrap_or_default();
                for o in self.over.iter() {
                    t.merge(Tree::clone(o));
                }
                if let Some(ctx) = ctx {
                    ctx.apply(&mut t, &deps);
                }
                for a in self.added.iter() {
                    t.merge(Tree::clone(a));
                }
                Some(Arc::new(t))
            })
            .as_deref()
    }

    /// Counts a load (`ok`: every file read), stamps the cell and lists an evictable page
    /// for eviction; whether eviction may be due.
    fn mark_loaded(&self, ok: bool) -> bool {
        if ok {
            self.ctl.loads.fetch_add(1, Ordering::Relaxed);
        }
        let now = TICK.fetch_add(2, Ordering::Relaxed) + 1;
        self.stamp.store(now, Ordering::Relaxed);
        if self.evictable
            && let (l, Some(p)) = &self.pair
        {
            lock(&self.ctl.loaded).insert((l.clone(), p.clone()), ());
        }
        self.evictable
    }

    /// Whether every remote file arrived and the pair is not filled yet.
    pub(crate) fn complete(&self) -> bool {
        self.cell.get().is_none()
            && self
                .files
                .iter()
                .zip(lock(&self.arrived).iter())
                .all(|(f, t)| !f.is_remote() || t.is_some())
    }

    /// Fills a remote pair once its last file arrived (under the writer); `Some(evictable)`
    /// when it was filled now.
    pub(crate) fn fill(&self) -> Option<bool> {
        if !self.complete() {
            return None;
        }
        // The trees move into the chunk: no second copy stays behind.
        let arrived = std::mem::take(&mut *lock(&self.arrived));
        let deps = self.prepare();
        let mut fresh = None;
        self.cell.get_or_init(|| {
            let (tree, ok) = self.build(arrived, &deps);
            fresh = Some(ok);
            tree
        });
        fresh.map(|ok| self.mark_loaded(ok))
    }

    /// base ⊕ files (in order, roots under) ⊕ over, links resolved over `deps`, ⊕ added. A
    /// local file that fails is reported and left out: a lookup has no caller to return the
    /// error to. Whether every file was read.
    fn build(&self, mut arrived: Vec<Option<Arc<Tree>>>, deps: &[Layer]) -> (Tree, bool) {
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
            ctx.apply(&mut tree, deps);
        }
        for a in self.added.iter() {
            tree.merge(Tree::clone(a));
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
