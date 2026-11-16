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
  /**
   * The IANA zone Rust shows dates in, when it is known by name (the catalog's
   * `time_zone`, or the WebView's zone sent with the first `meta`); `null`: the system's.
   * `connect` and `createTauriI18n` format dates in it too.
   */
  timeZone: string | null
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
 * (`navigator.languages`) lets the first call pick the start locale (8b.3); `timeZone`
 * (the WebView's IANA zone) becomes the zone of Rust dates unless the catalog has one.
 */
export async function meta(
  options: { locale?: string; page?: string; languages?: readonly string[]; timeZone?: string } = {},
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
  /** Loads a page without showing it; it is never evicted and follows every locale. */
  preload(page: string): Promise<void>
  /**
   * Loads a page about to be shown (a router guard, before the navigation is confirmed)
   * without showing it: the shown page and its texts stay. It is kept until the next
   * `setPage` or `load`, which a cancelled navigation may never bring.
   */
  load(page: string): Promise<void>
  /** Reads the meta again and loads what changed. */
  refresh(): Promise<void>
  /** Stops following `changed` and reporting misses; gives the instance its `missingHandler` back. */
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

/** The zone of a formatter (`FormatService` keeps it in a field it has no setter for). */
type Zoned = { timeZone?: string }

const kindOf = (e: unknown) => (typeof e === 'object' && e !== null ? (e as { kind?: unknown }).kind : undefined)

/** The catalog moved between `meta` and a read: start over from a new `meta`. */
class Stale extends Error {}

/** The meta of a page read before `connect` (by `createTauriI18n`), not a public option. */
const PRIMED = Symbol.for('tauri-plugin-i18n-next.meta')

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
  let pending: string | undefined // `load`ed, not shown yet
  // `createTauriI18n` read the start meta already: the first `show` of the root takes it
  let primed = (options as { [PRIMED]?: { meta: Meta; page: string } })[PRIMED]

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
    if (res.status === 410) throw new Stale(`i18n-next: chunk ${key} changed while it was read`)
    // an HTTP error is an answer of the protocol: a failed read, not a missing protocol
    if (!res.ok) throw new Error(`i18n-next: ${res.status} for chunk ${key}`)
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
      try {
        const all = await Promise.all(missing.map((key) => fetchChunk(m.tags[key]!, key)))
        missing.forEach((key, i) => got.set(key, all[i]!))
        return got
      } catch (e) {
        // `fetch` rejects with a TypeError when the scheme is not there (or not for this
        // origin): raw invoke from now on; anything else is an error of this read
        if (!(e instanceof TypeError) || options.transport === 'protocol') throw e
        transport = 'invoke'
      }
    }
    const payload = parse<Load>(await invoke<ArrayBuffer>(cmd('load'), pg === 'index' ? {} : { page: pg }))
    if (payload.version !== m.version || payload.locale !== m.locale) {
      throw new Stale(`i18n-next: the catalog moved from version ${m.version} to ${payload.version} during a load`)
    }
    for (const c of payload.chunks) {
      const key = `${c.locale}/${c.page}`
      const chunk: Chunk = c.linked ? { data: c.data, linked: c.linked } : { data: c.data }
      got.set(key, chunk)
      if (m.tags[key]) chunks.set(m.tags[key], chunk)
    }
    return got
  }

  /** Removes a page view and tells the instance (it re-renders on the root view). */
  function drop(loc: string, pg: string) {
    // an empty view through the public call notifies every adapter; the entry goes after it
    i18n.addRouteTranslations(loc, pg, {}, false)
    i18n.storage.translations.delete(`${loc}:${pg}`)
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
      if (old?.tags.length) drop(m.locale, pg)
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
      if (pg === 'index' || !v.tags.length || (loc === locale && (pg === page || pg === pending || pinned.has(pg)))) continue
      views.delete(id)
      drop(loc, pg)
      pages--
    }
    const live = new Set<string>()
    for (const v of views.values()) for (const t of v.tags) live.add(t)
    for (const t of chunks.keys()) if (!live.has(t)) chunks.delete(t)
  }

  function applyMeta(m: Meta) {
    current = m
    if (m.version > version) version = m.version
    const f = JSON.stringify([m.numberFormats, m.datetimeFormats, m.locales, m.timeZone])
    if (f === formats) return
    formats = f
    const fmt = i18n.formatter
    if (fmt) {
      fmt.setNumberFormats({ ...fmt.getNumberFormats(), ...m.numberFormats })
      fmt.setDateTimeFormats({ ...fmt.getDateTimeFormats(), ...m.datetimeFormats })
      // dates in the zone Rust shows them in; `null`: the system's, as the instance has it
      if (m.timeZone) (fmt as Zoned).timeZone = m.timeZone
    }
    const iso: Record<string, string> = {}
    for (const l of m.locales) if (l.iso) iso[l.code] = l.iso
    i18n.addLocaleIso?.(iso)
  }

  /** Reads the meta of the active locale and `pg`, loads what changed, then shows it. */
  async function show(pg: string, route: boolean, extra: { languages?: string[]; timeZone?: string } = {}, attempt = 0): Promise<void> {
    let m: Meta
    const ready = primed
    primed = undefined
    try {
      m = ready?.page === pg ? ready.meta : await meta(pg === 'index' ? extra : { ...extra, page: pg })
    } catch (e) {
      // a route the catalog has no page for: the root alone answers it
      if (kindOf(e) !== 'UnknownPage') throw e
      m = await meta(extra)
    }
    let got: Map<string, Chunk>
    try {
      got = await read(m, pg)
      // a visit run beside this one showed a newer catalog already
      if (m.version < version) throw new Stale(`i18n-next: version ${m.version} read after ${version}`)
    } catch (e) {
      // ponytail: 3 retries; a catalog that changes faster than a read is a bug elsewhere
      if (!(e instanceof Stale)) throw e
      if (attempt < 3) return show(pg, route, {}, attempt + 1)
      throw new Error(`${e.message} (the catalog kept changing: 3 new reads of meta)`)
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
      if (pending === pg) pending = undefined
      i18n.setRoute(pg)
    }
    evict()
  }

  /** The views of `pg` are checked at the current version: showing it needs no call (3.4). */
  const fresh = (pg: string) =>
    views.get(`${locale}:index`)?.version === version && (pg === 'index' || views.get(`${locale}:${pg}`)?.version === version)

  /** Shows a fresh page: the most recently used, the route of the instance. */
  function route(pg: string) {
    const id = `${locale}:${pg}`
    const v = views.get(id)
    if (v) {
      views.delete(id)
      views.set(id, v)
    }
    page = pg
    if (pending === pg) pending = undefined
    i18n.setRoute(pg)
    evict() // the page shown before may be over the limit now
  }

  /** `show`, without any call while the page's view is fresh. */
  async function visit(pg: string, showIt: boolean) {
    if (!fresh(pg)) return show(pg, showIt)
    if (showIt) route(pg)
    else {
      const id = `${locale}:${pg}`
      const v = views.get(id)
      if (v) {
        views.delete(id)
        views.set(id, v)
      }
    }
  }

  /** Visits the preloaded pages (and a `load`ed one) of the shown locale, all at once. */
  async function follow() {
    const pages = new Set(pinned)
    if (pending !== undefined) pages.add(pending)
    pages.delete(page)
    await Promise.all([...pages].map((p) => visit(p, false)))
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
  // the start locale from the WebView's languages, the zone of Rust dates from the
  // instance's zone, else the WebView's (DESIGN 5.4)
  const timeZone = (i18n.formatter as Zoned | undefined)?.timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone
  await enqueue(() => show(options.page ?? 'index', true, { languages: [...(languages ?? [])], ...(timeZone ? { timeZone } : {}) }))

  const unlisten = await listen<{ locale: string; version: number }>(EVENT_CHANGED, (e) => {
    const v = e.payload.version
    if (v <= version) return // late: a newer state is shown already
    enqueue(async () => {
      if (closed || v <= version) return
      await show(page, true)
      await follow()
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
    // nothing queued and the page loaded (by a guard's `load`): shown at once, in this tick
    setPage: (pg) => {
      if (busy > 0 || !fresh(pg)) return enqueue(() => visit(pg, true))
      route(pg)
      return Promise.resolve()
    },
    setLocale: (code) =>
      enqueue(async () => {
        await invoke<number>(cmd('set_locale'), { locale: code })
        await show(page, true)
        await follow()
      }),
    preload: (pg) =>
      enqueue(async () => {
        pinned.add(pg)
        await visit(pg, false)
      }),
    load: (pg) =>
      enqueue(async () => {
        pending = pg
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
