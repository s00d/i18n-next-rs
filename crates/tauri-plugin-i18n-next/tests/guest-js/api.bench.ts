// Benches of the public API (DESIGN 7.7, PLAN: a bench per public function), against the
// fake plugin with precomputed chunk tags, so the numbers are guest-js and not the fake.
// Names are `group/function/case` (`scripts/bench-inventory.mjs` checks every exported
// function and `Link` method has one). `pnpm bench` prints the table (tinybench, medians);
// `BENCH_TIME` (ms per task) shortens it for the CI smoke.
import { createI18n } from '@i18n-micro/runtime'
import { Bench } from 'tinybench'
import { it } from 'vitest'
import { connect, meta } from '../../guest-js/index.js'
import { FakePlugin, benchSource } from './fake.js'

const time = Number((globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.BENCH_TIME ?? 300)

it('public API of guest-js', async () => {
  const bench = new Bench({ time })
  const quiet = { document: null, reportMissing: false } as const
  for (const [keys, shape] of [
    [10_000, 'namespaced'],
    [10_000, 'spread'],
  ] as const) {
    const fake = new FakePlugin(benchSource(keys, shape, 2), 'en')
    fake.install()
    const link = await connect(createI18n({ locale: 'en', missingWarn: false }), { ...quiet, maxPages: 1 })
    let n = 0
    // two pages, room for one: every visit builds the page view again
    bench.add(`link/setPage/first_visit_${keys / 1000}k_${shape}`, () => link.setPage(`p${n++ % 2}`), { beforeAll: () => fake.install() })
  }

  const keys = 1_000
  const fake = new FakePlugin(benchSource(keys, 'namespaced', 2), 'en')
  const install = { beforeAll: () => fake.install() }
  fake.install()
  bench.add('api/meta/ipc_and_parse_1k', () => meta(), install)
  // start to shown: meta, the root chunk (cached by tag after the first), the view, the listener
  bench.add(
    'api/connect/start_1k',
    async () => (await connect(createI18n({ locale: 'en', missingWarn: false }), quiet)).close(),
    install,
  )
  const i18n = createI18n({ locale: 'en', missingWarn: false })
  const link = await connect(i18n, quiet)
  await link.preload('p0')
  await link.setPage('p1')
  // a page checked at the shown version: no call
  bench.add('link/setPage/cached', () => link.setPage(link.page === 'p1' ? 'index' : 'p1'), install)
  // en ↔ de, chunks cached by tag: set_locale and meta, no read; p0 (preloaded) follows
  let de = false
  bench.add('link/setLocale/cached_chunks', () => link.setLocale((de = !de) ? 'de' : 'en'), install)
  bench.add('link/preload/cached', () => link.preload('p0'), install)
  bench.add('link/load/cached', () => link.load('p0'), install)
  bench.add('link/refresh/unchanged', () => link.refresh(), install)
  // a hook per iteration would cost more than the call: close is measured with its connect
  bench.add('link/close/after_connect_1k', async () => (await connect(createI18n({ locale: 'en', missingWarn: false }), quiet)).close(), install)
  await bench.run()
  console.table(bench.table())
  link.close()
  fake.uninstall()
}, 600_000)
