//! Chunks: one `(locale, page)` tree read straight from JSON events into dotted paths
//! (DESIGN 2.2, 4.5, spike S6), merged like `@i18n-micro/utils` `deepMergeTranslationsRecursive`,
//! and flattened into the lookup map with the rules of `core/src/tree.ts` `getByPath`:
//!
//! - an own key at the chunk root (`"a.b"`) wins over the dotted walk unless it is `null`;
//! - a nested key with a dot is unreachable (the walk splits on dots);
//! - arrays by index (`list.0`), never `length`; `__proto__` is skipped at every level;
//! - a subtree or array is a node: a value for `has`, not a message;
//! - `null` is nothing; numbers print as JS prints them, booleans as `true` / `false`.
//!
//! The same map serves lookups and later merges (`add_messages`, page folder
//! collisions): nulls and containers stay in it as markers, so a merge knows what an
//! incoming value replaces.

use crate::args::{Arg, Args, Count, MAX_SCALAR_LEN};
use crate::error::Error;
use crate::hash::FastMap;
use crate::json::{Event, Reader, number_to_js_string};
use crate::plural::{Plural, Shape};
use std::sync::Arc;

/// Where placeholder values come from: [`Args`], or `tc`'s args with `{count}` on top.
pub(crate) trait Params {
    /// Appends the value of `name`; `false` when there is none (`{name}` stays).
    fn put(&self, name: &str, out: &mut String) -> bool;
    /// Bytes [`Params::put`] appends for `name`: exact, an upper bound for a float; `None`
    /// when there is no value.
    fn len_of(&self, name: &str) -> Option<usize>;
    /// At least the longest value any name can get.
    fn max_len(&self) -> usize;
}

impl Params for Args<'_> {
    fn put(&self, name: &str, out: &mut String) -> bool {
        match self.find(name) {
            Some(v) => {
                v.with(|v| v.write(out));
                true
            }
            None => false,
        }
    }

    fn len_of(&self, name: &str) -> Option<usize> {
        self.find(name).map(|v| v.with(Arg::len))
    }

    fn max_len(&self) -> usize {
        self.max_value_len()
    }
}

/// `tc` params: JS `{ ...params, count }`, so `{count}` is the count whatever `args` say,
/// and a list gives its items but not `{length}` (a spread copies no `length`).
pub(crate) struct WithCount<'x, 'a> {
    pub(crate) args: Option<&'x Args<'a>>,
    pub(crate) count: Count,
}

impl Params for WithCount<'_, '_> {
    fn put(&self, name: &str, out: &mut String) -> bool {
        if name == "count" {
            self.count.write(out);
            return true;
        }
        match self.args.and_then(|a| a.spread(name)) {
            Some(v) => {
                v.with(|v| v.write(out));
                true
            }
            None => false,
        }
    }

    fn len_of(&self, name: &str) -> Option<usize> {
        if name == "count" {
            return Some(self.count.len());
        }
        self.args
            .and_then(|a| a.spread(name))
            .map(|v| v.with(Arg::len))
    }

    fn max_len(&self) -> usize {
        self.args.map_or(0, Args::max_value_len).max(MAX_SCALAR_LEN)
    }
}

/// A string leaf, tokenized once at load: `{name}` ranges are found here, never on `t`.
#[derive(Debug, Clone)]
pub(crate) struct Template {
    pub(crate) raw: Arc<str>,
    /// Byte ranges of placeholder names (without the braces), in order.
    holes: Box<[(u32, u32)]>,
}

/// A message. Linked messages join in M3.
#[derive(Debug, Clone)]
pub(crate) enum Msg {
    Text(Template),
    /// A string with `|`: `t` reads its text, `tc` one of its forms.
    Plural(Box<PluralMsg>),
}

impl Msg {
    /// A string leaf: plural when it has a `|`.
    pub(crate) fn new(s: &str) -> Msg {
        if s.contains('|') {
            Msg::Plural(Box::new(PluralMsg::new(s)))
        } else {
            Msg::Text(Template::new(s))
        }
    }
}

/// `a | b | c`, split once at load (DESIGN 4.5): `tc` parses nothing. The forms are ranges
/// of the one shared text, so a message of many tiny forms (`"|"` × 10^6) costs 12 bytes a
/// form, not a string each.
#[derive(Debug, Clone)]
pub(crate) struct PluralMsg {
    /// The whole string, for `t` / `get` (JS `t` interpolates the string with its pipes).
    pub(crate) text: Template,
    /// Starts with the `=0 ` marker.
    marked: bool,
    /// The forms, trimmed (JS `trim`): byte ranges of `text.raw`.
    forms: Box<[Form]>,
    /// Placeholders of every form in order, as byte ranges of `text.raw`. Each form is
    /// tokenized alone: `{a|b}` is no placeholder.
    holes: Box<[(u32, u32)]>,
    shape: Shape,
}

