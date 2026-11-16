/**
 * Frontend API of tauri-plugin-i18n-next (DESIGN 6.2): `meta` and `connect`, which feeds an
 * `@i18n-micro` instance (runtime, Vue, React, Preact, Solid) from the Rust catalog.
 *
 * Data comes over the `i18n-next://` protocol (`fetch(convertFileSrc(…)).json()`, content-
 * tagged immutable URLs), control over commands. When the protocol is unavailable, the raw
 * `load` / `load_page` commands carry the same chunks. Relative imports keep the `.js`
 * extension: the emitted `.d.ts` must resolve under node16 ESM.
 */
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { type Chunk, type Tree, pageView, premerge } from './merge.js'

/** One locale of `meta().locales` (`Locale` of `@i18n-micro/types`). */
export interface Locale {
  code: string
  iso?: string
  dir?: 'ltr' | 'rtl' | 'auto'
  displayName?: string
  [key: string]: unknown
}

/** What the `meta` command returns (DESIGN 6.1). */
export interface Meta {
  /** The catalog version: grows with every change (event `i18n-next://changed`). */
  version: number
  /** The locale the tags are for. */
  locale: string
  defaultLocale: string
  fallbackLocale: string | null
  locales: Locale[]
  /** The fallback chain of every locale, strongest first: premerge a view in this order. */
  chains: Record<string, string[]>
  numberFormats: Record<string, Record<string, Intl.NumberFormatOptions>>
  datetimeFormats: Record<string, Record<string, Intl.DateTimeFormatOptions>>
  /** The content tag of each chunk of the chain, by `"<locale>/<page>"`: `/{tag}/{locale}/{page}.json`. */
  tags: Record<string, string>
  /** The tag of the protocol bundle: `/{bundle}/bundle?locale=…&page=…`. */
  bundle: string
}

/** The event every window gets after a change (DESIGN 6.1). */
export const EVENT_CHANGED = 'i18n-next://changed'
/** The dev event of the missing-key report: one `{ locale, page, key, count }` per new key. */
export const EVENT_MISSING = 'i18n-next://missing'

const SCHEME = 'i18n-next'
const cmd = (name: string) => `plugin:i18n-next|${name}`
const parse = <T>(raw: ArrayBuffer): T => JSON.parse(new TextDecoder().decode(raw)) as T

/**
 * Asks the plugin for the meta of `locale` (the active one by default) and `page`. The
 * command answers raw JSON bytes (no serde round trip): they are parsed here. `languages`
 * (`navigator.languages`) lets the first call pick the start locale (8b.3).
 */
export async function meta(
  options: { locale?: string; page?: string; languages?: readonly string[] } = {},
): Promise<Meta> {
  return parse<Meta>(await invoke<ArrayBuffer>(cmd('meta'), options))
}

/**
 * What `connect` needs of an `@i18n-micro` instance: `createI18n` of `@i18n-micro/runtime`,
 * `/vue`, `/react`, `/preact` and `/solid` all build one.
 */
export interface I18nInstance {
  /** Takes a locale code (Vue's instance answers a `Ref`, the others a string). */
  locale: unknown
  fallbackLocale: unknown
  setRoute(page: string): void
  addTranslations(locale: string, translations: Tree, merge?: boolean): void
  addRouteTranslations(locale: string, page: string, translations: Tree, merge?: boolean): void
  readonly storage: { translations: Map<string, unknown> }
  missingHandler?: (locale: string, key: string, page: string) => void
  formatter?: {
    getNumberFormats(): Record<string, unknown>
    setNumberFormats(f: Record<string, unknown>): void
    getDateTimeFormats(): Record<string, unknown>
    setDateTimeFormats(f: Record<string, unknown>): void
  }
  addLocaleIso?(iso: Record<string, string>): void
}

