//! Linked messages (DESIGN 1.1, M3): `@:key`, `@.upper:key` / `@.lower:key` /
//! `@.capitalize:key`, `@:{'key'}`, resolved once at `build` (opt-in,
//! [`Builder::linked_messages`](crate::Builder::linked_messages)), so a lookup reads plain text.
//!
//! Semantics are those of `@i18n-micro/utils` `resolveLinkedMessages` as the Nuxt module applies
//! it (`mergeSourceTranslations`): each `(locale, page)` view is resolved as a whole, a link
//! naming any key that view reads, its root and its fallback locales included, and the case
//! modifiers go by the view's locale (its `iso`, else its code). Per message:
//!
//! - a bare key runs to JS whitespace or one of `{ } @ % | ( )`, dots included, so
//!   `@:brand.` names `brand.`; `@:{'brand'}.` is the way to end a sentence;
//! - a link resolves to the target's text, its own links resolved first; a number is its JS
//!   spelling; `{name}` placeholders of the target stay and interpolate when the message is read
//!   (the modifiers leave them as they are);
//! - a target that is missing, a subtree, a boolean, in a cycle or itself left as written, or an
//!   unknown modifier leaves the **whole** message as written; `build` reports it once through
//!   `missing(locale, key, page)`, `check()` lists every failing link with its reason.
//!
//! The Rust catalog keeps layers apart (page, root, then the same in each fallback locale),
//! where Nuxt premerges them. A view's result is stored where its lookups find it first:
//!
//! - a message of the view's own chunk is replaced in place;
//! - a message the view reads from a weaker layer, whose result differs in this view (its link
//!   targets are overridden here, or the case rules of this locale differ), gets its result as a
//!   [`Node::Linked`](crate::catalog::Node) text in the view's own chunk: `t` reads it, `has` and
//!   `check()` do not (they see the locale's own keys, as without links). A page chunk is made
//!   for it when the locale has no file for that page.
//!
//! Targets are read the way a lookup reads them (layer by layer: `null` never hides a weaker
//! value, a subtree in a stronger layer hides the same key), not through Nuxt's deep merge; the
//! two differ only for structure conflicts between layers (`null` over a value, a scalar over a
//! subtree, a flat `"a.b"` in one layer beside a nested one in another). A locale with no files
//! reads the texts as resolved for the locale it falls back to. Messages added later
//! (`add_messages`) are not resolved, as in Nuxt (`$mergeTranslations`).
//!
//! Lazy pairs: a lazy or remote pair is not part of the pass above; it resolves its own
//! messages when it loads ([`lazy_contexts`]) over the other layers of its view (its locale's root,
//! read first if needed; eager layers; other locales' lazy layers if loaded already). Other views'
//! messages are not re-resolved for it, and its broken links are not reported.
//!
//! Known difference, not copied: `valueAt` reads JS own properties, so `@:list.length` is the
//! length of the array `list`; a lookup has no `length` (`fixtures/compat/README.md`).

mod case;
#[rustfmt::skip]
#[allow(clippy::unreadable_literal, reason = "generated tables")]
mod case_data;

use crate::catalog::{AtString, Chunk, Entry, Msg, hole_ranges, is_js_space};
use crate::check::LinkError;
use crate::hash::FastMap;
use crate::i18n::LocaleData;
use crate::lazy::{Layer, LinkCtx, LinkLayer};
use case::Lang;
use std::sync::Arc;

/// `(locale, page)`; `None` is the root chunk.
pub(crate) type ChunkKey = (Arc<str>, Option<Arc<str>>);

/// Every chunk of the catalog with its `(locale, page)`, sorted by it.
pub(crate) type Chunks = Vec<(ChunkKey, Arc<Chunk>)>;

/// The builder facts resolution needs (`i18n.rs` `Meta`).
pub(crate) struct Setup<'a> {
    pub(crate) links: &'a FastMap<Arc<str>, Arc<str>>,
    pub(crate) fallback: Option<&'a str>,
    pub(crate) iso: &'a FastMap<Arc<str>, Arc<str>>,
}