/// One trimmed form: `text.raw[start..end]`; its holes end at `holes_end` and start where
/// the previous form's end.
#[derive(Debug, Clone, Copy)]
struct Form {
    start: u32,
    end: u32,
    holes_end: u32,
}

/// A form: its byte range in the raw text and its holes.
pub(crate) type FormRange<'a> = (usize, usize, &'a [(u32, u32)]);

/// The `=0 ` marker of `text.ts`: the first form for 0, the others by category.
const ZERO_MARKER: &str = "=0 ";

impl PluralMsg {
    fn new(raw: &str) -> PluralMsg {
        let marked = raw.starts_with(ZERO_MARKER);
        let skip = if marked { ZERO_MARKER.len() } else { 0 };
        let body = raw.get(skip..).unwrap_or("");
        let mut forms = Vec::with_capacity(body.matches('|').count() + 1);
        let mut holes = Vec::new();
        let mut at = skip;
        for f in body.split('|') {
            let t = f.trim_matches(is_js_space);
            // `t` is a slice of `f`: its offset in `raw`.
            let start = at + (t.as_ptr() as usize - f.as_ptr() as usize);
            holes.extend(Holes::new(t).map(|(s, e)| ((start + s) as u32, (start + e) as u32)));
            forms.push(Form {
                start: start as u32,
                end: (start + t.len()) as u32,
                holes_end: holes.len() as u32,
            });
            at += f.len() + 1;
        }
        PluralMsg {
            text: Template::new(raw),
            marked,
            forms: forms.into_boxed_slice(),
            holes: holes.into_boxed_slice(),
            shape: Shape::new(body.split('|')),
        }
    }

    /// The form `count` picks (`pluralForm`), interpolated with `params`.
    pub(crate) fn pick(
        &self,
        count: Count,
        plural: Option<&Plural>,
        params: &impl Params,
    ) -> String {
        let Some((start, end, holes)) = self.form(count, plural) else {
            return String::new();
        };
        render(
            &self.text.raw,
            start,
            end,
            holes.iter().map(|&(s, e)| (s as usize, e as usize)),
            holes.len(),
            params,
        )
    }

    /// The byte range of the form `count` picks in `text.raw` and its holes (ranges of
    /// `text.raw`), not interpolated: `tc` renders it, `Text` writes it to a formatter.
    #[inline]
    pub(crate) fn form(&self, count: Count, plural: Option<&Plural>) -> Option<FormRange<'_>> {
        let at = self.shape.index(self.marked, count.whole(), plural);
        let f = self.forms.get(at)?;
        let first = at
            .checked_sub(1)
            .and_then(|j| self.forms.get(j))
            .map_or(0, |p| p.holes_end as usize);
        let holes = self.holes.get(first..f.holes_end as usize).unwrap_or(&[]);
        Some((f.start as usize, f.end as usize, holes))
    }
}

/// What a dotted key holds in one chunk.
#[derive(Debug, Clone)]
pub(crate) enum Entry {
    Msg(Msg),
    /// A number or boolean as JS prints it: returned as is, never interpolated.
    Value(Arc<str>),
    /// A subtree or an array: `has` is true, there is no text.
    Node,
}

impl Entry {
    /// The raw text: a message as written, a number or boolean as printed.
    pub(crate) fn text(&self) -> Option<&Arc<str>> {
        match self {
            Entry::Msg(Msg::Text(t)) => Some(&t.raw),
            Entry::Msg(Msg::Plural(p)) => Some(&p.text.raw),
            Entry::Value(v) => Some(v),
            Entry::Node => None,
        }
    }
}

/// JS `\s`: ECMAScript WhiteSpace and LineTerminator.
pub(crate) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// The placeholders of `core/src/text.ts` in a string: `{` + one or more chars that are not
/// `{`, `}` or JS `\s` + `}`, as byte ranges of the name (without the braces).
#[derive(Clone)]
struct Holes<'a> {
    chars: std::str::CharIndices<'a>,
    open: Option<usize>,
}

impl<'a> Holes<'a> {
    fn new(raw: &'a str) -> Self {
        Holes {
            chars: raw.char_indices(),
            open: None,
        }
    }
}

impl Iterator for Holes<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        for (i, c) in self.chars.by_ref() {
            match c {
                '{' => self.open = Some(i + 1),
                '}' => {
                    if let Some(start) = self.open.take()
                        && i > start
                    {
                        return Some((start, i));
                    }
                }
                c if is_js_space(c) => self.open = None,
                _ => {}
            }
        }
        None
    }
}

