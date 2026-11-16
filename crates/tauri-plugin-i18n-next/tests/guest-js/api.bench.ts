// Benches of the public API (DESIGN 7.7, PLAN: a bench per public function), against the
// fake plugin with precomputed chunk tags, so the numbers are guest-js and not the fake.
// Names are `group/function/case` (`scripts/bench-inventory.mjs` checks every exported
// function and `Link` method has one). `pnpm bench` prints the table (tinybench, medians);
// `BENCH_TIME` (ms per task) shortens it for the CI smoke.
import { createI18n } from '@i18n-micro/runtime'
import { Bench } from 'tinybench'
import { it } from 'vitest'
import { connect, meta } from '../../guest-js/index.js'
import { FakePlugin, type Source } from './fake.js'

const time = Number((globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.BENCH_TIME ?? 300)

type Tree = Record<string, unknown>

/** A root of `keys` keys: in 50 groups (`namespaced`) or all at the top (`spread`). */
function root(keys: number, shape: 'namespaced' | 'spread', tag: string): Tree {
  const out: Tree = {}
  for (let i = 0; i < keys; i++) {
    const text = `${tag} text number ${i} with {param}`
    if (shape === 'spread') out[`key${i}`] = text
    else ((out[`group${i % 50}`] ??= {}) as Tree)[`key${i}`] = text
  }
  return out
}

/**
 * A plugin over fixed chunks: `en` and `de → en`, a root and `pages` small pages per locale,
 * tags computed once.
 */
export function benchSource(keys: number, shape: 'namespaced' | 'spread', pages: number): Source {
  const chains: Record<string, string[]> = { en: ['en'], de: ['de', 'en'] }
  const bodies = new Map<string, string>()
  const tags = new Map<string, string>()
  const put = (key: string, data: Tree) => {
    bodies.set(key, JSON.stringify({ data }))
    tags.set(key, (tags.size + 1).toString(16).padStart(16, '0'))
  }
  put('en/index', root(keys, shape, 'en'))
  put('de/index', root(keys / 2, shape, 'de'))
  for (let p = 0; p < pages; p++) {
    put(`en/p${p}`, { title: `Page ${p}`, group0: { key0: `page ${p} key0` } })
    put(`de/p${p}`, { title: `Seite ${p}` })
  }
  const keysOf = (l: string, page = 'index') =>
    chains[l]!.flatMap((c) => (page === 'index' ? ['index'] : ['index', page]).map((p) => `${c}/${p}`)).filter((k) => tags.has(k))
  const locales = Object.keys(chains).map((code) => ({ code }))
  return {
    meta: (l, page) =>
      page && page !== 'index' && !tags.has(`en/${page}`)
        ? undefined
        : {
            version: 0,
            locale: l,
            defaultLocale: 'en',
            fallbackLocale: 'en',
            locales,
            chains,
            numberFormats: {},
            datetimeFormats: {},
            timeZone: null,
            tags: Object.fromEntries(keysOf(l, page).map((k) => [k, tags.get(k)!])),
            bundle: '0'.repeat(16),
          },
    chunk: (path) => {
      const key = path.slice(17, -5)
      return tags.get(key) === path.slice(0, 16) ? JSON.parse(bodies.get(key)!) : undefined
    },
    tag: (key) => tags.get(key)!,
    load: (l, page) => ({
      version: 0,
      locale: l,
      chunks: keysOf(l, page).map((k) => {
        const [locale, p] = k.split('/')
        return { locale, page: p, ...JSON.parse(bodies.get(k)!) }
      }),
    }),
    locales: () => Object.keys(chains),
  }
}

it('public API of guest-js', async () => {
  const bench = new Bench({ time })
  for (const [keys, shape] of [
    [10_000, 'namespaced'],
    [10_000, 'spread'],
  ] as const) {
    const fake = new FakePlugin(benchSource(keys, shape, 2), 'en')
    fake.install()
    const i18n = createI18n({ locale: 'en', missingWarn: false })
    const link = await connect(i18n, { document: null, reportMissing: false, maxPages: 1 })
    let n = 0
    // two pages, room for one: every visit builds the page view again
    bench.add(`link/setPage/first_visit_${keys / 1000}k_${shape}`, () => link.setPage(`p${n++ % 2}`), { beforeAll: () => fake.install() })
  }
  await bench.run()
  console.table(bench.table())
  void meta
}, 600_000)