/// A link that left its message as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Broken {
    pub(crate) locale: Arc<str>,
    pub(crate) page: Option<Arc<str>>,
    /// The message left as written.
    pub(crate) key: Arc<str>,
    /// The link as written (`@.upper:brand`).
    pub(crate) link: Box<str>,
    pub(crate) reason: LinkError,
}

/// One link failure inside a message.
#[derive(Debug, Clone)]
struct Fail {
    link: Box<str>,
    reason: LinkError,
}

/// What a message came to in one view.
#[derive(Debug, Clone)]
enum Outcome {
    Text(Arc<str>),
    Missing,
    Broken(Vec<Fail>),
}

/// What a link gets from its target.
enum Got {
    Text(Arc<str>),
    Missing,
    Cycle,
    Broken,
}

/// A link as written: `@` `(.modifier)?` `:` `({'literal'}|bare)`.
struct Link<'a> {
    len: usize,
    modifier: Option<&'a str>,
    key: &'a str,
}

/// The `LINK` pattern of `linked-messages` at the start of `s` (which starts with `@`).
fn parse_link(s: &str) -> Option<Link<'_>> {
    let rest = s.strip_prefix('@')?;
    let (modifier, rest) = match rest.strip_prefix('.') {
        Some(r) => {
            let n = r.bytes().take_while(u8::is_ascii_alphabetic).count();
            let name = r.get(..n).filter(|m| !m.is_empty())?;
            (Some(name), r.get(n..)?.strip_prefix(':')?)
        }
        None => (None, rest.strip_prefix(':')?),
    };
    let head = s.len() - rest.len();
    let (key, used) = match rest.strip_prefix("{'") {
        Some(r) => {
            let end = r.find('\'')?;
            r.get(end + 1..)?.strip_prefix('}')?;
            (r.get(..end)?, end + 4)
        }
        None => {
            let end = rest
                .find(|c: char| {
                    is_js_space(c) || matches!(c, '{' | '}' | '@' | '%' | '|' | '(' | ')')
                })
                .unwrap_or(rest.len());
            (rest.get(..end).filter(|k| !k.is_empty())?, end)
        }
    };
    Some(Link {
        len: head + used,
        modifier,
        key,
    })
}

/// A modifier on the text between placeholders: `Hello {name}` → `HELLO {name}`.
fn modify(text: &str, modifier: &str, lang: Lang, out: &mut String) {
    let mut at = 0;
    let mut first = true;
    let mut part = |part: &str, out: &mut String| {
        match modifier {
            "upper" => case::upper(part, lang, out),
            "lower" => case::lower(part, lang, out),
            // the first character of the first text part only: a leading `{name}` stays
            _ if first => {
                let n = part.chars().next().map_or(0, char::len_utf8);
                case::upper(part.get(..n).unwrap_or(""), lang, out);
                out.push_str(part.get(n..).unwrap_or(""));
            }
            _ => out.push_str(part),
        }
        first = false;
    };
    for (s, e) in hole_ranges(text) {
        part(text.get(at..s - 1).unwrap_or(""), out);
        out.push_str(text.get(s - 1..e + 1).unwrap_or(""));
        at = e + 1;
    }
    part(text.get(at..).unwrap_or(""), out);
}

const MODIFIERS: [&str; 3] = ["upper", "lower", "capitalize"];

/// One view's resolution: its layers, strongest first, and the case rules of its locale.
struct Resolver<'a> {
    layers: Vec<&'a Chunk>,
    lang: Lang,
    done: FastMap<Box<str>, Outcome>,
    active: Vec<Box<str>>,
}

impl<'a> Resolver<'a> {
    fn new(layers: Vec<&'a Chunk>, lang: Lang) -> Self {
        Resolver {
            layers,
            lang,
            done: FastMap::default(),
            active: Vec::new(),
        }
    }

    /// The text a link reads at `key`: a message, or a number in its JS spelling.
    fn text_at(&self, key: &str) -> Option<Arc<str>> {
        match self.layers.iter().find_map(|c| c.link_target(key))? {
            Entry::Value(v) if &**v == "true" || &**v == "false" => None,
            e => e.text().cloned(),
        }
    }

