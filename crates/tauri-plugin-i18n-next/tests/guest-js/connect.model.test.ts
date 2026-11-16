// Model of `connect` (DESIGN 6.2, 8b.16): random catalogs over the chains en, de → en,
// de-at → de → en, ar → en with pages, and random runs of page visits, preloads, locale
// switches from this window and from others, catalog edits (also under a read: the chunk URL
// answers 410), `changed` events delivered late and out of order, with and without the
// protocol (raw-invoke fallback) and with `maxPages`. Invariants:
// - once every event is delivered, every key reads what the Rust lookup reads in the active
//   locale and page (page, root, then the same along the chain; `null` hides nothing);
// - eviction is invisible and keeps at most `maxPages` page views besides the protected ones;
// - a second visit of a checked page makes no call; the version never goes back.
import { createI18n } from '@i18n-micro/runtime'
import fc from 'fast-check'
import { afterEach, describe, expect, it } from 'vitest'
import { connect } from '../../guest-js/index.js'
import { type Catalog, FakePlugin, type Tree, catalogSource } from './fake.js'

const CHAINS: Record<string, string[]> = { en: ['en'], de: ['de', 'en'], 'de-at': ['de-at', 'de', 'en'], ar: ['ar', 'en'] }
const LOCALES = Object.keys(CHAINS)
const PAGES = ['index', 'p1', 'p2', 'p3', 'p4']
const VISIT = [...PAGES, 'ghost'] // `ghost`: a route the catalog has no page for
const KEYS = ['k0', 'k1', 'k2', 'g.a', 'g.b', 'only']

/** A nested tree from flat keys; `null` values are kept (the Rust lookup skips them). */
function nest(flat: Record<string, string | null>): Tree {
  const out: Tree = {}
  for (const [k, v] of Object.entries(flat)) {
    const [a, b] = k.split('.')
    if (b === undefined) out[a!] = v
    else ((out[a!] ??= {}) as Tree)[b] = v
  }
  return out
}

/** What the Rust lookup answers: the first layer with a non-null value. */
function answer(cat: Catalog, locale: string, page: string, key: string): string | null {
  for (const c of CHAINS[locale]!) {
    for (const p of page === 'index' ? ['index'] : [page, 'index']) {
      const [a, b] = key.split('.')
      const top = cat[c]?.[p]?.[a!]
      const v = b === undefined ? top : (top as Tree | undefined)?.[b]
      if (typeof v === 'string') return v
    }
  }
  return null
}

const value = fc.option(fc.constantFrom('a', 'b', 'c', 'Ü', 'z z'), { nil: null })
const tree = fc.dictionary(fc.constantFrom(...KEYS), value, { maxKeys: 4 }).map(nest)
const catalog: fc.Arbitrary<Catalog> = fc.record(
  Object.fromEntries(LOCALES.map((l) => [l, fc.record(Object.fromEntries(PAGES.map((p) => [p, tree])), { requiredKeys: ['index'] })])),
) as fc.Arbitrary<Catalog>

type Action =
  | { t: 'page'; page: string }
  | { t: 'preload'; page: string }
  | { t: 'locale'; locale: string }
  | { t: 'other'; locale: string } // another window or the tray switches
  // `del`: the key goes (a page whose last key goes has no chunk); `underRead`: the edit lands
  // between `meta` and the read of the shown view, whose chunk then answers 410
  | { t: 'edit'; locale: string; page: string; key: string; value: string | null; del: boolean; underRead: boolean }
  | { t: 'deliver'; index: number }

const action: fc.Arbitrary<Action> = fc.oneof(
  fc.record({ t: fc.constant('page' as const), page: fc.constantFrom(...VISIT) }),
  fc.record({ t: fc.constant('preload' as const), page: fc.constantFrom(...PAGES) }),
  fc.record({ t: fc.constant('locale' as const), locale: fc.constantFrom(...LOCALES) }),
  fc.record({ t: fc.constant('other' as const), locale: fc.constantFrom(...LOCALES) }),
  fc.record({
    t: fc.constant('edit' as const),
    locale: fc.constantFrom(...LOCALES),
    page: fc.constantFrom(...PAGES),
    key: fc.constantFrom(...KEYS),
    value,
    del: fc.boolean(),
    underRead: fc.boolean(),
  }),
  fc.record({ t: fc.constant('deliver' as const), index: fc.nat(8) }),
)

let fake: FakePlugin | undefined
afterEach(() => fake?.uninstall())

