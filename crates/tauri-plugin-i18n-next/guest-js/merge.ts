/**
 * The premerge of one view (DESIGN 6.2): the JS core reads one locale (and one fallback),
 * the catalog the whole chain. So the chunks of a view are layered here into one tree that
 * answers every key as the Rust lookup does: first layer that has the key wins, `null`
 * hides nothing, a stronger leaf does not hide a weaker key under the same path.
 *
 * Trees are never mutated: a layer copies the nodes on the paths it writes and shares the
 * rest, so views of one locale share their untouched subtrees.
 */

export type Tree = Record<string, unknown>

/** One `(locale, page)` chunk as the protocol serves it. */
export interface Chunk {
  data: Tree
  /** Linked texts of weaker layers resolved in this chunk's view, by flat dotted key. */
  linked?: Tree
}

const isTree = (v: unknown): v is Tree => typeof v === 'object' && v !== null && !Array.isArray(v)
const hasOwn = (t: object, k: string) => Object.prototype.hasOwnProperty.call(t, k)
const own = (t: object, k: string): unknown => (hasOwn(t, k) ? (t as Tree)[k] : undefined)

/** `getByPath` of the JS core: an own (dotted) key first, then the walk by dots. */
export function lookup(tree: Tree, key: string): unknown {
  const direct = own(tree, key)
  if (direct != null || !key.includes('.')) return direct
  let at: unknown = tree
  for (const part of key.split('.')) {
    if (typeof at !== 'object' || at === null || part === '__proto__') return undefined
    at = own(at, part)
  }
  return at
}

const join = (prefix: string, k: string) => (prefix ? `${prefix}.${k}` : k)

/**
 * What a weaker container a stronger value replaced answers, as flat dotted keys: its
 * leaves and its inner containers (a lookup stops at a node, as in Rust).
 */
function leaves(node: object, prefix: string, out: Tree) {
  for (const k of Object.keys(node)) {
    if (k === '__proto__') continue
    const v = (node as Tree)[k]
    if (v == null) continue
    const path = join(prefix, k)
    out[path] = v
    if (typeof v === 'object') leaves(v, path, out)
  }
}

/**
 * A node to write into: one made by this premerge as it is (nobody else holds it), any
 * other copied. `Object.keys` + assignment: twice as fast as a spread on the dictionary-mode
 * objects a big parsed chunk is made of.
 */
function writable(t: Tree, fresh: WeakSet<object>): Tree {
  if (fresh.has(t)) return t
  const out: Tree = {}
  for (const k of Object.keys(t)) out[k] = t[k]
  fresh.add(out)
  return out
}

function mergeNode(lower: Tree, upper: Tree, prefix: string, flat: Tree, fresh: WeakSet<object>): Tree {
  const out = writable(lower, fresh)
  for (const k of Object.keys(upper)) {
    const v = upper[k]
    if (k === '__proto__' || v == null) continue
    const w = own(out, k)
    if (isTree(w) && isTree(v)) out[k] = mergeNode(w, v, join(prefix, k), flat, fresh)
    else {
      if (typeof w === 'object' && w !== null) leaves(w, join(prefix, k), flat)
      out[k] = v
    }
  }
  return out
}

/**
 * `upper` over `lower`: what `mergeTranslationLayers` of the JS core does, `null` skipped.
 * Nodes in `fresh` are written in place, the rest is copied on write.
 */
export function layer(lower: Tree, upper: Tree, fresh: WeakSet<object> = new WeakSet()): Tree {
  // nothing below to hide or keep: the layer as it is (its `null`s read as missing)
  if (Object.keys(lower).length === 0) return upper
  // a weaker own dotted key the stronger layer answers by the walk (an own `null` is no answer)
  const covered = Object.keys(lower).filter((k) => k.includes('.') && own(upper, k) == null && lookup(upper, k) != null)
  const flat: Tree = {}
  const out = mergeNode(lower, upper, '', flat, fresh)
  for (const k of covered) delete out[k]
  for (const k of Object.keys(flat)) {
    if (lookup(upper, k) == null && own(out, k) == null) out[k] = flat[k]
  }
  return out
}

/**
 * `linked` over `view` as own keys. A linked text answers its exact key only: an undotted key
 * that replaces a container first keeps what the container answers as own dotted keys.
 */
function overlay(view: Tree, linked: Tree, fresh: WeakSet<object>): Tree {
  const keys = Object.keys(linked).filter((k) => k !== '__proto__' && linked[k] != null)
  if (keys.length === 0) return view
  const out = writable(view, fresh)
  for (const k of keys) {
    const w = own(out, k)
    if (!k.includes('.') && typeof w === 'object' && w !== null) {
      const flat: Tree = {}
      leaves(w, k, flat)
      for (const p of Object.keys(flat)) if (own(out, p) == null) out[p] = flat[p]
    }
    out[k] = linked[k]
  }
  return out
}

/**
 * One view from its chunks, weakest first: each `data` layered over the ones before, then
 * its `linked` as own keys (an own key is read before the walk).
 */
export function premerge(chunks: readonly Chunk[]): Tree {
  const fresh = new WeakSet<object>()
  let view: Tree = {}
  for (const c of chunks) {
    view = layer(view, c.data, fresh)
    if (c.linked) view = overlay(view, c.linked, fresh)
  }
  return view
}