    /// `target` of `linked-messages`: each key is resolved once per view.
    fn target(&mut self, key: &str) -> Got {
        if self.active.iter().any(|k| &**k == key) {
            return Got::Cycle;
        }
        if !self.done.contains_key(key) {
            let outcome = match self.text_at(key) {
                None => Outcome::Missing,
                Some(text) => match self.message(key, &text) {
                    Ok(s) => Outcome::Text(s),
                    Err(f) => Outcome::Broken(f),
                },
            };
            self.done.insert(key.into(), outcome);
        }
        match self.done.get(key) {
            Some(Outcome::Text(s)) => Got::Text(s.clone()),
            Some(Outcome::Missing) | None => Got::Missing,
            Some(Outcome::Broken(_)) => Got::Broken,
        }
    }

    /// The outcome of the message at `key` as a lookup reads it.
    fn outcome(&mut self, key: &str) -> Outcome {
        self.target(key);
        self.done.get(key).cloned().unwrap_or(Outcome::Missing)
    }

    /// `text` with every link resolved, or every link that failed.
    fn message(&mut self, key: &str, text: &Arc<str>) -> Result<Arc<str>, Vec<Fail>> {
        if !text.contains('@') {
            return Ok(text.clone());
        }
        self.active.push(key.into());
        let mut out = String::with_capacity(text.len());
        let mut fails = Vec::new();
        let (mut copied, mut scan) = (0, 0);
        while let Some(off) = text.get(scan..).and_then(|t| t.find('@')) {
            let start = scan + off;
            let Some(link) = text.get(start..).and_then(parse_link) else {
                scan = start + 1;
                continue;
            };
            let written = text.get(start..start + link.len).unwrap_or("");
            out.push_str(text.get(copied..start).unwrap_or(""));
            copied = start + link.len;
            scan = copied;
            let modifier = match link.modifier {
                Some(m) if !MODIFIERS.contains(&m) => {
                    fails.push(Fail {
                        link: written.into(),
                        reason: LinkError::UnknownModifier,
                    });
                    out.push_str(written);
                    continue;
                }
                m => m,
            };
            let reason = match self.target(link.key) {
                Got::Text(t) => {
                    match modifier {
                        Some(m) => modify(&t, m, self.lang, &mut out),
                        None => out.push_str(&t),
                    }
                    continue;
                }
                Got::Missing => LinkError::Missing,
                Got::Cycle => LinkError::Cycle,
                Got::Broken => LinkError::Broken,
            };
            fails.push(Fail {
                link: written.into(),
                reason,
            });
            out.push_str(written);
        }
        out.push_str(text.get(copied..).unwrap_or(""));
        self.active.pop();
        if fails.is_empty() {
            Ok(out.into())
        } else {
            Err(fails)
        }
    }
}

/// The locales a view of `code` reads, strongest first (`i18n.rs` `Meta::chain`: the locale,
/// its `fallback_for` links to their end, then the global fallback alone).
fn chain<'a>(setup: &'a Setup<'_>, code: &'a str) -> Vec<&'a str> {
    let mut codes: Vec<&str> = Vec::new();
    let mut next = Some(code);
    while let Some(c) = next {
        if codes.contains(&c) {
            break;
        }
        codes.push(c);
        next = setup.links.get(c).map(|s| &**s);
    }
    if let Some(fb) = setup.fallback
        && !codes.contains(&fb)
    {
        codes.push(fb);
    }
    codes
}

fn lang(setup: &Setup<'_>, code: &str) -> Lang {
    let tag = setup.iso.get(code).map_or(code, |t| &**t);
    // an empty iso is none: the code (`normalizeConfiguredLocales`)
    Lang::of(Some(if tag.is_empty() { code } else { tag }))
}

fn final_text(outcome: &Outcome, raw: &Arc<str>) -> Arc<str> {
    match outcome {
        Outcome::Text(s) => s.clone(),
        _ => raw.clone(),
    }
}

/// What a lookup answers at a key, without building an [`Entry`]: a message's text, or an
/// entry of a chunk as stored.
#[derive(Clone)]
enum Answer<'c> {
    Text(Arc<str>),
    Stored(&'c Entry),
}

