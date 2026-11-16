/**
 * The premerge of one view (DESIGN 6.2): the JS core reads one locale (and one fallback),
 * the catalog the whole chain. So the chunks of a view are layered here into one tree that
 * answers every key as the Rust lookup does: first layer that has the key wins, `null`
 * hides nothing, a stronger leaf does not hide a weaker key under the same path.
 *
 * Trees are never mutated: a layer copies the nodes on the paths it writes and shares the
 * rest, so views of one locale share their untouched subtrees.
 */
export type Tree = Record<string, unknown>;
/** One `(locale, page)` chunk as the protocol serves it. */
export interface Chunk {
    data: Tree;
    /** Linked texts of weaker layers resolved in this chunk's view, by flat dotted key. */
    linked?: Tree;
}
/** `getByPath` of the JS core: an own (dotted) key first, then the walk by dots. */
export declare function lookup(tree: Tree, key: string): unknown;
/**
 * `upper` over `lower`: what `mergeTranslationLayers` of the JS core does, `null` skipped.
 * Nodes in `fresh` are written in place, the rest is copied on write.
 */
export declare function layer(lower: Tree, upper: Tree, fresh?: WeakSet<object>): Tree;
/**
 * One view from its chunks, weakest first: each `data` layered over the ones before, then
 * its `linked` as own keys (an own key is read before the walk).
 */
export declare function premerge(chunks: readonly Chunk[]): Tree;
