// @vitest-environment happy-dom
// `createTauriI18n` in a Vue app with vue-router (DESIGN 3.4): the route's page is loaded
// before the navigation is confirmed, so raw keys never show; `$t`, `$tc` and `<i18n-t>`
// read the Rust catalog; a locale set on the instance goes to Rust and the old texts stay
// until the new ones are in; a failed load lets the navigation through.
import { createApp, defineComponent, h, nextTick, resolveComponent } from 'vue'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'
import { afterEach, describe, expect, it } from 'vitest'
import { createTauriI18n, useI18nLink } from '../../guest-js/vue.js'
import { type Catalog, FakePlugin, catalogSource } from './fake.js'

const cat: Catalog = {
  en: {
    index: { title: 'Home', apples: 'no apples | one apple | {count} apples', total: 'Total: {sum}' },
    settings: { title: 'Settings' },
  },
  ar: { index: { title: 'الرئيسية' }, settings: { title: 'الإعدادات' } },
}

let fake: FakePlugin
afterEach(() => fake.uninstall())

const Page = defineComponent({
  render() {
    const I18nT = resolveComponent('i18n-t')
    return h('main', [
      h('h1', String(this.$t('title'))),
      h('p', this.$tc('apples', 3)),
      h(I18nT, { keypath: 'total', tag: 'span', params: { sum: this.$tn(12.5, 'money') } }),
    ])
  },
})

