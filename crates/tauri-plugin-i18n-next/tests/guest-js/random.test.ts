// Random catalogs (fixed seed) recorded by `tests/guest_random.rs`: per catalog the chunk
// tags of every view, the protocol chunk bodies and what Rust `try_t` answers. `connect`
// replays each catalog into a real `@i18n-micro/runtime` instance over the protocol and over
// raw invoke, and every key of every view must read what Rust reads.
import { createI18n } from '@i18n-micro/runtime'
import { afterEach, describe, expect, it } from 'vitest'
import { connect, type Meta } from '../../guest-js/index.js'
import { FakePlugin, type Source } from './fake.js'
import fixture from './fixtures/random.json'

interface Catalog {
  /** Every chunk with its content tag, by `locale/page`. */
  chunks: Record<string, { tag: string; data: unknown; linked?: unknown }>
  /** What Rust answers per view; a key left out is a miss. */
  answers: Record<string, Record<string, string>>
}
const fx = fixture as unknown as { keys: string[]; base: Omit<Meta, 'version' | 'locale' | 'tags' | 'bundle'>; catalogs: Catalog[] }
const LOCALES = ['x', 'de-at', 'de', 'en']
const PAGES = ['index', 'p1', 'p2']

/** The plugin over one catalog: the tags of a view are its chain's chunks (root, page), as Rust checks. */
function source(c: Catalog): Source {
  const keys = (l: string, p = 'index') =>
    fx.base.chains[l]!.flatMap((loc) => (p === 'index' ? ['index'] : ['index', p]).map((pg) => `${loc}/${pg}`)).filter((k) => c.chunks[k])
  const known = (p = 'index') => p === 'index' || Object.keys(c.chunks).some((k) => k.endsWith(`/${p}`))
  const body = ({ data, linked }: Catalog['chunks'][string]) => (linked ? { data, linked } : { data })
  return {
    meta: (l, p) =>
      known(p) ? { ...fx.base, version: 0, locale: l, tags: Object.fromEntries(keys(l, p).map((k) => [k, c.chunks[k]!.tag])), bundle: '0'.repeat(16) } : undefined,
    chunk: (path) => {
      const key = path.slice(17, -5)
      return c.chunks[key]?.tag === path.slice(0, 16) ? body(c.chunks[key]!) : undefined
    },
    tag: (key) => c.chunks[key]!.tag,
    load: (l, p) => ({
      version: 0,
      locale: l,
      chunks: keys(l, p).map((k) => {
        const [locale, page] = k.split('/')
        return { locale, page, ...body(c.chunks[k]!) }
      }),
    }),
    locales: () => LOCALES,
  }
}

function read(i18n: ReturnType<typeof createI18n>, key: string): string | null {
  const v = i18n.t(key, undefined, '\u0000missing')
  if (v === '\u0000missing' || (typeof v === 'object' && v !== null)) return null
  return String(v)
}

let fake: FakePlugin | undefined
afterEach(() => fake?.uninstall())

describe('guest-js reads what Rust reads on random catalogs', () => {
  for (const transport of ['protocol', 'invoke'] as const) {
    it(`${fx.catalogs.length} catalogs, every view (${transport})`, async () => {
      const wrong: unknown[] = []
      for (const [n, c] of fx.catalogs.entries()) {
        fake?.uninstall()
        fake = new FakePlugin(source(c), 'en')
        fake.protocol = transport === 'protocol'
        fake.install()
        const i18n = createI18n({ locale: 'en', missingWarn: false })
        const link = await connect(i18n, { document: null, reportMissing: false })
        for (const l of LOCALES) {
          await link.setLocale(l)
          for (const p of PAGES) {
            await link.setPage(p)
            const want = c.answers[`${l}|${p}`]!
            for (const k of fx.keys) {
              const got = read(i18n, k)
              if (got !== (want[k] ?? null)) wrong.push({ catalog: n, view: `${l}|${p}`, key: k, got, want: want[k] ?? null })
            }
          }
        }
        link.close()
      }
      expect(wrong).toEqual([])
    })
  }
})
