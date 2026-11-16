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
    const handler = (_l: string, key: string) => void own.push(key)
    i18n.missingHandler = handler
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
    expect(i18n.missingHandler).toBe(handler)
    i18n.t('x3')
    expect(own[own.length - 1]).toBe('x3')
  })

  it('sets <html lang dir> per locale from the iso code; adds nothing to <head>', async () => {
    const { i18n } = setup(catalog(), { locales: [{ code: 'en', iso: 'en-US' }, { code: 'ar', dir: 'rtl' }] })
    const appended: unknown[] = []
    const doc = {
      documentElement: { lang: '', dir: '' },
      head: { appendChild: (l: unknown) => appended.push(l) },
      createElement: () => ({}),
    } as unknown as Document
    const link = await connect(i18n, { document: doc, reportMissing: false })
    // step 1: en: the iso code, ltr; the chunk is fetched, no <link> for it
    expect([doc.documentElement.lang, doc.documentElement.dir, appended.length, fake.fetches.length]).toEqual(['en-US', 'ltr', 0, 1])
    // step 2: ar has no iso code: the code, rtl; the instance reads ar alone (its view holds the chain)
    await link.setLocale('ar')
    expect([doc.documentElement.lang, doc.documentElement.dir, i18n.t('title'), i18n.t('nav.home')]).toEqual(['ar', 'rtl', 'تطبيق', 'Home'])
    expect([i18n.locale, i18n.fallbackLocale]).toEqual(['ar', 'ar'])
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
    fake.uninstall()
    // the iso code is what Intl gets: `en` shown as de-DE
    const de = setup(catalog(), { locales: [{ code: 'en', iso: 'de-DE' }], numberFormats: { en: { money: { style: 'currency', currency: 'USD' } } } })
    const link2 = await connect(de.i18n, { document: null, reportMissing: false })
    expect(de.i18n.tn(1.5, 'money')).toBe('1,50\u00a0$')
    link2.close()
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

  it('reads a chunk again after a 410, retries a stale raw load, gives up on a catalog that never holds still', async () => {
    // step 1: p2 changes, and again between meta and the read: the old URL answers 410, a new meta reads the newest
    let { cat, i18n } = setup()
    let link = await connect(i18n, { document: null, reportMissing: false })
    cat.en!.p2 = { h: 'Two!' }
    fake.bump()
    fake.afterMeta = () => {
      cat.en!.p2 = { h: 'Two!!' }
      fake.bump()
    }
    await link.setPage('p2')
    expect([i18n.t('h'), link.fallback, fake.fetches.length]).toEqual(['Two!!', false, 3])
    // step 2: a catalog that changes under every read: the call fails and says why
    let edits = 0
    const edit = () => {
      cat.en!.p3 = { h: `Three ${edits++}` }
      fake.bump()
      fake.afterMeta = edit
    }
    edit()
    await expect(link.setPage('p3')).rejects.toThrow(/chunk en\/p3 changed while it was read.*kept changing/)
    expect([link.page, i18n.t('h')]).toEqual(['p2', 'Two!!'])
    fake.afterMeta = undefined
    link.close()
    fake.uninstall()
    // step 3: raw invoke: A, then B with C landing under the read, then B again: B, not C cached under B's tag
    ;({ cat, i18n } = setup())
    link = await connect(i18n, { transport: 'invoke', document: null, reportMissing: false })
    cat.en!.index = { title: 'B' }
    fake.bump()
    fake.afterMeta = () => {
      cat.en!.index = { title: 'C' }
      fake.bump()
    }
    await link.refresh()
    expect(i18n.t('title')).toBe('C')
    cat.en!.index = { title: 'B' }
    fake.bump()
    await link.refresh()
    expect(i18n.t('title')).toBe('B')
    link.close()
  })

  it('an HTTP error of the protocol fails the read and keeps the protocol', async () => {
    const { i18n } = setup()
    const link = await connect(i18n, { document: null, reportMissing: false })
    fake.status = 500
    await expect(link.setPage('p1')).rejects.toThrow('i18n-next: 500 for chunk en/p1')
    expect([link.fallback, link.page]).toEqual([false, 'index'])
    fake.status = undefined
    await link.setPage('p1')
    expect([link.fallback, i18n.t('h'), fake.ipc.filter((c) => c.endsWith('|load'))]).toEqual([false, 'One', []])
    link.close()
  })

  it('a page whose chunks all go reads the root again; events back to back cost one read', async () => {
    const { cat, i18n } = setup({ en: { index: { title: 'Root' }, p1: { title: 'Page' } } })
    const link = await connect(i18n, { page: 'p1', document: null, reportMissing: false })
    expect(i18n.t('title')).toBe('Page')
    // step 1: p1 loses its only chunk in Rust
    delete cat.en!.p1
    fake.bump()
    await fake.deliver(0)
    await link.setPage('p1')
    expect([i18n.t('title'), i18n.storage.translations.has('en:p1')]).toEqual(['Root', false])
    // step 2: two changes, both events arrive before the first is handled: one meta
    cat.en!.index = { title: 'Root 2' }
    fake.bump()
    fake.bump()
    const metas = () => fake.ipc.filter((c) => c.endsWith('|meta')).length
    const before = metas()
    await Promise.all([fake.deliver(0), fake.deliver(0)])
    await link.setPage('p1')
    expect([metas() - before, i18n.t('title')]).toEqual([1, 'Root 2'])
    link.close()
  })

  it('a page used again after a change is the most recently used one', async () => {
    const { cat, i18n } = setup()
    const link = await connect(i18n, { maxPages: 2, document: null, reportMissing: false })
    await link.setPage('p1')
    await link.setPage('p2')
    cat.en!.index = { title: 'App 2' }
    fake.bump()
    await fake.deliver(0) // p2 shown: read again
    await link.setPage('p1') // read again: now the newest
    await link.setPage('p3')
    expect(pageViews(i18n)).toEqual(['en:p1', 'en:p3'])
    link.close()
  })

  it('load: a page about to show is read without showing it and kept; preloaded pages follow a locale switch', async () => {
    const { i18n } = setup()
    const link = await connect(i18n, { page: 'p1', maxPages: 1, document: null, reportMissing: false })
    // step 1: the guard of a navigation to p2: p1 stays shown, both views stay
    await link.load('p2')
    expect([link.page, i18n.getRoute(), i18n.t('h'), pageViews(i18n)]).toEqual(['p1', 'p1', 'One', ['en:p1', 'en:p2']])
    // step 2: the navigation is confirmed: no call
    const calls = fake.ipc.length
    await link.setPage('p2')
    expect([i18n.t('h'), fake.ipc.length - calls, pageViews(i18n)]).toEqual(['Two', 0, ['en:p2']])
    // step 3: a navigation to p3 is cancelled: p2 stays shown
    await link.load('p3')
    expect([link.page, i18n.t('h')]).toEqual(['p2', 'Two'])
    // step 4: a preloaded page is loaded for the new locale with it, ready without a call
    await link.preload('p1')
    await link.setLocale('ar')
    const after = fake.ipc.length
    expect(i18n.t('h', undefined, null, 'p1')).toBe('واحد')
    await link.setPage('p1')
    expect([i18n.t('h'), fake.ipc.length - after]).toEqual(['واحد', 0])
    link.close()
  })
})