impl Template {
    /// Finds the placeholders once ([`Holes`]).
    pub(crate) fn new(raw: &str) -> Self {
        Template {
            raw: Arc::from(raw),
            holes: Holes::new(raw).map(|(s, e)| (s as u32, e as u32)).collect(),
        }
    }

    /// The text with every placeholder `args` has replaced, in one allocation.
    pub(crate) fn interpolate(&self, args: &impl Params) -> String {
        let raw = &*self.raw;
        if self.holes.is_empty() {
            return String::from(raw);
        }
        render(raw, 0, raw.len(), self.holes(), self.holes.len(), args)
    }

    fn holes(&self) -> impl Iterator<Item = (usize, usize)> + Clone + '_ {
        self.holes.iter().map(|&(s, e)| (s as usize, e as usize))
    }

    /// The one form of a `tc` on a string without `|`: the `=0 ` marker dropped, trimmed
    /// (JS `trim`), interpolated. Whitespace is never inside a placeholder, so the holes
    /// of the whole string inside the trimmed range are those of the form.
    pub(crate) fn single_form(&self, args: &impl Params) -> String {
        let (start, end) = self.single_range();
        let holes = self.holes().filter(|&(s, e)| s > start && e < end);
        render(&self.raw, start, end, holes, self.holes.len(), args)
    }

    /// The byte range of [`Template::single_form`] in `raw`.
    pub(crate) fn single_range(&self) -> (usize, usize) {
        let raw = &*self.raw;
        let body = raw.strip_prefix(ZERO_MARKER).unwrap_or(raw);
        let trimmed = body.trim_matches(is_js_space);
        let start = trimmed.as_ptr() as usize - raw.as_ptr() as usize;
        (start, start + trimmed.len())
    }

    /// Placeholder name ranges of `raw`, in order.
    pub(crate) fn hole_slice(&self) -> &[(u32, u32)] {
        &self.holes
    }
}

/// Growth up to which [`render`] sizes its result by the cheap bound, holes × longest
/// value: at most this many bytes too big, and every hole is looked up once. Past it a
/// first pass sizes the result exactly (a second lookup per hole), so a long argument the
/// text does not use, or many holes, can neither over-reserve nor overflow: the bound alone
/// reserved 11.5 MB for 11 holes beside an unused 1 MiB argument, and overflowed `usize`
/// on wasm32. (Always sizing exactly cost +5 ns on `t_with` with one argument, a cache of
/// the looked-up values +30 ns.)
const CHEAP: usize = 256;

/// `raw[from..end]` with each hole (name ranges inside it, in order) that `args` has a
/// value for replaced, in one allocation. `max_holes` bounds the number of holes.
fn render(
    raw: &str,
    from: usize,
    end: usize,
    holes: impl Iterator<Item = (usize, usize)> + Clone,
    max_holes: usize,
    args: &impl Params,
) -> String {
    if max_holes == 0 {
        return String::from(raw.get(from..end).unwrap_or(""));
    }
    let text = end.saturating_sub(from);
    let growth = max_holes.saturating_mul(args.max_len());
    let capacity = if growth <= CHEAP {
        text + growth
    } else {
        let (mut removed, mut added) = (0usize, 0usize);
        for (s, e) in holes.clone() {
            if let Some(n) = raw.get(s..e).and_then(|name| args.len_of(name)) {
                // `{name}`: the braces go too.
                removed += e - s + 2;
                added = added.saturating_add(n);
            }
        }
        text.saturating_sub(removed).saturating_add(added)
    };
    let mut out = String::with_capacity(capacity);
    let mut at = from;
    for (s, e) in holes {
        // `.get` instead of indexing: the ranges are char boundaries by construction, and
        // no slice panic (with its `fmt` code) is linked.
        let Some(name) = raw.get(s..e) else {
            continue;
        };
        let mark = out.len();
        out.push_str(raw.get(at..s - 1).unwrap_or(""));
        if args.put(name, &mut out) {
            at = e + 1;
        } else {
            out.truncate(mark);
        }
    }
    out.push_str(raw.get(at..end).unwrap_or(""));
    out
}

/// [`Template::new`] + [`Template::interpolate`] for text used once (the default of
/// `t_with`): no `Template` is built, the text is tokenized on the fly. Every `{` may open
/// a placeholder: that bounds the holes.
pub(crate) fn interpolate_once(raw: &str, args: &impl Params) -> String {
    let max_holes = raw.bytes().filter(|&b| b == b'{').count();
    render(raw, 0, raw.len(), Holes::new(raw), max_holes, args)
}

/// A node of the tree as stored by path.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    /// `kids: false`: nothing is stored under it, so replacing it scans nothing.
    Obj {
        kids: bool,
    },
    Arr {
        kids: bool,
    },
    Null,
    Leaf(Entry),
    /// A linked message resolved for this chunk's view from a weaker layer's text
    /// (`linked.rs`): lookups read it like a leaf, `has` and `check` do not see it.
    Linked(Entry),
}

