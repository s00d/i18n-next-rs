//! What the builder was given, and how it becomes the chunks of a snapshot (DESIGN 3.3,
//! 8b.4). The sources are kept after `build`: `reload` runs [`load`] again on them.
//!
//! Layer order of one `(locale, page)` pair, weakest first:
//! 1. `extend` files, call by call (each call in path order);
//! 2. the app's eager files (`dir`, `files`) in path order (JS `<`), a root file under the
//!    page files folded into it;
//! 3. the app's lazy files (`lazy_dir`, `lazy_files`, `remote`), read on first use;
//! 4. `messages`, in call order.

use crate::catalog::Tree;
use crate::error::Error;
use crate::hash::FastMap;
use crate::i18n::{LocaleData, Meta, chunk_names, messages_path};
use crate::lazy::{Layer, LazyChunk, LazyCtl, LazyFile, Origin, apply};
use crate::loader::{self, Kind};
use std::borrow::Cow;
use std::sync::Arc;

/// Shared bytes of an embedded file.
pub(crate) type Bytes = Arc<[u8]>;

/// `(locale, page)`; `None` is the root.
pub(crate) type Pair = (Arc<str>, Option<Arc<str>>);

/// Everything the builder was given that `reload` reads again.
#[derive(Default)]
pub(crate) struct Sources {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(crate) dirs: Vec<std::path::PathBuf>,
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(crate) lazy_dirs: Vec<std::path::PathBuf>,
    pub(crate) files: Vec<(String, Bytes)>,
    pub(crate) lazy_files: Vec<(String, Bytes)>,
    pub(crate) remote: Vec<String>,
    pub(crate) extend: Vec<Vec<(String, Bytes)>>,
    pub(crate) messages: Vec<(String, Option<String>, String)>,
    pub(crate) no_pages: bool,
    pub(crate) evict: Option<usize>,
    pub(crate) pins: Vec<String>,
    /// Set by `lazy_dir` / `lazy_files` / `remote`; see [`LazyHooks`].
    pub(crate) lazy: Option<LazyHooks>,
}

/// Per lazy pair: its files and the messages over them.
pub(crate) type Pending = (Vec<LazyFile>, Vec<Arc<Tree>>);

/// The lazy part of loading, behind pointers that only the lazy builder calls take: an app
/// that never calls them links none of the lazy listing, loading and eviction code (wasm
/// size: ~10 KB of the micro build).
/// Builds the layer of one lazy pair: its files and messages, base, evictable.
type MakeCell = fn(Pending, Option<Arc<Tree>>, bool, &Arc<LazyCtl>) -> Layer;

#[derive(Clone, Copy)]
pub(crate) struct LazyHooks {
    list: fn(&Sources) -> Result<FastMap<Pair, Pending>, Error>,
    cell: MakeCell,
}

pub(crate) const LAZY: LazyHooks = LazyHooks {
    list: list_lazy,
    cell: |(files, over), base, evictable, ctl| {
        let cell = LazyChunk::new(files.into(), base, over.into(), evictable, ctl.clone());
        Layer::Lazy(Arc::new(cell))
    },
};

/// `\` → `/`: the layout uses `/` (Windows paths from `include_dir` use `\`).
pub(crate) fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

/// The pair a file feeds and whether it goes under the files before it (a root file).
fn place(rel: &str, no_pages: bool) -> Option<(Pair, bool)> {
    match loader::classify(rel) {
        Kind::Ignore => None,
        Kind::Root(l) => Some(((Arc::from(l), None), true)),
        Kind::Page(p, l) if no_pages || p == "index" => Some(((Arc::from(l), None), false)),
        Kind::Page(p, l) => Some(((Arc::from(l), Some(Arc::from(p))), false)),
    }
}

