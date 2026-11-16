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

/** Leaves of a weaker container a stronger value replaced, as flat dotted keys. */
function leaves(node: object, prefix: string, out: Tree) {
  for (const k of Object.keys(node)) {
    if (k === '__proto__') continue
    const v = (node as Tree)[k]
    const path = join(prefix, k)
    if (typeof v === 'object' && v !== null) leaves(v, path, out)
    else if (v != null) out[path] = v
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
  // a weaker own dotted key the stronger layer answers by the walk
  const covered = Object.keys(lower).filter((k) => k.includes('.') && !hasOwn(upper, k) && lookup(upper, k) != null)
  const flat: Tree = {}
  const out = mergeNode(lower, upper, '', flat, fresh)
  for (const k of covered) delete out[k]
  for (const k of Object.keys(flat)) {
    if (lookup(upper, k) == null && !hasOwn(out, k)) out[k] = flat[k]
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
    const linked = c.linked ? Object.keys(c.linked).filter((k) => k !== '__proto__' && c.linked![k] != null) : []
    if (linked.length > 0) view = writable(view, fresh)
    for (const k of linked) view[k] = c.linked![k]
  }
  return view
}
