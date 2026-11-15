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

use crate::args::Args;
use crate::error::Error;
use crate::hash::FastMap;
use crate::json::{Event, Reader, number_to_js_string};
use std::sync::Arc;

/// A string leaf, tokenized once at load: `{name}` ranges are found here, never on `t`.
#[derive(Debug, Clone)]
pub(crate) struct Template {
    pub(crate) raw: Arc<str>,
    /// Byte ranges of placeholder names (without the braces), in order.
    holes: Box<[(u32, u32)]>,
}

/// A message. Plurals and linked messages join in M2 / M3.
#[derive(Debug, Clone)]
pub(crate) enum Msg {
    Text(Template),
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

impl Template {
    /// Finds the tokens of `core/src/text.ts`: `{` + one or more chars that are not `{`,
    /// `}` or JS `\s` + `}`.
    pub(crate) fn new(raw: &str) -> Self {
        let mut holes = Vec::new();
        let mut open = None;
        for (i, c) in raw.char_indices() {
            match c {
                '{' => open = Some(i + 1),
                '}' => {
                    if let Some(start) = open.take()
                        && i > start
                    {
                        holes.push((start as u32, i as u32));
                    }
                }
                c if is_js_space(c) => open = None,
                _ => {}
            }
        }
        Template {
            raw: Arc::from(raw),
            holes: holes.into_boxed_slice(),
        }
    }

    /// The text with every placeholder `args` has replaced, in one allocation.
    pub(crate) fn interpolate(&self, args: &Args<'_>) -> String {
        if self.holes.is_empty() {
            return String::from(&*self.raw);
        }
        let raw = &*self.raw;
        let mut out = String::with_capacity(raw.len() + self.holes.len() * args.max_value_len());
        let mut from = 0;
        for &(s, e) in &self.holes {
            let (s, e) = (s as usize, e as usize);
            if let Some(value) = args.find(&raw[s..e]) {
                out.push_str(&raw[from..s - 1]);
                value.write(&mut out);
                from = e + 1;
            }
        }
        out.push_str(&raw[from..]);
        out
    }
}

/// A node of the tree as stored by path.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    Obj,
    Arr,
    Null,
    Leaf(Entry),
}

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
                Node::Obj | Node::Arr => Some(&NODE),
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
        if let Some(Node::Obj | Node::Arr) = self.walk.remove(path) {
            // ponytail: one O(chunk) scan, only when a container is replaced (a duplicate
            // key, an array patched over); keep children lists if patches ever get hot.
            self.walk.retain(|k, _| {
                !(k.len() > path.len() && k.starts_with(path) && k.as_bytes()[path.len()] == b'.')
            });
        }
    }

    /// Reads a JSON object document. `path` names the source in errors.
    pub(crate) fn parse(bytes: &[u8], path: &str) -> Result<Tree, Error> {
        let json_err = |e: crate::json::JsonError| Error::Json {
            path: path.to_owned(),
            offset: e.offset,
            kind: e.kind.as_str(),
        };
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
        let mut stack = vec![Frame {
            len: 0,
            array: false,
            next_index: 0,
            live: true,
        }];
        let mut key = None;
        while let Some(event) = r.next().map_err(json_err)? {
            let (container, node) = match event {
                Event::Key(k) => {
                    key = Some(k);
                    continue;
                }
                Event::ObjectEnd | Event::ArrayEnd => {
                    stack.pop();
                    continue;
                }
                Event::ObjectStart => (Some(false), Node::Obj),
                Event::ArrayStart => (Some(true), Node::Arr),
                Event::Str(s) => (None, Node::Leaf(Entry::Msg(Msg::Text(Template::new(&s))))),
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
        for (k, node) in &incoming.walk {
            if !(matches!(node, Node::Obj) && matches!(self.walk.get(k), Some(Node::Obj))) {
                self.remove_subtree(k);
            }
        }
        self.walk.extend(incoming.walk);
        for (k, node) in incoming.own {
            if !(matches!(node, Node::Obj) && matches!(self.own.get(&k), Some(Node::Obj))) {
                self.own.insert(k, node);
            }
        }
    }
}
