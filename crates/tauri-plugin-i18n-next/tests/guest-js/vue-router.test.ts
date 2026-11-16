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
})