/// The longest dotted path (`a.b.0.c`) a document may reach, in bytes. Every leaf stores
/// its full path, so a deep prefix over many leaves multiplies the input (a 137 KB file
/// took 2.2 GB); real keys, gettext-style sentence keys included, are far shorter.
const MAX_PATH: usize = 4096;

/// Stored path bytes allowed per input byte, plus [`PATH_SLACK`]: bounds the memory of
/// one document by its size even when every path stays under [`MAX_PATH`].
const PATH_RATIO: usize = 64;
const PATH_SLACK: usize = 1 << 20;

/// The stored path bytes allowed for a document of `len` bytes.
pub(crate) fn path_budget(len: usize) -> usize {
    len.saturating_mul(PATH_RATIO).saturating_add(PATH_SLACK)
}

/// One `(locale, page)` chunk by dotted path. `walk` holds what the dotted walk reaches
/// (keys without dots, array indices); `own` holds root keys with a dot, which only an
/// exact own-key read reaches (their subtrees never are).
#[derive(Debug, Clone, Default)]
pub(crate) struct Tree {
    walk: FastMap<Arc<str>, Node>,
    own: FastMap<Arc<str>, Node>,
    /// The payload JSON of this tree and its content tag, written once on first use
    /// (8b.4): a loaded chunk never changes, and every `&mut self` method clears it.
    #[cfg(feature = "payload")]
    json: std::sync::OnceLock<(Arc<str>, u64)>,
}

/// A loaded chunk.
pub(crate) type Chunk = Tree;

/// What [`Tree::get`] returns for a subtree or array.
static NODE: Entry = Entry::Node;

/// Where the next value of the document goes.
enum Target<'k> {
    /// A reachable path: the current `path` buffer.
    Walk,
    /// A root key with a dot.
    Own(&'k str),
    /// Unreachable (`__proto__`, a nested key with a dot, anything under those).
    Dead,
}

/// The container a walk path sits in: the path up to its last dot (a key with a dot is never
/// stored under a parent, so the last dot always ends the parent); `None` at the top level.
fn parent(path: &str) -> Option<&str> {
    path.rfind('.').and_then(|i| path.get(..i))
}

/// Children by container path, for a document that replaces a non-empty container (a
/// duplicate key): built by one scan of what is stored at the first such replacement, kept
/// up to date after it, so every replacement removes only its own subtree and the whole
/// parse stays linear. A document without one never builds it. A list of children is
/// linked through two maps of the type the builder already uses (`first` child of a
/// container, `next` sibling of a child): no new map type in the binary (wasm size).
#[derive(Default)]
struct Kids(Option<Lists>);

/// `first` child by container path, `next` sibling by child path.
type Lists = (FastMap<Arc<str>, Arc<str>>, FastMap<Arc<str>, Arc<str>>);

/// Links `child` (stored at its path) first under its parent.
fn link(walk: &FastMap<Arc<str>, Node>, (first, next): &mut Lists, child: &Arc<str>) {
    if let Some((p, _)) = parent(child).and_then(|p| walk.get_key_value(p))
        && let Some(was) = first.insert(p.clone(), child.clone())
    {
        next.insert(child.clone(), was);
    }
}

impl Kids {
    /// `path` was just stored for the first time.
    fn add(&mut self, walk: &FastMap<Arc<str>, Node>, path: &str) {
        if let Some(lists) = &mut self.0
            && let Some((k, _)) = walk.get_key_value(path)
        {
            link(walk, lists, k);
        }
    }

    /// Removes everything stored under `path` (the container there was just replaced).
    fn drop_under(&mut self, walk: &mut FastMap<Arc<str>, Node>, path: &str) {
        let (first, next) = self.0.get_or_insert_with(|| {
            let mut lists = (FastMap::default(), FastMap::default());
            for k in walk.keys() {
                link(walk, &mut lists, k);
            }
            lists
        });
        let mut todo: Vec<Arc<str>> = first.remove(path).into_iter().collect();
        while let Some(k) = todo.pop() {
            walk.remove(&k);
            todo.extend(next.remove(&k));
            todo.extend(first.remove(&k));
        }
    }
}

struct Frame {
    /// `path` length when this container was opened.
    len: usize,
    array: bool,
    next_index: usize,
    live: bool,
    /// Whether anything reachable was stored under it.
    kids: bool,
}

impl Tree {
    /// Drops what was derived from the content (the payload JSON).
    #[inline]
    fn changed(&mut self) {
        #[cfg(feature = "payload")]
        self.json.take();
    }

    fn is_empty(&self) -> bool {
        self.walk.is_empty() && self.own.is_empty()
    }