async function run(cat: Catalog, actions: Action[], maxPages: number, protocol: boolean) {
  fake?.uninstall()
  fake = new FakePlugin(catalogSource(cat, CHAINS, PAGES), 'en')
  fake.protocol = protocol
  fake.install()
  const f = fake
  const i18n = createI18n({ locale: 'en', missingWarn: false })
  const link = await connect(i18n, { maxPages, document: null, reportMissing: false })
  const pinned = new Set<string>()
  let lastVersion = link.version
  const edit = (a: Extract<Action, { t: 'edit' }>) => {
    const t = ((cat[a.locale] ??= {})[a.page] ??= {})
    const [k, sub] = a.key.split('.')
    const next: Tree = { ...t }
    if (a.del) {
      if (sub === undefined) delete next[k!]
      else if (typeof t[k!] === 'object' && t[k!]) {
        const inner = { ...(t[k!] as Tree) }
        delete inner[sub]
        next[k!] = inner
      }
    } else if (sub === undefined) next[k!] = a.value
    else next[k!] = { ...((typeof t[k!] === 'object' && t[k!]) || {}), [sub]: a.value }
    cat[a.locale]![a.page] = next
    f.bump()
  }
  const check = (when: string) => {
    expect(link.version, when).toBeGreaterThanOrEqual(lastVersion)
    lastVersion = link.version
    const views = [...i18n.storage.translations.keys()].filter((k) => !k.endsWith(':index'))
    const kept = views.filter((k) => k.startsWith(`${link.locale}:`) && (k.endsWith(`:${link.page}`) || pinned.has(k.slice(link.locale.length + 1))))
    expect(views.length, `${when}: page views ${views}`).toBeLessThanOrEqual(Math.max(maxPages, kept.length))
    if (f.pending.length > 0) return // not told yet: the old state is right
    expect(link.locale, when).toBe(f.locale)
    const got = KEYS.map((k) => {
      const v = i18n.t(k, undefined, '\u0000')
      return v === '\u0000' ? null : v
    })
    expect(got, `${when}: ${link.locale}/${link.page}`).toEqual(KEYS.map((k) => answer(cat, f.locale, link.page, k)))
  }
  check('start')
  for (const [i, a] of actions.entries()) {
    const when = `#${i} ${JSON.stringify(a)}`
    switch (a.t) {
      case 'page': {
        await link.setPage(a.page)
        // a second visit at the same version: no call, no fetch (3.4)
        const [ipc, fetches] = [f.ipc.length, f.fetches.length]
        await link.setPage(a.page)
        expect([f.ipc.length, f.fetches.length], when).toEqual([ipc, fetches])
        break
      }
      case 'preload':
        pinned.add(a.page)
        await link.preload(a.page)
        break
      case 'locale':
        await link.setLocale(a.locale)
        break
      case 'other':
        f.switch(a.locale)
        break
      case 'edit':
        if (a.underRead) {
          // a chunk of the shown view, edited before the read and again between meta and the read
          const shown = { ...a, locale: CHAINS[f.locale]![a.page.length % CHAINS[f.locale]!.length]!, page: PAGES.includes(link.page) ? link.page : 'index' }
          edit(shown)
          f.afterMeta = () => edit({ ...shown, value: `${shown.value}!` })
          await link.refresh()
        } else edit(a)
        break
      case 'deliver': {
        await link.setPage(link.page) // the shown page checked at the shown version
        const at = a.index % Math.max(f.pending.length, 1)
        const late = (f.pending[at]?.version ?? Infinity) <= link.version
        const ipc = f.ipc.length
        await f.deliver(at)
        await link.setPage(link.page) // waits for what the event queued
        // a late event (older than what is shown) is dropped: no call at all
        if (late) expect(f.ipc.length, when).toBe(ipc)
        break
      }
    }
    check(when)
  }
  while (f.pending.length > 0) await f.deliver(f.pending.length - 1) // newest first: the rest are late
  await link.setPage(link.page)
  check('end')
  expect(link.fallback).toBe(!protocol && f.fetches.length > 0) // nothing read: nothing failed
  link.close()
}

describe('connect model', () => {
  it('reads what Rust reads through switches, edits, late events, eviction and the fallback', async () => {
    await fc.assert(
      fc.asyncProperty(
        catalog,
        fc.array(action, { maxLength: 30 }),
        fc.constantFrom(1, 2, Infinity),
        fc.boolean(),
        (cat, actions, maxPages, protocol) => run(structuredClone(cat), actions, maxPages, protocol),
      ),
      { seed: 0x18e8, numRuns: 150 },
    )
  })
})