/// Eager files `(relative path, bytes, path for errors)` merged per pair in path order
/// (JS `<`, as the JS loader reads them); the index makes the order total.
fn eager(
    files: &[(String, Cow<'_, [u8]>, String)],
    no_pages: bool,
) -> Result<FastMap<Pair, Tree>, Error> {
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_unstable_by(|&a, &b| loader::js_order(&files[a].0, &files[b].0).then(a.cmp(&b)));
    let mut out: FastMap<Pair, Tree> = FastMap::default();
    for i in order {
        let (rel, bytes, shown) = &files[i];
        let Some((pair, under)) = place(rel, no_pages) else {
            continue;
        };
        let timer = crate::trace::Timer::start();
        let tree = Tree::parse(bytes, shown)?;
        timer.done("load", shown, bytes.len());
        apply(out.entry(pair).or_default(), tree, under);
    }
    Ok(out)
}

/// Manifest path → pair, for `add_file` (paths outside the layout are not listed).
pub(crate) fn remote_index(src: &Sources) -> FastMap<Box<str>, Pair> {
    src.remote
        .iter()
        .filter_map(|p| place(p, src.no_pages).map(|(pair, _)| (Box::from(p.as_str()), pair)))
        .collect()
}

/// Builds every chunk from `src`. `meta.page` resolves a page name the app passed (aliases,
/// `index`). Nothing lazy is read here, only listed.
pub(crate) fn load(
    src: &Sources,
    meta: &Meta,
    ctl: &Arc<LazyCtl>,
) -> Result<FastMap<Arc<str>, LocaleData>, Error> {
    let no_pages = src.no_pages;
    let mut acc: FastMap<Pair, Tree> = FastMap::default();
    let add = |acc: &mut FastMap<Pair, Tree>, layer: FastMap<Pair, Tree>| {
        for (pair, tree) in layer {
            acc.entry(pair).or_default().merge(tree);
        }
    };
    // 1. Libraries, call by call.
    for call in &src.extend {
        let files: Vec<_> = call
            .iter()
            .map(|(rel, b)| (rel.clone(), Cow::Borrowed(&b[..]), rel.clone()))
            .collect();
        add(&mut acc, eager(&files, no_pages)?);
    }
    // 2. The app's eager files.
    let mut files: Vec<(String, Cow<'_, [u8]>, String)> = Vec::new();
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    for dir in &src.dirs {
        for (rel, full) in loader::list_dir(dir)? {
            if loader::classify(&rel) == Kind::Ignore {
                continue;
            }
            let shown = full.to_string_lossy().into_owned();
            let bytes = std::fs::read(&full).map_err(|source| Error::Io {
                path: shown.clone(),
                source,
            })?;
            files.push((rel, Cow::Owned(bytes), shown));
        }
    }
    for (rel, b) in &src.files {
        files.push((rel.clone(), Cow::Borrowed(&b[..]), rel.clone()));
    }
    add(&mut acc, eager(&files, no_pages)?);
    drop(files);

    // 3. Lazy files: listed, not read.
    let mut lazy = match src.lazy {
        Some(hooks) => (hooks.list)(src)?,
        None => FastMap::default(),
    };

    // 4. Messages: over the files; a lazy pair keeps them until it is loaded.
    for (locale, page, json) in &src.messages {
        let page = page.as_deref();
        chunk_names(locale, page)?;
        let tree = Tree::parse(json.as_bytes(), &messages_path(locale, page))?;
        let pair: Pair = (Arc::from(locale.as_str()), meta.page(page).map(Arc::from));
        if let Some((_, over)) = lazy.get_mut(&pair) {
            over.push(Arc::new(tree));
        } else {
            acc.entry(pair).or_default().merge(tree);
        }
    }

    // 5. Chunks.
    let pinned = |p: &str| src.pins.iter().any(|x| meta.page(Some(x)) == Some(p));
    let mut data: FastMap<Arc<str>, LocaleData> = FastMap::default();
    let mut put = |(locale, page): Pair, layer: Layer| {
        let slot = data.entry(locale).or_default();
        match page {
            None => slot.root = Some(layer),
            Some(p) => {
                slot.pages.insert(p, layer);
            }
        }
    };
    if let Some(hooks) = src.lazy {
        for (pair, pending) in lazy {
            let evictable = src.evict.is_some()
                && pending.1.is_empty()
                && pair.1.as_deref().is_some_and(|p| !pinned(p));
            let base = acc.remove(&pair).map(Arc::new);
            put(pair, (hooks.cell)(pending, base, evictable, ctl));
        }
    }
    for (pair, tree) in acc {
        put(pair, Layer::Ready(Arc::new(tree)));
    }
    Ok(data)
}

/// Lists the lazy files of `src` per pair, in path order (not read).
fn list_lazy(src: &Sources) -> Result<FastMap<Pair, Pending>, Error> {
    let no_pages = src.no_pages;
    let mut lazy: FastMap<Pair, Pending> = FastMap::default();
    let mut push = |rel: &str, shown: &str, origin: Origin| {
        let Some((pair, under)) = place(rel, no_pages) else {
            return;
        };
        let files = &mut lazy.entry(pair).or_default().0;
        // A remote path listed twice is one file: it arrives once.
        let remote = matches!(origin, Origin::Remote);
        if remote && files.iter().any(|f| f.is_remote() && *f.path == *rel) {
            return;
        }
        // Path order (JS `<`), equal paths in source order. ponytail: an insertion sort, a
        // pair has one file (a few with folded pages); quadratic only for thousands of
        // pages folded into one root, and it keeps the sort code out of the binary.
        let at = files
            .iter()
            .rposition(|f| loader::js_order(&f.path, rel).is_le())
            .map_or(0, |i| i + 1);
        let file = LazyFile {
            path: rel.into(),
            shown: shown.into(),
            origin,
            under,
        };
        files.insert(at, file);
    };
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    for dir in &src.lazy_dirs {
        for (rel, full) in loader::list_dir(dir)? {
            let shown = full.to_string_lossy().into_owned();
            push(&rel, &shown, Origin::Path(full));
        }
    }
    for (rel, b) in &src.lazy_files {
        push(rel, rel, Origin::Bytes(b.clone()));
    }
    for rel in &src.remote {
        push(rel, rel, Origin::Remote);
    }
    Ok(lazy)
}