    /// What `key` holds for a lookup: an own root key unless null, else the walk.
    pub(crate) fn get(&self, key: &str) -> Option<&Entry> {
        fn read<'a>(map: &'a FastMap<Arc<str>, Node>, key: &str) -> Option<&'a Entry> {
            match map.get(key)? {
                Node::Null => None,
                Node::Obj { .. } | Node::Arr { .. } => Some(&NODE),
                Node::Leaf(e) | Node::Linked(e) => Some(e),
            }
        }
        // The root key "" is never read (`t("")` reads nothing).
        if key.is_empty() {
            return None;
        }
        if !self.own.is_empty()
            && let Some(e) = read(&self.own, key)
        {
            return Some(e);
        }
        read(&self.walk, key)
    }

    /// Reads a JSON object document. `path` names the source in errors.
    pub(crate) fn parse(bytes: &[u8], path: &str) -> Result<Tree, Error> {
        let error = |offset: usize, kind: &'static str| Error::Json {
            path: path.to_owned(),
            offset,
            kind,
        };
        let json_err = |e: crate::json::JsonError| error(e.offset, e.kind.as_str());
        let mut r = Reader::new(bytes);
        match r.next().map_err(json_err)? {
            Some(Event::ObjectStart) => {}
            _ => {
                // Valid JSON of another kind, or broken: the whole document decides.
                while r.next().map_err(json_err)?.is_some() {}
                return Err(Error::NotObject {
                    path: path.to_owned(),
                });
            }
        }
        let mut budget = path_budget(bytes.len());
        let tree = Tree::read_object(&mut r, path, &mut budget)?;
        // the reader reports trailing bytes as an error
        r.next().map_err(json_err)?;
        Ok(tree)
    }

    /// Reads one object from `r`, right after its `{`, up to and with its `}`: a whole
    /// document ([`Tree::parse`]) or a value inside one (a payload chunk's `data`, read in
    /// the same pass as the payload). Stored path bytes come out of `budget`.
    pub(crate) fn read_object(
        r: &mut Reader<'_>,
        path: &str,
        budget: &mut usize,
    ) -> Result<Tree, Error> {
        let error = |offset: usize, kind: &'static str| Error::Json {
            path: path.to_owned(),
            offset,
            kind,
        };
        let json_err = |e: crate::json::JsonError| error(e.offset, e.kind.as_str());
        let mut tree = Tree::default();
        let mut buf = String::new();
        let mut stack = vec![Frame {
            len: 0,
            array: false,
            next_index: 0,
            live: true,
            kids: false,
        }];
        let mut key = None;
        let mut kids = Kids::default();
        while let Some(event) = r.next().map_err(json_err)? {
            let (container, node) = match event {
                Event::Key(k) => {
                    key = Some(k);
                    continue;
                }
                Event::ObjectEnd | Event::ArrayEnd => {
                    // An empty reachable container (not the root) says so.
                    if let Some(f) = stack.pop()
                        && f.live
                        && !f.kids
                        && !stack.is_empty()
                        && let Some(Node::Obj { kids } | Node::Arr { kids }) =
                            buf.get(..f.len).and_then(|p| tree.walk.get_mut(p))
                    {
                        *kids = false;
                    }
                    if stack.is_empty() {
                        return Ok(tree);
                    }
                    continue;
                }
                Event::ObjectStart => (Some(false), Node::Obj { kids: true }),
                Event::ArrayStart => (Some(true), Node::Arr { kids: true }),
                Event::Str(s) => (None, Node::Leaf(Entry::Msg(Msg::new(&s)))),
                Event::Num(n) => (
                    None,
                    Node::Leaf(Entry::Value(number_to_js_string(n).into())),
                ),
                Event::Bool(b) => (
                    None,
                    Node::Leaf(Entry::Value(Arc::from(if b { "true" } else { "false" }))),
                ),
                Event::Null => (None, Node::Null),
            };
            let depth = stack.len();
            let Some(parent) = stack.last_mut() else {
                break;
            };
            buf.truncate(parent.len);
            let k = key.take();
            let target = if !parent.live {
                Target::Dead
            } else if parent.array {
                if depth > 1 {
                    buf.push('.');
                }
                crate::json::push_u64(&mut buf, parent.next_index as u64);
                parent.next_index += 1;
                Target::Walk
            } else {
                let k = k.as_deref().unwrap_or("");
                if k == "__proto__" || (k.contains('.') && depth > 1) {
                    Target::Dead
                } else if k.contains('.') {
                    Target::Own(k)
                } else {
                    if depth > 1 {
                        buf.push('.');
                    }
                    buf.push_str(k);
                    Target::Walk
                }
            };
            let live = match target {
                Target::Walk => {
                    parent.kids = true;
                    if buf.len() > MAX_PATH {
                        return Err(error(r.offset(), "key path longer than 4096 bytes"));
                    }
                    *budget = match budget.checked_sub(buf.len()) {
                        Some(left) => left,
                        None => {
                            return Err(error(
                                r.offset(),
                                "key paths too long for the document size",
                            ));
                        }
                    };
                    // A duplicate key replaces, it does not merge (`JSON.parse`).
                    match tree.walk.insert(Arc::from(buf.as_str()), node) {
                        Some(Node::Obj { kids: true } | Node::Arr { kids: true }) => {
                            kids.drop_under(&mut tree.walk, &buf);
                        }
                        Some(_) => {}
                        None => kids.add(&tree.walk, &buf),
                    }
                    true
                }
                Target::Own(k) => {
                    tree.own.insert(Arc::from(k), node);
                    false
                }
                Target::Dead => false,
            };
            if let Some(array) = container {
                stack.push(Frame {
                    len: buf.len(),
                    array,
                    next_index: 0,
                    live,
                    kids: false,
                });
            }
        }
        // the reader ends a document only after its root closed
        Err(error(r.offset(), "unexpected end"))
    }

    /// Deep merge, `incoming` wins: objects merge, everything else (arrays, scalars,
    /// `null`) replaces what was there.
    pub(crate) fn merge(&mut self, incoming: Tree) {
        self.changed();
        if self.is_empty() {
            *self = incoming;
            return;
        }
        // Two passes, so the map's order does not matter: first drop everything under a
        // container an incoming value replaces (an object over an object replaces nothing),
        // in one scan for all of them, then insert.
        let mut incoming = incoming;
        // the walk's own map type: no new one in the binary (wasm size)
        let mut gone: FastMap<Arc<str>, Node> = FastMap::default();
        for (k, node) in &mut incoming.walk {
            match (node, self.walk.get(k)) {
                // Object over object: the merged one holds the children of both.
                (Node::Obj { kids }, Some(Node::Obj { kids: had })) => *kids |= *had,
                (_, Some(Node::Obj { kids: true } | Node::Arr { kids: true })) => {
                    gone.insert(k.clone(), Node::Null);
                }
                _ => {}
            }
        }
        if !gone.is_empty() {
            // a key is under a replaced container when one of its dotted prefixes is one
            self.walk.retain(|k, _| {
                !k.bytes()
                    .enumerate()
                    .any(|(i, b)| b == b'.' && k.get(..i).is_some_and(|p| gone.contains_key(p)))
            });
        }
        self.walk.extend(incoming.walk);
        for (k, node) in incoming.own {
            let both_obj = matches!(node, Node::Obj { .. })
                && matches!(self.own.get(&k), Some(Node::Obj { .. }));
            if !both_obj {
                self.own.insert(k, node);
            }
        }
    }
}

