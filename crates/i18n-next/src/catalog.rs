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

use crate::args::{Args, Count, MAX_SCALAR_LEN};
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
                v.write(out);
                true
            }
            None => false,
        }
    }

    fn len_of(&self, name: &str) -> Option<usize> {
        self.find(name).map(|v| v.len())
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
                v.write(out);
                true
            }
            None => false,
        }
    }

    fn len_of(&self, name: &str) -> Option<usize> {
        if name == "count" {
            return Some(self.count.len());
        }
        self.args.and_then(|a| a.spread(name)).map(|v| v.len())
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
        let at = self.shape.index(self.marked, count.whole(), plural);
        let Some(f) = self.forms.get(at) else {
            return String::new();
        };
        let first = at
            .checked_sub(1)
            .and_then(|j| self.forms.get(j))
            .map_or(0, |p| p.holes_end as usize);
        let holes = self.holes.get(first..f.holes_end as usize).unwrap_or(&[]);
        render(
            &self.text.raw,
            f.start as usize,
            f.end as usize,
            holes.iter().map(|&(s, e)| (s as usize, e as usize)),
            holes.len(),
            params,
        )
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
fn is_js_space(c: char) -> bool {
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
        let raw = &*self.raw;
        let body = raw.strip_prefix(ZERO_MARKER).unwrap_or(raw);
        let trimmed = body.trim_matches(is_js_space);
        let start = trimmed.as_ptr() as usize - raw.as_ptr() as usize;
        let end = start + trimmed.len();
        let holes = self.holes().filter(|&(s, e)| s > start && e < end);
        render(raw, start, end, holes, self.holes.len(), args)
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
}

/// The longest dotted path (`a.b.0.c`) a document may reach, in bytes. Every leaf stores
/// its full path, so a deep prefix over many leaves multiplies the input (a 137 KB file
/// took 2.2 GB); real keys, gettext-style sentence keys included, are far shorter.
const MAX_PATH: usize = 4096;

/// Stored path bytes allowed per input byte, plus [`PATH_SLACK`]: bounds the memory of
/// one document by its size even when every path stays under [`MAX_PATH`].
const PATH_RATIO: usize = 64;
const PATH_SLACK: usize = 1 << 20;

/// One `(locale, page)` chunk by dotted path. `walk` holds what the dotted walk reaches
/// (keys without dots, array indices); `own` holds root keys with a dot, which only an
/// exact own-key read reaches (their subtrees never are).
#[derive(Debug, Clone, Default)]
pub(crate) struct Tree {
    walk: FastMap<Arc<str>, Node>,
    own: FastMap<Arc<str>, Node>,
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
    fn is_empty(&self) -> bool {
        self.walk.is_empty() && self.own.is_empty()
    }

    /// What `key` holds for a lookup: an own root key unless null, else the walk.
    pub(crate) fn get(&self, key: &str) -> Option<&Entry> {
        fn read<'a>(map: &'a FastMap<Arc<str>, Node>, key: &str) -> Option<&'a Entry> {
            match map.get(key)? {
                Node::Null => None,
                Node::Obj { .. } | Node::Arr { .. } => Some(&NODE),
                Node::Leaf(e) => Some(e),
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

    /// Removes `path` and everything under it from the walk.
    fn remove_subtree(&mut self, path: &str) {
        if let Some(Node::Obj { kids: true } | Node::Arr { kids: true }) = self.walk.remove(path) {
            // ponytail: one O(chunk) scan, only when a container with children is replaced
            // (a duplicate key, an array patched over); an empty one scans nothing, so a
            // repeated `"x": {}` stays linear. Many duplicate non-empty containers in one
            // document still scan once each: keep children lists if that ever matters.
            self.walk.retain(|k, _| {
                !(k.len() > path.len()
                    && k.starts_with(path)
                    && k.as_bytes().get(path.len()) == Some(&b'.'))
            });
        }
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
        let mut tree = Tree::default();
        let mut buf = String::new();
        let mut budget = bytes
            .len()
            .saturating_mul(PATH_RATIO)
            .saturating_add(PATH_SLACK);
        let mut stack = vec![Frame {
            len: 0,
            array: false,
            next_index: 0,
            live: true,
            kids: false,
        }];
        let mut key = None;
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
                    continue;
                }
                Event::ObjectStart => (Some(false), Node::Obj { kids: true }),
                Event::ArrayStart => (Some(true), Node::Arr { kids: true }),
                Event::Str(s) => (
                    None,
                    Node::Leaf(Entry::Msg(if s.contains('|') {
                        Msg::Plural(Box::new(PluralMsg::new(&s)))
                    } else {
                        Msg::Text(Template::new(&s))
                    })),
                ),
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
                    budget = match budget.checked_sub(buf.len()) {
                        Some(left) => left,
                        None => {
                            return Err(error(
                                r.offset(),
                                "key paths too long for the document size",
                            ));
                        }
                    };
                    // A duplicate key replaces, it does not merge (`JSON.parse`).
                    tree.remove_subtree(&buf);
                    tree.walk.insert(Arc::from(buf.as_str()), node);
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
        Ok(tree)
    }

    /// Deep merge, `incoming` wins: objects merge, everything else (arrays, scalars,
    /// `null`) replaces what was there.
    pub(crate) fn merge(&mut self, incoming: Tree) {
        if self.is_empty() {
            *self = incoming;
            return;
        }
        // Two passes, so the map's order does not matter: first drop everything an
        // incoming value replaces (an object over an object replaces nothing), then insert.
        let mut incoming = incoming;
        for (k, node) in &mut incoming.walk {
            match (node, self.walk.get(k)) {
                // Object over object: the merged one holds the children of both.
                (Node::Obj { kids }, Some(Node::Obj { kids: had })) => *kids |= *had,
                _ => self.remove_subtree(k),
            }
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
