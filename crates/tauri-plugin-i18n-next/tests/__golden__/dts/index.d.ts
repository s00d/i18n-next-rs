import { type Tree } from './merge.js';
/** One locale of `meta().locales` (`Locale` of `@i18n-micro/types`). */
export interface Locale {
    code: string;
    iso?: string;
    dir?: 'ltr' | 'rtl' | 'auto';
    displayName?: string;
    [key: string]: unknown;
}
/** What the `meta` command returns (DESIGN 6.1). */
export interface Meta {
    /** The catalog version: grows with every change (event `i18n-next://changed`). */
    version: number;
    /** The locale the tags are for. */
    locale: string;
    defaultLocale: string;
    fallbackLocale: string | null;
    locales: Locale[];
    /** The fallback chain of every locale, strongest first: premerge a view in this order. */
    chains: Record<string, string[]>;
    numberFormats: Record<string, Record<string, Intl.NumberFormatOptions>>;
    datetimeFormats: Record<string, Record<string, Intl.DateTimeFormatOptions>>;
    /**
     * The IANA zone Rust shows dates in, when it is known by name (the catalog's
     * `time_zone`, or the WebView's zone sent with the first `meta`); `null`: the system's.
     * `connect` and `createTauriI18n` format dates in it too.
     */
    timeZone: string | null;
    /** The content tag of each chunk of the chain, by `"<locale>/<page>"`: `/{tag}/{locale}/{page}.json`. */
    tags: Record<string, string>;
    /** The tag of the protocol bundle: `/{bundle}/bundle?locale=…&page=…`. */
    bundle: string;
}
/** The event every window gets after a change (DESIGN 6.1). */
export declare const EVENT_CHANGED = "i18n-next://changed";
/** The dev event of the missing-key report: one `{ locale, page, key, count }` per new key. */
export declare const EVENT_MISSING = "i18n-next://missing";
/**
 * Asks the plugin for the meta of `locale` (the active one by default) and `page`. The
 * command answers raw JSON bytes (no serde round trip): they are parsed here. `languages`
 * (`navigator.languages`) lets the first call pick the start locale (8b.3); `timeZone`
 * (the WebView's IANA zone) becomes the zone of Rust dates unless the catalog has one.
 */
export declare function meta(options?: {
    locale?: string;
    page?: string;
    languages?: readonly string[];
    timeZone?: string;
}): Promise<Meta>;
/**
 * What `connect` needs of an `@i18n-micro` instance: `createI18n` of `@i18n-micro/runtime`,
 * `/vue`, `/react`, `/preact` and `/solid` all build one.
 */
export interface I18nInstance {
    /** Takes a locale code (Vue's instance answers a `Ref`, the others a string). */
    locale: unknown;
    fallbackLocale: unknown;
    setRoute(page: string): void;
    addTranslations(locale: string, translations: Tree, merge?: boolean): void;
    addRouteTranslations(locale: string, page: string, translations: Tree, merge?: boolean): void;
    readonly storage: {
        translations: Map<string, unknown>;
    };
    missingHandler?: (locale: string, key: string, page: string) => void;
    formatter?: {
        getNumberFormats(): Record<string, unknown>;
        setNumberFormats(f: Record<string, unknown>): void;
        getDateTimeFormats(): Record<string, unknown>;
        setDateTimeFormats(f: Record<string, unknown>): void;
    };
    addLocaleIso?(iso: Record<string, string>): void;
}
export interface ConnectOptions {
    /** The page shown first (`index`, the root alone, by default). */
    page?: string;
    /** The page chunks kept loaded (8b.16); the current page and preloaded ones stay. Unbounded by default. */
    maxPages?: number;
    /**
     * Sends keys the instance misses to `report_missing` (8b.7), batched per microtask. The
     * command exists in debug builds only: the first refusal turns reporting off. Default `true`.
     */
    reportMissing?: boolean;
    /** `auto` (default): the protocol, raw invoke once it fails. */
    transport?: 'auto' | 'protocol' | 'invoke';
    /** Sets `<html lang dir>` of this document on every locale (default: the global one; `null`: never). */
    document?: Document | null;
}
/** A connected instance (DESIGN 3.4, 6.2). Every method waits for the ones before it. */
export interface Link {
    /** The active locale the instance shows. */
    readonly locale: string;
    /** The page the instance shows (`index`: the root alone). */
    readonly page: string;
    /** The catalog version the instance shows. */
    readonly version: number;
    /** The last meta read. */
    readonly meta: Meta;
    /** `true` while the protocol is out and raw invoke carries the data. */
    readonly fallback: boolean;
    /** Loads the page's chunks of the active chain, then shows the page (`setRoute`). */
    setPage(page: string): Promise<void>;
    /** Switches Rust, native labels and every window; resolves once this instance shows it. */
    setLocale(locale: string): Promise<void>;
    /** Loads a page without showing it; it is never evicted and follows every locale. */
    preload(page: string): Promise<void>;
    /**
     * Loads a page about to be shown (a router guard, before the navigation is confirmed)
     * without showing it: the shown page and its texts stay. It is kept until the next
     * `setPage` or `load`, which a cancelled navigation may never bring.
     */
    load(page: string): Promise<void>;
    /** Reads the meta again and loads what changed. */
    refresh(): Promise<void>;
    /** Stops following `changed` and reporting misses; gives the instance its `missingHandler` back. */
    close(): void;
}
/**
 * Connects `i18n` to the plugin: loads the active locale's root (and `page`), follows
 * `changed` from Rust and other windows (late or out-of-order events are dropped by version),
 * reports misses in dev and keeps `<html lang dir>` in step.
 *
 * ```ts
 * import { createI18n } from '@i18n-micro/runtime'
 * const link = await connect(createI18n({ locale: 'en' }), { page: 'index' })
 * await link.setPage('settings')
 * await link.setLocale('de')
 * ```
 */
export declare function connect(i18n: I18nInstance, options?: ConnectOptions): Promise<Link>;