impl Answer<'_> {
    /// Kind (message, value, subtree) and text: what a lookup returns.
    fn shape(&self) -> (u8, Option<&str>) {
        match self {
            Answer::Text(t) => (0, Some(t)),
            Answer::Stored(e @ Entry::Msg(_)) => (0, e.text().map(|t| &**t)),
            Answer::Stored(e @ Entry::Value(_)) => (1, e.text().map(|t| &**t)),
            Answer::Stored(Entry::Node) => (2, None),
        }
    }

    fn entry(&self) -> Entry {
        match self {
            Answer::Text(t) => Entry::Msg(Msg::new(t)),
            Answer::Stored(e) => (*e).clone(),
        }
    }
}

fn report(out: &mut Vec<Broken>, view: &ChunkKey, key: &Arc<str>, outcome: &Outcome) {
    if let Outcome::Broken(fails) = outcome {
        out.extend(fails.iter().map(|f| Broken {
            locale: view.0.clone(),
            page: view.1.clone(),
            key: key.clone(),
            link: f.link.clone(),
            reason: f.reason,
        }));
    }
}

/// The links written in `text`.
fn links(text: &str) -> impl Iterator<Item = Link<'_>> {
    let mut scan = 0;
    std::iter::from_fn(move || {
        while let Some(off) = text.get(scan..).and_then(|t| t.find('@')) {
            let start = scan + off;
            match text.get(start..).and_then(parse_link) {
                Some(link) => {
                    scan = start + link.len;
                    return Some(link);
                }
                None => scan = start + 1,
            }
        }
        None
    })
}

/// Every key a message reaches through its links, transitively, with the view (by index)
/// whose chunk answers it in the view it was resolved in (`None`: no chunk does).
type Deps = Vec<(Box<str>, Option<usize>)>;

/// A chunk's own message with an `@`, resolved in the chunk's own view.
struct OwnMsg {
    s: AtString,
    outcome: Outcome,
    deps: Deps,
}

/// Every `(locale, page)` a lookup can be made in: locales with chunks × (root + pages with
/// chunks), each with its chunk if it has one and the views its lookups go through.
struct Views<'c> {
    keys: Vec<ChunkKey>,
    chunk: Vec<Option<&'c Chunk>>,
    /// Strongest first, every view of the chain (with a chunk or not).
    path: Vec<Vec<usize>>,
}

impl<'c> Views<'c> {
    fn new(chunks: &'c Chunks, setup: &Setup<'_>) -> Views<'c> {
        let mut locales: Vec<&Arc<str>> = chunks.iter().map(|((l, _), _)| l).collect();
        locales.dedup();
        let mut pages: Vec<&Arc<str>> =
            chunks.iter().filter_map(|((_, p), _)| p.as_ref()).collect();
        pages.sort_unstable();
        pages.dedup();
        let width = pages.len() + 1;
        let index = |locale: &str, page: Option<&str>| -> Option<usize> {
            let l = locales.binary_search_by(|x| (***x).cmp(locale)).ok()?;
            let p = match page {
                None => 0,
                Some(p) => pages.binary_search_by(|x| (***x).cmp(p)).ok()? + 1,
            };
            Some(l * width + p)
        };
        let mut keys = Vec::with_capacity(locales.len() * width);
        for l in &locales {
            keys.push(((*l).clone(), None));
            keys.extend(pages.iter().map(|p| ((*l).clone(), Some((*p).clone()))));
        }
        let mut chunk = vec![None; keys.len()];
        for ((l, p), c) in chunks {
            if let Some(v) = index(l, p.as_deref()) {
                chunk[v] = Some(&**c);
            }
        }
        let path = keys
            .iter()
            .map(|(l, p)| {
                let mut out = Vec::new();
                for code in chain(setup, l) {
                    out.extend(p.iter().filter_map(|p| index(code, Some(p))));
                    out.extend(index(code, None));
                }
                out
            })
            .collect();
        Views { keys, chunk, path }
    }

    /// The chunks a lookup in view `v` reads, strongest first, with their views.
    fn layers(&self, v: usize) -> Vec<(usize, &'c Chunk)> {
        self.path[v]
            .iter()
            .filter_map(|&w| self.chunk[w].map(|c| (w, c)))
            .collect()
    }
}