export interface ConnectOptions {
  /** The page shown first (`index`, the root alone, by default). */
  page?: string
  /** The page chunks kept loaded (8b.16); the current page and preloaded ones stay. Unbounded by default. */
  maxPages?: number
  /**
   * Sends keys the instance misses to `report_missing` (8b.7), batched per microtask. The
   * command exists in debug builds only: the first refusal turns reporting off. Default `true`.
   */
  reportMissing?: boolean
  /** `auto` (default): the protocol, raw invoke once it fails. */
  transport?: 'auto' | 'protocol' | 'invoke'
  /** Sets `<html lang dir>` of this document on every locale (default: the global one; `null`: never). */
  document?: Document | null
}

/** A connected instance (DESIGN 3.4, 6.2). Every method waits for the ones before it. */
export interface Link {
  /** The active locale the instance shows. */
  readonly locale: string
  /** The page the instance shows (`index`: the root alone). */
  readonly page: string
  /** The catalog version the instance shows. */
  readonly version: number
  /** The last meta read. */
  readonly meta: Meta
  /** `true` while the protocol is out and raw invoke carries the data. */
  readonly fallback: boolean
  /** Loads the page's chunks of the active chain, then shows the page (`setRoute`). */
  setPage(page: string): Promise<void>
  /** Switches Rust, native labels and every window; resolves once this instance shows it. */
  setLocale(locale: string): Promise<void>
  /** Loads a page without showing it; it is never evicted. */
  preload(page: string): Promise<void>
  /** Reads the meta again and loads what changed. */
  refresh(): Promise<void>
  /** Stops following `changed` and reporting misses. */
  close(): void
}

interface View {
  /** The tags it was built from, in layer order (`[]`: a page without chunks, read from the root view). */
  tags: string[]
  /** The catalog version it was checked against. */
  version: number
}

interface Load {
  version: number
  locale: string
  chunks: Array<Chunk & { locale: string; page: string }>
}

const kindOf = (e: unknown) => (typeof e === 'object' && e !== null ? (e as { kind?: unknown }).kind : undefined)