// ---- linked messages and `check()` (M3, `linked.rs` / `check.rs`) ----

/// Byte ranges of the placeholder names in `raw` (without the braces), as `t` fills them.
pub(crate) fn hole_ranges(raw: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    Holes::new(raw)
}

/// A string leaf of a chunk that has an `@` ([`Tree::strings_with_at`]).
pub(crate) struct AtString {
    pub(crate) key: Arc<str>,
    /// In the map of root keys with a dot.
    pub(crate) own: bool,
    /// A walk path an own root key covers: no lookup reaches it.
    pub(crate) hidden: bool,
    pub(crate) raw: Arc<str>,
}

impl PluralMsg {
    /// Number of forms, the `=0 ` marker, the shape `tc` reads.
    pub(crate) fn layout(&self) -> (usize, bool, &Shape) {
        (self.forms.len(), self.marked, &self.shape)
    }
}

impl Tree {
    /// Whether an own root key (not `null`) covers the walk path `key`.
    fn covered(&self, key: &str) -> bool {
        matches!(self.own.get(key), Some(n) if !matches!(n, Node::Null))
    }

    /// [`Tree::get`], except that the root key `""` is readable: a link `@:{''}` reads it
    /// (`valueAt` of `linked-messages`), `t("")` does not.
    pub(crate) fn link_target(&self, key: &str) -> Option<&Entry> {
        if key.is_empty() {
            return match self.walk.get(key)? {
                Node::Null => None,
                Node::Obj { .. } | Node::Arr { .. } => Some(&NODE),
                Node::Leaf(e) | Node::Linked(e) => Some(e),
            };
        }
        self.get(key)
    }

