// Stories of `connect` (DESIGN 3.4, 6.2, 8b.7, 8b.16) against the fake plugin: what a user
// of the app sees and what reaches Rust, step by step.
import { createI18n } from '@i18n-micro/runtime'
import { afterEach, describe, expect, it } from 'vitest'
import { connect } from '../../guest-js/index.js'
import { type Catalog, FakePlugin, catalogSource } from './fake.js'

const CHAINS = { en: ['en'], ar: ['ar', 'en'] }
const PAGES = ['index', 'p1', 'p2', 'p3', 'p4']

function catalog(): Catalog {
  return {
    en: {
      index: { title: 'App', nav: { home: 'Home' } },
      p1: { h: 'One' },
      p2: { h: 'Two' },
      p3: { h: 'Three' },
      p4: { h: 'Four' },
    },
    ar: { index: { title: 'تطبيق' }, p1: { h: 'واحد' } },
  }
}

let fake: FakePlugin
afterEach(() => fake.uninstall())

function setup(cat = catalog(), extra = {}) {
  fake = new FakePlugin(catalogSource(cat, CHAINS, PAGES, extra), 'en')
  fake.install()
  return { cat, i18n: createI18n({ locale: 'en', missingWarn: false }) }
}

const pageViews = (i18n: ReturnType<typeof createI18n>) =>
  [...i18n.storage.translations.keys()].filter((k) => !k.endsWith(':index')).sort()
