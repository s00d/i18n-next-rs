/**
 * Vue integration (`tauri-plugin-i18n-next-api/vue`) on top of the optional peer
 * `@i18n-micro/vue` (DESIGN 3.4, 6.2): the `createI18n` of `@i18n-micro/vue` (`$t`, `$tc`,
 * `<i18n-t>`, `useI18n`, as in nuxt-i18n-micro) started from the plugin's meta (locale,
 * `locales`, `defaultLocale`, named formats) and connected to it, plus a router guard that
 * loads the page of the route before the navigation is confirmed. Relative imports keep the
 * `.js` extension: the emitted `.d.ts` must resolve under node16 ESM.
 */
import { type I18nPlugin, createI18n } from '@i18n-micro/vue';
import { type ConnectOptions, type Link } from './index.js';
export * from './index.js';
/** What the guard needs of a vue-router route (`RouteLocationNormalized` fits). */
export interface RouteLike {
    name?: unknown;
    /** The matched records: none before the first navigation (`START_LOCATION`). */
    matched?: readonly unknown[];
}
/** What the guards need of a vue-router `Router`. */
export interface RouterLike {
    beforeResolve(guard: (to: RouteLike) => unknown): () => void;
    afterEach(hook: (to: RouteLike, from: RouteLike, failure?: unknown) => unknown): () => void;
    readonly currentRoute: {
        readonly value: RouteLike;
    };
}
type VueOptions = Parameters<typeof createI18n>[0];
export interface TauriI18nOptions extends Omit<VueOptions, 'locale' | 'fallbackLocale' | 'locales' | 'defaultLocale' | 'messages'>, Omit<ConnectOptions, 'page'> {
    /**
     * A vue-router router: `beforeResolve` loads the page of every route before it shows,
     * `afterEach` shows it once the navigation is confirmed; the current route is loaded first.
     */
    router?: RouterLike;
    /** The page of a route: its name by default (Nuxt names `pages/user/profile` `user-profile`), `index` without one. */
    pageOf?: (to: RouteLike) => string;
}
/**
 * The plugin with its link to Rust; `app.use` also provides the link to `useI18nLink`. When
 * the app unmounts, the link closes and the router hooks and the locale watch go (`dispose`
 * does the same without an app).
 */
export type TauriI18nPlugin = I18nPlugin & {
    link: Link;
    dispose(): void;
};
/** The link of the app's `createTauriI18n` plugin (`setLocale`, `preload`, `version`, …). */
export declare function useI18nLink(): Link;
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
export declare function createTauriI18n(options?: TauriI18nOptions): Promise<TauriI18nPlugin>;