    /// Every own string leaf with an `@`, in no particular order.
    pub(crate) fn strings_with_at(&self) -> Vec<AtString> {
        let out: Vec<AtString> = [(&self.own, true), (&self.walk, false)]
            .into_iter()
            .flat_map(|(map, own)| {
                map.iter().filter_map(move |(k, n)| match n {
                    Node::Leaf(e @ Entry::Msg(_)) => e
                        .text()
                        .filter(|t| t.contains('@'))
                        .map(|t| (k.clone(), own, t.clone())),
                    _ => None,
                })
            })
            .map(|(key, own, raw)| AtString {
                hidden: !own && self.covered(&key),
                key,
                own,
                raw,
            })
            .collect();
        out
    }

    /// Replaces the string leaf at `key` (in the own map when `own`) with `text`.
    pub(crate) fn set_message(&mut self, key: &str, own: bool, text: &str) {
        self.changed();
        let map = if own { &mut self.own } else { &mut self.walk };
        if let Some(node) = map.get_mut(key) {
            *node = Node::Leaf(Entry::Msg(Msg::new(text)));
        }
    }

    /// Stores what this chunk's view reads at `key` where a weaker layer's stored answer
    /// is not it ([`Node::Linked`]).
    pub(crate) fn insert_linked(&mut self, key: &str, entry: Entry) {
        self.changed();
        self.walk.insert(Arc::from(key), Node::Linked(entry));
    }

    /// What `has` sees: [`Tree::get`] without [`Node::Linked`] texts.
    pub(crate) fn has(&self, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        let read = |map: &FastMap<Arc<str>, Node>| match map.get(key) {
            None | Some(Node::Null) => None,
            Some(Node::Linked(_)) => Some(false),
            Some(_) => Some(true),
        };
        match read(&self.own) {
            Some(true) => true,
            _ => read(&self.walk) == Some(true),
        }
    }

    /// Only linked texts: a chunk `linked.rs` made for a view, no file behind it.
    pub(crate) fn only_linked(&self) -> bool {
        !self.walk.is_empty()
            && self.own.is_empty()
            && self.walk.values().all(|n| matches!(n, Node::Linked(_)))
    }

    /// The messages and values a lookup reaches, own texts only (no [`Node::Linked`]).
    pub(crate) fn leaves(&self) -> impl Iterator<Item = (&str, &Entry)> {
        let own = self.own.iter().filter_map(|(k, n)| match n {
            Node::Leaf(e) => Some((&**k, e)),
            _ => None,
        });
        let walk = self.walk.iter().filter_map(|(k, n)| match n {
            Node::Leaf(e) if !k.is_empty() && !self.covered(k) => Some((&**k, e)),
            _ => None,
        });
        own.chain(walk)
    }
}

// ---- payload for the frontend (M7, `payload.rs`) ----

#[cfg(feature = "payload")]
impl Tree {
    /// The chunk as the front end gets it, written once on first use (8b.4: a loaded chunk
    /// never changes, every `&mut self` method clears it), with its content tag (the
    /// protocol's URL and `ETag`): `{"data":{…}}`, plus `"linked":{"key":text,…}` when
    /// the chunk holds answers `linked.rs` stored for its view ([`Node::Linked`]).
    ///
    /// `data` is the nested tree JS `addRouteTranslations` takes, keys sorted (byte order)
    /// and arrays by index, so the bytes depend only on the content. Unreachable parts are
    /// left out: `__proto__`, keys with a dot below the root. `linked` holds flat dotted
    /// keys: JS merges them into `data` as own root keys (which `getByPath` reads before
    /// the dotted walk); a Rust guest stores them as [`Node::Linked`] again, so `has` does
    /// not see them, as on the server.
    pub(crate) fn payload(&self) -> &(Arc<str>, u64) {
        self.json.get_or_init(|| {
            let mut json = String::from("{\"data\":");
            self.write_json_uncached(&mut json);
            self.write_linked(&mut json);
            json.push('}');
            let tag = content_tag(json.as_bytes());
            (Arc::from(json), tag)
        })
    }

    /// The nested tree alone (`data` of [`Tree::payload`], no linked texts).
    pub(crate) fn write_json(&self, out: &mut String) {
        self.write_json_uncached(out);
    }