describe('createTauriI18n with vue-router', () => {
  it('loads the page before the route shows, switches locales through Rust, survives a failed load', async () => {
    fake = new FakePlugin(
      catalogSource(structuredClone(cat), { en: ['en'], ar: ['ar', 'en'] }, ['index', 'settings'], {
        locales: [{ code: 'en', iso: 'en-US' }, { code: 'ar', iso: 'ar-EG', dir: 'rtl' }],
        numberFormats: { en: { money: { style: 'currency', currency: 'USD' } }, ar: { money: { style: 'currency', currency: 'EGP' } } },
      }),
      'en',
    )
    fake.install()
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/', name: 'index', component: Page },
        { path: '/settings', name: 'settings', component: Page },
        { path: '/ghost', name: 'ghost', component: Page },
      ],
    })
    const i18n = await createTauriI18n({ router, reportMissing: false, document: null })
    let link: ReturnType<typeof useI18nLink> | undefined
    const app = createApp(
      defineComponent({
        setup() {
          link = useI18nLink()
          return () => h(RouterView)
        },
      }),
    )
    app.use(router)
    app.use(i18n)
    const el = document.createElement('div')
    // step 1: the first route: the root and its formats (en-US, USD) from Rust
    await router.push('/')
    app.mount(el)
    expect(link).toBe(i18n.link)
    expect(el.textContent).toBe('Home3 applesTotal: $12.50')
    // step 2: the guard loads `settings` before the route is confirmed: no raw key in between
    const seen: string[] = []
    router.afterEach(() => {
      seen.push(i18n.global.t('title') as string)
    })
    await router.push('/settings')
    await nextTick()
    expect([seen, el.textContent]).toEqual([['Settings'], 'Settings3 applesTotal: $12.50'])
    // step 3: a locale set on the instance reaches Rust; old texts until the new are in
    i18n.global.locale.value = 'ar'
    await nextTick()
    expect(el.textContent!.startsWith('Settings')).toBe(true)
    await i18n.link.setPage('settings') // waits for the switch
    await nextTick()
    expect([fake.locale, el.querySelector('h1')!.textContent]).toEqual(['ar', 'الإعدادات'])
    // step 4: Rust fails on the next page: the navigation goes through, texts stay
    const meta = fake.source.meta
    fake.source.meta = () => {
      throw new Error('boom')
    }
    const warn = console.warn
    const warned: unknown[] = []
    console.warn = (...a: unknown[]) => warned.push(a[0])
    await router.push('/ghost')
    await nextTick()
    console.warn = warn
    fake.source.meta = meta
    expect([router.currentRoute.value.name, warned, el.querySelector('h1')!.textContent]).toEqual([
      'ghost',
      ['[i18n-next] page not loaded:'],
      'الإعدادات',
    ])
    // step 5: a route the catalog has no page for reads the root
    await router.push('/')
    await router.push('/ghost')
    await nextTick()
    expect([i18n.link.page, el.querySelector('h1')!.textContent]).toEqual(['ghost', 'الرئيسية'])
    app.unmount()
    i18n.link.close()
  })

  it('loads a deep link before the first render, shows a page only once its navigation is confirmed, cleans up on unmount', async () => {
    const pages: Catalog = {
      en: { index: { title: 'Home' }, p1: { title: 'One' }, p2: { title: 'Two' }, p3: { title: 'Three' } },
      ar: { index: { title: 'الرئيسية' }, p1: { title: 'واحد' } },
    }
    fake = new FakePlugin(catalogSource(pages, { en: ['en'], ar: ['ar', 'en'] }, ['index', 'p1', 'p2', 'p3']), 'en')
    fake.install()
    const router = createRouter({
      history: createMemoryHistory(),
      routes: ['index', 'p1', 'p2', 'p3'].map((name) => ({ path: name === 'index' ? '/' : `/${name}`, name, component: Page })),
    })
    // step 1: the app was opened on /p1 (the first navigation is done): p1 is loaded at once, no miss
    await router.push('/p1')
    const i18n = await createTauriI18n({ router, maxPages: 1, document: null })
    const app = createApp(defineComponent({ setup: () => () => h(RouterView) }))
    app.use(router)
    app.use(i18n)
    const el = document.createElement('div')
    app.mount(el)
    await nextTick()
    const h1 = () => el.querySelector('h1')!.textContent
    expect([h1(), i18n.link.page, fake.ipc.filter((c) => c.endsWith('|meta')).length]).toEqual(['One', 'p1', 1])
    // step 2: a guard after ours cancels the navigation to p2: p1 stays shown with its texts and its view
    const seen: string[] = []
    const stop = router.beforeResolve(async (to) => {
      await nextTick()
      seen.push(h1()!)
      return to.name !== 'p2'
    })
    await router.push('/p2')
    await nextTick()
    expect([seen, router.currentRoute.value.name, i18n.link.page, h1(), i18n.global.storage.translations.has('en:p1')]).toEqual([
      ['One'],
      'p1',
      'p1',
      'One',
      true,
    ])
    // step 3: p3 is confirmed: shown in the same tick the route changes, the old page goes (maxPages 1)
    const shown: string[] = []
    const after = router.afterEach(() => void shown.push(String(i18n.global.t('title'))))
    await router.push('/p3')
    await nextTick()
    expect([shown, h1(), [...i18n.global.storage.translations.keys()].filter((k) => !k.endsWith(':index'))]).toEqual([['Three'], 'Three', ['en:p3']])
    // step 4: the same route again (a duplicated navigation): no call
    const calls = fake.ipc.length
    await router.push('/p3')
    expect([fake.ipc.length - calls, i18n.link.page]).toEqual([0, 'p3'])
    stop()
    after()
    // step 5: p3 loses its chunk in Rust: the page re-renders from the root
    delete pages.en!.p3
    fake.bump()
    await fake.deliver(0)
    await i18n.link.setPage('p3')
    await nextTick()
    expect(h1()).toBe('Home')
    // step 6: ar, then en before ar is in: en wins; a locale Rust refuses puts the shown one back
    i18n.global.locale.value = 'ar'
    await nextTick()
    i18n.global.locale.value = 'en'
    await nextTick()
    await i18n.link.refresh()
    await nextTick()
    expect([fake.locale, i18n.link.locale, i18n.global.locale.value, h1()]).toEqual(['en', 'en', 'en', 'Home'])
    const warn = console.warn
    console.warn = () => {}
    i18n.global.locale.value = 'xx'
    await nextTick()
    await i18n.link.refresh().catch(() => {})
    await nextTick()
    console.warn = warn
    expect([fake.locale, i18n.global.locale.value, i18n.global.fallbackLocale.value, h1()]).toEqual(['en', 'en', 'en', 'Home'])
    // step 7: unmount: no more events, guards or locale watch
    app.unmount()
    const before = fake.ipc.length
    fake.bump()
    await fake.deliver(0)
    await router.push('/p1')
    i18n.global.locale.value = 'ar'
    await nextTick()
    expect([fake.ipc.length - before, fake.locale]).toEqual([0, 'en'])
  })
})
