/**
 * Vue integration (`tauri-plugin-i18n-next-api/vue`) on top of the optional peer
 * `@i18n-micro/vue` (DESIGN 3.4, 6.2): the `createI18n` of `@i18n-micro/vue` (`$t`, `$tc`,
 * `<i18n-t>`, `useI18n`, as in nuxt-i18n-micro) started from the plugin's meta (locale,
 * `locales`, `defaultLocale`, named formats) and connected to it, plus a router guard that
 * loads the page of the route before the navigation is confirmed. Relative imports keep the
 * `.js` extension: the emitted `.d.ts` must resolve under node16 ESM.
 */
import { type I18nPlugin, createI18n } from '@i18n-micro/vue'
import { type App, type InjectionKey, inject, watch } from 'vue'
import { type ConnectOptions, type I18nInstance, type Link, type Meta, connect, meta } from './index.js'

export * from './index.js'

/** What the guard needs of a vue-router route (`RouteLocationNormalized` fits). */
export interface RouteLike {
  name?: unknown
  /** The matched records: none before the first navigation (`START_LOCATION`). */
  matched?: readonly unknown[]
}

/** What the guards need of a vue-router `Router`. */
export interface RouterLike {
  beforeResolve(guard: (to: RouteLike) => unknown): () => void
  afterEach(hook: (to: RouteLike, from: RouteLike, failure?: unknown) => unknown): () => void
  readonly currentRoute: { readonly value: RouteLike }
}

type VueOptions = Parameters<typeof createI18n>[0]

export interface TauriI18nOptions
  extends Omit<VueOptions, 'locale' | 'fallbackLocale' | 'locales' | 'defaultLocale' | 'messages'>,
    Omit<ConnectOptions, 'page'> {
  /**
   * A vue-router router: `beforeResolve` loads the page of every route before it shows,
   * `afterEach` shows it once the navigation is confirmed; the current route is loaded first.
   */
  router?: RouterLike
  /** The page of a route: its name by default (Nuxt names `pages/user/profile` `user-profile`), `index` without one. */
  pageOf?: (to: RouteLike) => string
}

/**
 * The plugin with its link to Rust; `app.use` also provides the link to `useI18nLink`. When
 * the app unmounts, the link closes and the router hooks and the locale watch go (`dispose`
 * does the same without an app).
 */
export type TauriI18nPlugin = I18nPlugin & { link: Link; dispose(): void }

const LinkKey: InjectionKey<Link> = Symbol('i18n-next link')

/** The link of the app's `createTauriI18n` plugin (`setLocale`, `preload`, `version`, …). */
export function useI18nLink(): Link {
  const link = inject(LinkKey)
  if (!link) throw new Error('tauri-plugin-i18n-next-api/vue: app.use(await createTauriI18n()) first')
  return link
}

const pageOfRoute = (to: RouteLike) => (typeof to.name === 'string' && to.name ? to.name : 'index')

/**
 * `createI18n` of `@i18n-micro/vue` fed by the plugin:
 *
 * ```ts
 * import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'
 * app.use(router)
 * app.use(await createTauriI18n({ router }))   // beforeResolve loads the page chunk
 * ```
 *
 * The locale, `locales`, `defaultLocale` and the named formats come from Rust (8b.1). A
 * failed page load keeps what is shown and lets the navigation through. A locale set on the
 * instance (`useI18n().locale.value = 'de'`, `<I18nSwitcher>`) goes to Rust like
 * `link.setLocale`; the old texts stay until the new ones are in, and a locale Rust refuses
 * puts the shown one back.
 */
export async function createTauriI18n(options: TauriI18nOptions = {}): Promise<TauriI18nPlugin> {
  const { router, pageOf = pageOfRoute, maxPages, reportMissing, transport, document, ...vue } = options
  const languages = (globalThis as { navigator?: { languages?: readonly string[] } }).navigator?.languages
  // the route already shown (the first navigation done before this call) is loaded first
  const shown = router?.currentRoute.value
  let page = shown?.matched?.length ? pageOf(shown) : 'index'
  // the zone of Rust dates: the app's own, else the WebView's (DESIGN 5.4)
  const zone = vue.timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone
  const start = { languages: [...(languages ?? [])], ...(zone ? { timeZone: zone } : {}) }
  let m: Meta
  try {
    m = await meta(page === 'index' ? start : { ...start, page })
  } catch (e) {
    if ((e as { kind?: unknown } | null)?.kind !== 'UnknownPage') throw e
    m = await meta(start) // a route the catalog has no page for: the root answers it
    page = 'index'
  }
  const plugin = createI18n({
    ...vue,
    locale: m.locale,
    fallbackLocale: m.locale,
    locales: m.locales,
    defaultLocale: m.defaultLocale,
    numberFormats: { ...m.numberFormats, ...vue.numberFormats },
    datetimeFormats: { ...m.datetimeFormats, ...vue.datetimeFormats },
    // dates in the zone Rust shows them in
    timeZone: m.timeZone ?? vue.timeZone,
  })
  const i18n = plugin.global
  const link = await connect(i18n as unknown as I18nInstance, {
    page,
    maxPages,
    reportMissing,
    transport,
    document,
    // the meta just read: `connect` starts from it instead of asking again
    [Symbol.for('tauri-plugin-i18n-next.meta')]: { meta: m, page },
  } as ConnectOptions)
  const off: Array<() => void> = []
  if (router) {
    // before the navigation is confirmed: load only, the shown page keeps its texts
    off.push(
      router.beforeResolve(async (to) => {
        try {
          await link.load(pageOf(to))
        } catch (e) {
          console.warn('[i18n-next] page not loaded:', e)
        }
      }),
    )
    // once it is: show it (no call when the guard loaded it); a cancelled or duplicated
    // navigation changes nothing
    off.push(
      router.afterEach((to, _from, failure) => {
        if (failure) return
        link.setPage(pageOf(to)).catch((e) => console.warn('[i18n-next] page not shown:', e))
      }),
    )
  }
  // locales asked for through the instance and not shown yet, oldest first: when one of them
  // lands it is no new ask, and a later ask is never dropped for an older one
  let asks: string[] = []
  off.push(
    watch(i18n.locale, (code) => {
      const at = asks.indexOf(code)
      if (code === link.locale && at >= 0) {
        asks = asks.slice(at + 1)
        return
      }
      if (code === (asks[asks.length - 1] ?? link.locale)) return
      asks.push(code)
      // keep showing the old locale until the new one is loaded (the view holds its chain)
      i18n.fallbackLocale = link.locale
      link.setLocale(code).catch((e) => {
        console.warn('[i18n-next] locale not switched:', e)
        // refused (an unknown locale, say): back to what is shown, unless a newer ask waits
        asks = asks.filter((c) => c !== code)
        if (asks.length > 0) return
        i18n.locale.value = link.locale
        i18n.fallbackLocale = link.locale
      })
    }),
  )
  const dispose = () => {
    for (const f of off.splice(0)) f()
    link.close()
  }
  const install = (plugin as { install: (app: App, ...rest: unknown[]) => void }).install.bind(plugin)
  return Object.assign(plugin, {
    link,
    dispose,
    install(app: App, ...rest: unknown[]) {
      install(app, ...rest)
      app.provide(LinkKey, link)
      app.onUnmount?.(dispose) // Vue 3.5+
    },
  })
}