/// [`Deps`] of `key` in a view with the layers `found`.
fn reach(found: &[(usize, &Chunk)], key: &str) -> Deps {
    // ponytail: a linear `seen` list, quadratic in the keys one message reaches; links chain
    // a handful of keys, a map if a catalog ever links hundreds deep.
    let mut seen: Deps = Vec::new();
    let mut todo: Vec<Box<str>> = vec![key.into()];
    while let Some(k) = todo.pop() {
        if seen.iter().any(|(s, _)| *s == k) {
            continue;
        }
        let hit = found
            .iter()
            .find_map(|&(v, c)| c.link_target(&k).map(|e| (v, e)));
        if let Some(text) = hit.and_then(|(_, e)| e.text()) {
            todo.extend(links(text).map(|l| Box::from(l.key)));
        }
        seen.push((k, hit.map(|(v, _)| v)));
    }
    seen
}

/// Pass 1: each chunk's own messages with an `@`, resolved in the chunk's own view; per view,
/// sorted by key (hidden walk paths last, never looked up).
fn own_messages(views: &Views<'_>, setup: &Setup<'_>) -> Vec<Vec<OwnMsg>> {
    (0..views.keys.len())
        .map(|v| {
            let Some(chunk) = views.chunk[v] else {
                return Vec::new();
            };
            let mut strings = chunk.strings_with_at();
            if strings.is_empty() {
                return Vec::new();
            }
            // key order: which message of a cycle is reported as the cycle does not depend on
            // the map's order
            strings.sort_unstable_by(|a, b| (a.hidden, &a.key).cmp(&(b.hidden, &b.key)));
            let found = views.layers(v);
            let mut r = Resolver::new(
                found.iter().map(|&(_, c)| c).collect(),
                lang(setup, &views.keys[v].0),
            );
            strings
                .into_iter()
                .map(|s| {
                    if s.hidden {
                        let outcome = match r.message(&s.key, &s.raw) {
                            Ok(t) => Outcome::Text(t),
                            Err(f) => Outcome::Broken(f),
                        };
                        OwnMsg {
                            s,
                            outcome,
                            deps: Deps::new(),
                        }
                    } else {
                        let outcome = r.outcome(&s.key);
                        let deps = reach(&found, &s.key);
                        OwnMsg { s, outcome, deps }
                    }
                })
                .collect()
        })
        .collect()
}

/// The own linked message of view `v` at `key`, if any.
fn own_at<'o>(own: &'o [Vec<OwnMsg>], v: usize, key: &str) -> Option<&'o OwnMsg> {
    let list = own.get(v)?;
    let visible = list.partition_point(|m| !m.s.hidden);
    let at = list
        .get(..visible)?
        .binary_search_by(|m| (*m.s.key).cmp(key))
        .ok()?;
    list.get(at)
}

/// Answers pass 2 stored for one view, sorted by key.
type Stored<'c> = Vec<(Box<str>, Answer<'c>)>;

fn stored_at<'s, 'c>(list: &'s Stored<'c>, key: &str) -> Option<&'s Answer<'c>> {
    let at = list.binary_search_by(|(k, _)| (**k).cmp(key)).ok()?;
    list.get(at).map(|(_, a)| a)
}

