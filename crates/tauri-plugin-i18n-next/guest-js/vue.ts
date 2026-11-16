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
import { type ConnectOptions, type I18nInstance, type Link, connect, meta } from './index.js'

export * from './index.js'

/** What the guard needs of a vue-router route (`RouteLocationNormalized` fits). */
export interface RouteLike {
  name?: unknown
}

/** What the guard needs of a vue-router `Router`. */
export interface RouterLike {
  beforeResolve(guard: (to: RouteLike) => unknown): unknown
}

type VueOptions = Parameters<typeof createI18n>[0]

export interface TauriI18nOptions
  extends Omit<VueOptions, 'locale' | 'fallbackLocale' | 'locales' | 'defaultLocale' | 'messages'>,
    Omit<ConnectOptions, 'page'> {
  /** A vue-router router: its `beforeResolve` loads the page of every route before it shows. */
  router?: RouterLike
  /** The page of a route: its name by default (Nuxt names `pages/user/profile` `user-profile`), `index` without one. */
  pageOf?: (to: RouteLike) => string
}

/** The plugin with its link to Rust; `app.use` also provides the link to `useI18nLink`. */
export type TauriI18nPlugin = I18nPlugin & { link: Link }

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
 * `link.setLocale`; the old texts stay until the new ones are in.
 */
export async function createTauriI18n(options: TauriI18nOptions = {}): Promise<TauriI18nPlugin> {
  const { router, pageOf = pageOfRoute, maxPages, reportMissing, transport, document, ...vue } = options
  const languages = (globalThis as { navigator?: { languages?: readonly string[] } }).navigator?.languages
  const m = await meta({ languages: [...(languages ?? [])] })
  const plugin = createI18n({
    ...vue,
    locale: m.locale,
    fallbackLocale: m.locale,
    locales: m.locales,
    defaultLocale: m.defaultLocale,
    numberFormats: { ...m.numberFormats, ...vue.numberFormats },
    datetimeFormats: { ...m.datetimeFormats, ...vue.datetimeFormats },
  })
  const i18n = plugin.global
  const link = await connect(i18n as unknown as I18nInstance, { maxPages, reportMissing, transport, document })
  router?.beforeResolve(async (to) => {
    try {
      await link.setPage(pageOf(to))
    } catch (e) {
      console.warn('[i18n-next] page not loaded:', e)
    }
  })
  watch(i18n.locale, (code) => {
    if (code !== link.locale) {
      // keep showing the old locale until the new one is loaded (the view holds its chain)
      i18n.fallbackLocale = link.locale
      link.setLocale(code).catch((e) => console.warn('[i18n-next] locale not switched:', e))
    }
  })
  const install = (plugin as { install: (app: App, ...rest: unknown[]) => void }).install.bind(plugin)
  return Object.assign(plugin, {
    link,
    install(app: App, ...rest: unknown[]) {
      install(app, ...rest)
      app.provide(LinkKey, link)
    },
  })
}