    /// `,"linked":{…}` for the linked texts of the walk, sorted; nothing without any.
    fn write_linked(&self, out: &mut String) {
        let mut linked: Vec<(&str, &Entry)> = self
            .walk
            .iter()
            .filter_map(|(k, n)| match n {
                Node::Linked(e) => Some((&**k, e)),
                _ => None,
            })
            .collect();
        if linked.is_empty() {
            return;
        }
        linked.sort_unstable_by(|a, b| a.0.cmp(b.0));
        out.push_str(",\"linked\":{");
        for (i, (key, e)) in linked.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            crate::json::write_str(out, key);
            out.push(':');
            match e {
                Entry::Msg(_) => crate::json::write_str(out, e.text().map_or("", |t| t)),
                Entry::Value(v) => push_value(out, v),
                // a subtree: JS reads an object there, as a lookup reads a node
                Entry::Node => out.push_str("{}"),
            }
        }
        out.push('}');
    }

    /// Stores what a payload's `linked` object holds for this chunk's view.
    pub(crate) fn add_linked(&mut self, linked: Vec<(Arc<str>, Entry)>) {
        self.changed();
        for (k, e) in linked {
            self.walk.insert(k, Node::Linked(e));
        }
    }

    fn write_json_uncached(&self, out: &mut String) {
        // children by container path (`None`: the chunk root); a path's last segment is its
        // key, as `parent` splits it
        let mut kids: FastMap<Option<&str>, Vec<(&str, &str, &Node)>> = FastMap::default();
        for (path, node) in &self.walk {
            if matches!(node, Node::Linked(_)) {
                continue;
            }
            let (parent, key) = match path.rfind('.') {
                Some(i) => (path.get(..i), path.get(i + 1..).unwrap_or("")),
                None => (None, &**path),
            };
            kids.entry(parent).or_default().push((key, path, node));
        }
        let mut top: Vec<(&str, Option<&str>, &Node)> = kids
            .remove(&None)
            .unwrap_or_default()
            .into_iter()
            .map(|(k, p, n)| (k, Some(p), n))
            .collect();
        // own root keys hold dots, walk keys never do: no clash; an own key's subtree is
        // never stored (unreachable), so it has no path and writes as empty
        top.extend(
            self.own
                .iter()
                .filter_map(|(k, n)| (!matches!(n, Node::Linked(_))).then_some((&**k, None, n))),
        );
        top.sort_unstable_by(|a, b| a.0.cmp(b.0));
        out.push('{');
        for (i, (key, path, node)) in top.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            crate::json::write_str(out, key);
            out.push(':');
            write_node(out, &mut kids, path, node);
        }
        out.push('}');
    }

    /// Merges `incoming` with its entries at `depth` replacing (not merging into) what was
    /// stored there: named formats `{"<locale>": {"<name>": {..}}}` at depth 2, where a name
    /// given again wins whole, as `Builder::formats` documents.
    pub(crate) fn merge_replacing(&mut self, incoming: Tree, depth: usize) {
        let at: Vec<Arc<str>> = incoming
            .walk
            .keys()
            .filter(|k| k.bytes().filter(|&b| b == b'.').count() + 1 == depth)
            .cloned()
            .collect();
        if !at.is_empty() {
            self.walk.retain(|k, _| {
                !at.iter().any(|p| {
                    k.strip_prefix(&**p)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
                })
            });
        }
        self.merge(incoming);
    }
}

/// A number or boolean as JS prints it is valid JSON as it is, except the non-finite
/// numbers (`1e400` reads as `Infinity`): `null` there, as `JSON.stringify` writes them.
#[cfg(feature = "payload")]
fn push_value(out: &mut String, v: &str) {
    if v.ends_with("Infinity") || v == "NaN" {
        out.push_str("null");
    } else {
        out.push_str(v);
    }
}

/// The content tag of a chunk's payload JSON (the protocol's URL and `ETag`): the
/// catalog's own fixed-seed hasher over the bytes, so the same content gives the same tag
/// in every run.
#[cfg(feature = "payload")]
pub(crate) fn content_tag(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut h = crate::hash::FastHasher::default();
    h.write(bytes);
    h.finish()
}

/// Writes `node` stored at walk path `path` (`None`: an own root key, no children).
#[cfg(feature = "payload")]
fn write_node<'t>(
    out: &mut String,
    kids: &mut FastMap<Option<&'t str>, Vec<(&'t str, &'t str, &'t Node)>>,
    path: Option<&'t str>,
    node: &'t Node,
) {
    let (open, close, array) = match node {
        Node::Leaf(Entry::Msg(m)) => {
            let raw = match m {
                Msg::Text(t) => &t.raw,
                Msg::Plural(p) => &p.text.raw,
            };
            return crate::json::write_str(out, raw);
        }
        Node::Leaf(Entry::Value(v)) => return push_value(out, v),
        Node::Null | Node::Leaf(Entry::Node) | Node::Linked(_) => return out.push_str("null"),
        Node::Obj { .. } => ('{', '}', false),
        Node::Arr { .. } => ('[', ']', true),
    };
    let mut list = path.and_then(|p| kids.remove(&Some(p))).unwrap_or_default();
    if array {
        list.sort_unstable_by_key(|k| k.0.parse::<usize>().unwrap_or(usize::MAX));
    } else {
        list.sort_unstable_by(|a, b| a.0.cmp(b.0));
    }
    out.push(open);
    for (i, (key, child, n)) in list.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if !array {
            crate::json::write_str(out, key);
            out.push(':');
        }
        write_node(out, kids, Some(child), n);
    }
    out.push(close);
}