/// Pass 2 over the chunks as loaded (module docs): for each view, the answers to store in
/// its own chunk, and the links that left those answers as written.
fn view_answers<'c>(
    views: &Views<'c>,
    setup: &Setup<'_>,
    own: &[Vec<OwnMsg>],
    broken: &mut Vec<Broken>,
) -> Vec<Stored<'c>> {
    // What a chunk answers at `key` after pass 1.
    let after_pass1 = |w: usize, key: &str| -> Option<Answer<'c>> {
        match own_at(own, w, key) {
            Some(m) => Some(Answer::Text(final_text(&m.outcome, &m.s.raw))),
            None => views.chunk[w]?.link_target(key).map(Answer::Stored),
        }
    };
    let n = views.keys.len();

    // A view reads the wrong answer at a key when the layer its lookup stops at stores
    // another view's: a linked message of a weaker layer, or an answer this pass stored in a
    // weaker chunk for that chunk's view. Round 1 looks at every such key; a later round only
    // at the keys the previous round stored in a chunk of the view's path (nothing else
    // changed for it). Each `(view, key)` is stored once, so the rounds end.
    let mut linked: Vec<Stored<'c>> = (0..n).map(|_| Vec::new()).collect();
    let mut fresh: Vec<Vec<Box<str>>> = (0..n).map(|_| Vec::new()).collect();
    let mut first = true;
    loop {
        let mut added: Vec<(usize, Box<str>, Answer<'c>, Option<Outcome>)> = Vec::new();
        for v in 0..n {
            let path = &views.path[v];
            let raw = views.layers(v);
            let mut keys: Vec<&str> = path
                .iter()
                .filter(|&&w| w != v)
                .flat_map(|&w| fresh[w].iter().map(|k| &**k))
                .collect();
            if first {
                keys.extend(
                    raw.iter().filter(|&&(w, _)| w != v).flat_map(|&(w, _)| {
                        own[w].iter().filter(|m| !m.s.hidden).map(|m| &*m.s.key)
                    }),
                );
            }
            keys.sort_unstable();
            keys.dedup();
            let mut r: Option<Resolver<'_>> = None;
            for key in keys {
                if stored_at(&linked[v], key).is_some() {
                    continue;
                }
                // ponytail: a key only the global fallback's own fallback locales define can
                // reach this view through an answer stored in the global fallback although no
                // layer of this view defines it; a layer cannot answer "missing", so it stays.
                let Some(&(provider, chunk)) =
                    raw.iter().find(|(_, c)| c.link_target(key).is_some())
                else {
                    continue;
                };
                if provider == v {
                    continue;
                }
                let (mine, outcome) = match own_at(own, provider, key) {
                    // The same texts behind every key it reaches, the same case rules: the
                    // provider's own result, no resolution.
                    Some(m)
                        if lang(setup, &views.keys[v].0)
                            == lang(setup, &views.keys[provider].0)
                            && m.deps.iter().all(|(d, at)| {
                                let here = raw.iter().find(|(_, c)| c.link_target(d).is_some());
                                here.map(|&(w, _)| w) == *at
                            }) =>
                    {
                        (
                            Answer::Text(final_text(&m.outcome, &m.s.raw)),
                            Some(m.outcome.clone()),
                        )
                    }
                    Some(m) => {
                        let r = r.get_or_insert_with(|| {
                            let found = raw.iter().map(|&(_, c)| c).collect();
                            Resolver::new(found, lang(setup, &views.keys[v].0))
                        });
                        let outcome = r.outcome(key);
                        (Answer::Text(final_text(&outcome, &m.s.raw)), Some(outcome))
                    }
                    None => match chunk.link_target(key) {
                        Some(e) => (Answer::Stored(e), None),
                        None => continue,
                    },
                };
                let current = path.iter().find_map(|&w| {
                    stored_at(&linked[w], key)
                        .cloned()
                        .or_else(|| after_pass1(w, key))
                });
                if !current.is_some_and(|c| c.shape() == mine.shape()) {
                    added.push((v, Box::from(key), mine, outcome));
                }
            }
        }
        if added.is_empty() {
            return linked;
        }
        first = false;
        fresh.iter_mut().for_each(Vec::clear);
        for (v, key, answer, outcome) in added {
            if let Some(o) = &outcome {
                report(broken, &views.keys[v], &Arc::from(&*key), o);
            }
            fresh[v].push(key.clone());
            linked[v].push((key, answer));
        }
        for list in &mut linked {
            list.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }
    }
}