const microtasks = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe('connect', () => {
  it('keeps maxPages page views: the current page and preloaded ones stay, a revisit reads again, a change reaches preloaded pages', async () => {
    const { cat, i18n } = setup()
    const link = await connect(i18n, { maxPages: 1, document: null, reportMissing: false })
    // step 1: a preloaded page is loaded, not shown
    await link.preload('p4')
    expect([link.page, pageViews(i18n)]).toEqual(['index', ['en:p4']])
    // step 2: over the limit, but p4 is preloaded and p1 is shown: both stay
    await link.setPage('p1')
    expect([i18n.t('h'), pageViews(i18n)]).toEqual(['One', ['en:p1', 'en:p4']])
    // step 3: p1 is neither: it goes
    await link.setPage('p2')
    expect([i18n.t('h'), pageViews(i18n)]).toEqual(['Two', ['en:p2', 'en:p4']])
    // step 4: back on p1: its chunk is read again, p2 goes
    const fetched = fake.fetches.length
    await link.setPage('p1')
    expect([i18n.t('h'), pageViews(i18n), fake.fetches.length]).toEqual(['One', ['en:p1', 'en:p4'], fetched + 1])
    // step 5: p4 changes in Rust; after the event its view is new without a visit
    cat.en!.p4 = { h: 'Four!' }
    fake.bump()
    await fake.deliver(0)
    await link.setPage('p1')
    expect(i18n.t('h', undefined, null, 'p4')).toBe('Four!')
    link.close()
    // step 6: with room for two, only the least recently used page goes
    const two = createI18n({ locale: 'en', missingWarn: false })
    const link2 = await connect(two, { maxPages: 2, document: null, reportMissing: false })
    for (const p of ['p1', 'p2', 'p1', 'p3']) await link2.setPage(p)
    expect(pageViews(two)).toEqual(['en:p1', 'en:p3'])
    link2.close()
  })

  it('reports misses to Rust once per key, in one batch per microtask; not while loading; never in a release build', async () => {
    const { i18n } = setup()
    const own: string[] = []
    i18n.missingHandler = (_l, key) => own.push(key)
    const link = await connect(i18n, { document: null })
    // step 1: two keys, one of them twice → one call with two keys; the app's handler sees all
    i18n.t('nope')
    i18n.t('nope')
    i18n.t('nav.nope')
    await microtasks()
    expect(fake.missing).toEqual([
      { locale: 'en', page: 'index', key: 'nope' },
      { locale: 'en', page: 'index', key: 'nav.nope' },
    ])
    expect(fake.ipc.filter((c) => c.endsWith('report_missing'))).toHaveLength(1)
    expect(own).toEqual(['nope', 'nope', 'nav.nope'])
    // step 2: a miss while a page is on its way is no miss
    const going = link.setPage('p1')
    i18n.t('loading')
    await going
    await microtasks()
    expect(fake.missing.map((m) => m.key)).not.toContain('loading')
    // step 3: a release build has no command: the first refusal stops reporting
    fake.dev = false
    i18n.t('x1')
    await microtasks()
    i18n.t('x2')
    await microtasks()
    expect(fake.ipc.filter((c) => c.endsWith('report_missing'))).toHaveLength(2)
    // step 4: close gives the app its handler back
    link.close()
    expect(i18n.missingHandler).toBeDefined()
    i18n.t('x3')
    expect(own[own.length - 1]).toBe('x3')
  })

  it('sets <html lang dir> per locale and hints every chunk it reads with <link rel="preload">', async () => {
    const { i18n } = setup()
    const head: Array<{ rel: string; as: string; crossOrigin: string; href: string; removed: boolean }> = []
    const doc = {
      documentElement: { lang: '', dir: '' },
      head: { appendChild: (l: (typeof head)[0]) => head.push(l) },
      createElement: () => {
        const l = { rel: '', as: '', crossOrigin: '', href: '', removed: false, remove: () => (l.removed = true) }
        return l
      },
    } as unknown as Document
    const link = await connect(i18n, { document: doc, reportMissing: false })
    // step 1: en: the iso code would win; en has none here → the code, ltr
    expect([doc.documentElement.lang, doc.documentElement.dir]).toEqual(['en', 'ltr'])
    expect(head.map((l) => [l.rel, l.as, l.crossOrigin, l.removed])).toEqual([['preload', 'fetch', 'anonymous', true]])
    expect(head[0]!.href).toBe(fake.fetches[0])
    // step 2: ar is rtl
    await link.setLocale('ar')
    expect([doc.documentElement.lang, doc.documentElement.dir, i18n.t('title'), i18n.t('nav.home')]).toEqual(['ar', 'rtl', 'تطبيق', 'Home'])
    link.close()
  })

  it('applies the formats and iso codes of the payload: tn / td by name', async () => {
    const { i18n } = setup(catalog(), {
      locales: [{ code: 'en', iso: 'en-US' }, { code: 'ar', iso: 'ar-EG', dir: 'rtl' }],
      numberFormats: { en: { money: { style: 'currency', currency: 'USD' } } },
      datetimeFormats: { en: { day: { year: 'numeric', month: '2-digit', day: '2-digit', timeZone: 'UTC' } } },
    })
    const link = await connect(i18n, { document: null, reportMissing: false })
    expect(i18n.tn(1.5, 'money')).toBe('$1.50')
    expect(i18n.td(Date.UTC(2026, 10, 16), 'day')).toBe('11/16/2026')
    link.close()
  })

  it('goes over raw invoke when the protocol is out, or when asked to; refuses to when told to use the protocol', async () => {
    // step 1: asked for invoke: not one fetch
    let { i18n } = setup()
    let link = await connect(i18n, { transport: 'invoke', page: 'p1', document: null, reportMissing: false })
    expect([link.fallback, fake.fetches.length, i18n.t('h'), i18n.t('title')]).toEqual([true, 0, 'One', 'App'])
    link.close()
    fake.uninstall()
    // step 2: protocol only, and it is out: connect fails
    ;({ i18n } = setup())
    fake.protocol = false
    await expect(connect(i18n, { transport: 'protocol', document: null, reportMissing: false })).rejects.toThrow('Load failed')
    // step 3: auto: one failed fetch, then invoke for good
    link = await connect(i18n, { document: null, reportMissing: false })
    const fetches = fake.fetches.length
    await link.setPage('p2')
    expect([link.fallback, fake.fetches.length, i18n.t('h')]).toEqual([true, fetches, 'Two'])
    link.close()
  })

  it('shows a route the catalog has no page for from the root; a failed call keeps what is shown', async () => {
    const { i18n } = setup()
    const link = await connect(i18n, { page: 'p1', document: null, reportMissing: false })
    // step 1: an unknown route reads the root
    await link.setPage('ghost')
    expect([link.page, i18n.getRoute(), i18n.t('title'), i18n.t('h', undefined, '-')]).toEqual(['ghost', 'ghost', 'App', '-'])
    // step 2: Rust fails: the call rejects, the page stays
    const meta = fake.source.meta
    fake.source.meta = () => {
      throw new Error('boom')
    }
    await expect(link.setPage('p2')).rejects.toThrow('boom')
    expect([link.page, i18n.t('title')]).toEqual(['ghost', 'App'])
    fake.source.meta = meta
    // step 3: the next call works again
    await link.setPage('p2')
    expect(i18n.t('h')).toBe('Two')
    link.close()
  })
})