/** The catalog moved between `meta` and a read: start over from a new `meta`. */
class Stale extends Error {}

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
export async function connect(i18n: I18nInstance, options: ConnectOptions = {}): Promise<Link> {
  const maxPages = options.maxPages ?? Infinity
  const doc = options.document === undefined ? (globalThis as { document?: Document }).document : options.document
  let transport: 'protocol' | 'invoke' = options.transport === 'invoke' ? 'invoke' : 'protocol'
  let current: Meta | undefined
  let formats = ''
  let version = -1
  let locale = ''
  let page = 'index'
  let closed = false
  let busy = 0
  const chunks = new Map<string, Chunk>() // by content tag
  const views = new Map<string, View>() // by `locale:page`, least recently used first
  const pinned = new Set<string>()

  // every operation runs after the one before: events and calls apply in order
  let queue: Promise<unknown> = Promise.resolve()
  const enqueue = <T>(op: () => Promise<T>): Promise<T> => {
    busy++
    const run = queue.then(op).finally(() => busy--)
    queue = run.catch(() => {})
    return run
  }

  async function fetchChunk(tag: string, key: string): Promise<Chunk> {
    const res = await fetch(convertFileSrc(`${tag}/${key}.json`, SCHEME))
    if (res.status === 410) throw new Stale(key)
    if (!res.ok) throw new Error(`i18n-next: ${res.status} for ${key}`)
    const c = (await res.json()) as Chunk
    chunks.set(tag, c)
    return c
  }

  /** The chunks of `m` by `"locale/page"`: cached ones by tag, the rest read. */
  async function read(m: Meta, pg: string): Promise<Map<string, Chunk>> {
    const got = new Map<string, Chunk>()
    const missing = Object.keys(m.tags).filter((key) => {
      const c = chunks.get(m.tags[key]!)
      if (c) got.set(key, c)
      return !c
    })
    if (missing.length === 0) return got
    if (transport === 'protocol') {
      const links = missing.map((key) => hint(convertFileSrc(`${m.tags[key]}/${key}.json`, SCHEME)))
      try {
        const all = await Promise.all(missing.map((key) => fetchChunk(m.tags[key]!, key)))
        missing.forEach((key, i) => got.set(key, all[i]!))
        return got
      } catch (e) {
        if (e instanceof Stale || options.transport === 'protocol') throw e
        transport = 'invoke' // no protocol here (or not for this origin): raw invoke from now on
      } finally {
        for (const l of links) l?.remove()
      }
    }
    const payload = parse<Load>(await invoke<ArrayBuffer>(cmd('load'), pg === 'index' ? {} : { page: pg }))
    if (payload.version !== m.version || payload.locale !== m.locale) throw new Stale()
    for (const c of payload.chunks) {
      const key = `${c.locale}/${c.page}`
      const chunk: Chunk = c.linked ? { data: c.data, linked: c.linked } : { data: c.data }
      got.set(key, chunk)
      if (m.tags[key]) chunks.set(m.tags[key], chunk)
    }
    return got
  }

  /** `<link rel="preload">` for a chunk URL about to be read: the fetch takes the preloaded response. */
  function hint(href: string): HTMLLinkElement | undefined {
    if (!doc?.head) return undefined
    const l = doc.createElement('link')
    l.rel = 'preload'
    l.as = 'fetch'
    l.crossOrigin = 'anonymous'
    l.href = href
    doc.head.appendChild(l)
    return l
  }

  /** Builds and stores the view of `pg` (`index`: the root) unless its chunks are the same. */
  function store(m: Meta, got: Map<string, Chunk>, pg: string) {
    const keys: string[] = []
    for (const c of [...(m.chains[m.locale] ?? [m.locale])].reverse()) {
      for (const p of pg === 'index' ? ['index'] : ['index', pg]) {
        if (got.has(`${c}/${p}`)) keys.push(`${c}/${p}`)
      }
    }
    const id = `${m.locale}:${pg}`
    const old = views.get(id)
    views.delete(id) // re-inserted last: the most recently used
    // a page without chunks of its own reads the root view: nothing to store
    if (pg !== 'index' && keys.every((k) => k.endsWith('/index'))) {
      if (old?.tags.length) i18n.storage.translations.delete(id)
      views.set(id, { tags: [], version: m.version })
      return
    }
    const tags = keys.map((k) => m.tags[k]!)
    views.set(id, { tags, version: m.version })
    if (old && old.tags.join() === tags.join()) return
    const chain = keys.map((k) => ({ ...got.get(k)!, page: !k.endsWith('/index') }))
    if (pg === 'index') i18n.addTranslations(m.locale, premerge(chain), false)
    // a page view holds only what the page changes: the instance reads the root view after it
    else i18n.addRouteTranslations(m.locale, pg, pageView(chain), false)
  }

  /** Drops the least recently used page views past `maxPages` (8b.16) and chunks no view reads. */
  function evict() {
    let pages = 0
    // a page without chunks of its own holds nothing: it does not count
    for (const [id, v] of views) if (!id.endsWith(':index') && v.tags.length) pages++
    for (const [id, v] of views) {
      if (pages <= maxPages) break
      const at = id.lastIndexOf(':')
      const [loc, pg] = [id.slice(0, at), id.slice(at + 1)]
      if (pg === 'index' || !v.tags.length || (loc === locale && (pg === page || pinned.has(pg)))) continue
      views.delete(id)
      i18n.storage.translations.delete(id)
      pages--
    }
    const live = new Set<string>()
    for (const v of views.values()) for (const t of v.tags) live.add(t)
    for (const t of chunks.keys()) if (!live.has(t)) chunks.delete(t)
  }

  function applyMeta(m: Meta) {
    current = m
    if (m.version > version) version = m.version
    const f = JSON.stringify([m.numberFormats, m.datetimeFormats, m.locales])
    if (f === formats) return
    formats = f
    const fmt = i18n.formatter
    if (fmt) {
      fmt.setNumberFormats({ ...fmt.getNumberFormats(), ...m.numberFormats })
      fmt.setDateTimeFormats({ ...fmt.getDateTimeFormats(), ...m.datetimeFormats })
    }
    const iso: Record<string, string> = {}
    for (const l of m.locales) if (l.iso) iso[l.code] = l.iso
    i18n.addLocaleIso?.(iso)
  }

  /** Reads the meta of the active locale and `pg`, loads what changed, then shows it. */
  async function show(pg: string, route: boolean, extra: { languages?: string[] } = {}, attempt = 0): Promise<void> {
    let m: Meta
    try {
      m = await meta(pg === 'index' ? extra : { ...extra, page: pg })
    } catch (e) {
      // a route the catalog has no page for: the root alone answers it
      if (kindOf(e) !== 'UnknownPage') throw e
      m = await meta(extra)
    }
    let got: Map<string, Chunk>
    try {
      got = await read(m, pg)
    } catch (e) {
      // ponytail: 3 retries; a catalog that changes faster than a read is a bug elsewhere
      if (e instanceof Stale && attempt < 3) return show(pg, route, {}, attempt + 1)
      throw e
    }
    applyMeta(m)
    store(m, got, 'index')
    if (pg !== 'index') store(m, got, pg)
    if (m.locale !== locale) {
      locale = m.locale
      i18n.locale = m.locale
      i18n.fallbackLocale = m.locale // the view holds the whole chain
      const info = m.locales.find((l) => l.code === m.locale)
      if (doc?.documentElement) {
        doc.documentElement.lang = info?.iso ?? m.locale
        doc.documentElement.dir = info?.dir ?? 'ltr'
      }
    }
    if (route) {
      page = pg
      i18n.setRoute(pg)
    }
    evict()
  }

  /** `show`, without any call while the page's view is checked at the current version (3.4). */
  async function visit(pg: string, route: boolean) {
    const id = `${locale}:${pg}`
    const v = views.get(id)
    if (views.get(`${locale}:index`)?.version === version && (pg === 'index' || v?.version === version)) {
      if (v) {
        views.delete(id)
        views.set(id, v)
      }
      if (route) {
        page = pg
        i18n.setRoute(pg)
      }
      return
    }
    await show(pg, route)
  }

  // misses: deduped by (locale, page, key), sent per microtask
  let reporting = options.reportMissing ?? true
  const seen = new Set<string>()
  let batch: Array<{ locale: string; page: string; key: string }> = []
  const previous = i18n.missingHandler
  i18n.missingHandler = (loc, key, pg) => {
    previous?.(loc, key, pg)
    // a miss while chunks are on their way is not one
    if (!reporting || closed || busy > 0) return
    const id = `${loc}\u0000${pg}\u0000${key}`
    if (seen.has(id)) return
    seen.add(id)
    if (batch.length === 0) {
      queueMicrotask(() => {
        const keys = batch
        batch = []
        invoke(cmd('report_missing'), { keys }).catch(() => {
          reporting = false // a release build: the command is not registered
        })
      })
    }
    batch.push({ locale: loc, page: pg, key })
  }

  const languages = (globalThis as { navigator?: { languages?: readonly string[] } }).navigator?.languages
  await enqueue(() => show(options.page ?? 'index', true, { languages: [...(languages ?? [])] }))

  const unlisten = await listen<{ locale: string; version: number }>(EVENT_CHANGED, (e) => {
    const v = e.payload.version
    if (v <= version) return // late: a newer state is shown already
    enqueue(async () => {
      if (closed || v <= version) return
      await show(page, true)
      for (const p of pinned) await visit(p, false)
    }).catch(() => {})
  })

  return {
    get locale() {
      return locale
    },
    get page() {
      return page
    },
    get version() {
      return version
    },
    get meta() {
      return current!
    },
    get fallback() {
      return transport === 'invoke'
    },
    setPage: (pg) => enqueue(() => visit(pg, true)),
    setLocale: (code) =>
      enqueue(async () => {
        await invoke<number>(cmd('set_locale'), { locale: code })
        await show(page, true)
      }),
    preload: (pg) =>
      enqueue(async () => {
        pinned.add(pg)
        await visit(pg, false)
      }),
    refresh: () => enqueue(() => show(page, true)),
    close() {
      closed = true
      unlisten()
      i18n.missingHandler = previous
    },
  }
}