/// Resolves every linked message of `chunks` (sorted by key) in place (module docs) and
/// returns the links that left a message as written, in `(locale, page, key)` order.
pub(crate) fn resolve(chunks: &mut Chunks, setup: &Setup<'_>) -> Vec<Broken> {
    let mut broken = Vec::new();
    let (keys, own, linked) = {
        let views = Views::new(chunks, setup);
        let own = own_messages(&views, setup);
        if own.iter().all(Vec::is_empty) {
            return broken;
        }
        let linked: Vec<Vec<(Box<str>, Entry)>> = view_answers(&views, setup, &own, &mut broken)
            .into_iter()
            .map(|list| list.into_iter().map(|(k, a)| (k, a.entry())).collect())
            .collect();
        (views.keys, own, linked)
    };

    // Apply: own messages in place, then the views' answers (a new chunk where a view had none).
    for (v, (done, answers)) in own.into_iter().zip(linked).enumerate() {
        if done.is_empty() && answers.is_empty() {
            continue;
        }
        let key = &keys[v];
        let at = match chunks.binary_search_by(|(k, _)| k.cmp(key)) {
            Ok(at) => at,
            Err(at) => {
                chunks.insert(at, (key.clone(), Arc::default()));
                at
            }
        };
        let Some((_, chunk)) = chunks.get_mut(at) else {
            continue;
        };
        let chunk = Arc::make_mut(chunk);
        for m in done {
            match &m.outcome {
                Outcome::Text(t) if *t != m.s.raw => chunk.set_message(&m.s.key, m.s.own, t),
                Outcome::Broken(_) if !m.s.hidden => report(&mut broken, key, &m.s.key, &m.outcome),
                _ => {}
            }
        }
        for (k, entry) in answers {
            chunk.insert_linked(&k, entry);
        }
    }
    broken.sort_unstable_by(|a, b| {
        (&a.locale, &a.page, &a.key, &a.link).cmp(&(&b.locale, &b.page, &b.key, &b.link))
    });
    broken
}

// ---- lazy and remote pairs (`lazy.rs`) ----

/// Gives every lazy or remote pair of `data` its [`LinkCtx`]: on load it resolves its own
/// linked messages over the other layers of its view (module docs, "Lazy pairs").
pub(crate) fn lazy_contexts(data: &FastMap<Arc<str>, LocaleData>, setup: &Setup<'_>) {
    for (code, d) in data {
        let pairs = std::iter::once((None, d.root.as_ref()))
            .chain(d.pages.iter().map(|(p, l)| (Some(p), Some(l))));
        for (page, layer) in pairs {
            let Some(Layer::Lazy(cell)) = layer else {
                continue;
            };
            let mut layers = Vec::new();
            for c in chain(setup, code) {
                let Some(other) = data.get(c) else { continue };
                let wanted = page.map(|p| Some(&**p)).into_iter().chain([None]);
                for p in wanted {
                    if c == &**code && p == page.map(|p| &**p) {
                        continue;
                    }
                    match other.layer(p) {
                        Some(Layer::Ready(chunk)) => layers.push(LinkLayer::Ready(chunk.clone())),
                        Some(Layer::Lazy(l)) => {
                            let may_load = c == &**code && p.is_none();
                            layers.push(LinkLayer::Lazy(Arc::downgrade(l), may_load));
                        }
                        None => {}
                    }
                }
            }
            let tag = setup
                .iso
                .get(code)
                .filter(|t| !t.is_empty())
                .unwrap_or(code);
            let _ = cell.linked.set(Arc::new(LinkCtx {
                layers,
                tag: tag.clone(),
                resolve: resolve_tree,
            }));
        }
    }
}

/// Resolves the own linked messages of a lazy pair's `tree` as it loads, over `others`, the
/// other layers of its view (strongest first), by the case rules of `tag`.
fn resolve_tree(tree: &mut Chunk, others: &[&Chunk], tag: &str) {
    let mut strings = tree.strings_with_at();
    if strings.is_empty() {
        return;
    }
    strings.sort_unstable_by(|a, b| (a.hidden, &a.key).cmp(&(b.hidden, &b.key)));
    let done: Vec<(AtString, Option<Arc<str>>)> = {
        let layers = std::iter::once(&*tree)
            .chain(others.iter().copied())
            .collect();
        let mut r = Resolver::new(layers, Lang::of(Some(tag)));
        strings
            .into_iter()
            .map(|s| {
                let text = if s.hidden {
                    r.message(&s.key, &s.raw).ok()
                } else {
                    match r.outcome(&s.key) {
                        Outcome::Text(t) => Some(t),
                        _ => None,
                    }
                };
                (s, text)
            })
            .collect()
    };
    for (s, text) in done {
        if let Some(t) = text.filter(|t| *t != s.raw) {
            tree.set_message(&s.key, s.own, &t);
        }
    }
}
