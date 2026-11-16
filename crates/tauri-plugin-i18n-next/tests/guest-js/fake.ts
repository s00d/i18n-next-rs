// A fake plugin for guest-js tests: the commands (`meta`, `load`, `set_locale`,
// `report_missing`) through `mockIPC`, the `i18n-next://` protocol through a stubbed `fetch`
// (URLs from `convertFileSrc`, decoded as the Rust handler decodes them), and `changed`
// events the test delivers when and in what order it likes. No timers anywhere.
import { emit } from '@tauri-apps/api/event'
import { clearMocks, mockConvertFileSrc, mockIPC } from '@tauri-apps/api/mocks'
import type { Meta } from '../../guest-js/index.js'

export type Tree = Record<string, unknown>

// mockIPC patches window.__TAURI_INTERNALS__; the node environment has no window
;(globalThis as { window?: unknown }).window ??= globalThis

/** What the fake serves: per state, the meta of a `(locale, page)` and chunk bodies by URL path. */
export interface Source {
  /** `meta` of the active locale (`undefined`: unknown page). */
  meta(locale: string, page: string | undefined): Meta | undefined
  /** Protocol body of `tag/locale/page.json`; `undefined` when the tag is not current. */
  chunk(path: string): unknown
  /** The current tag of `locale/page` (the body of a 410). */
  tag(key: string): string
  /** `load` payload of the active locale. */
  load(locale: string, page: string | undefined): unknown
  locales(): string[]
}

const raw = (v: unknown) => new TextEncoder().encode(JSON.stringify(v)).buffer

export class FakePlugin {
  version = 0
  locale: string
  /** `false`: every protocol fetch fails as in a WebView without the scheme. */
  protocol = true
  /** Events emitted by Rust and not yet delivered to the window. */
  pending: Array<{ locale: string; version: number }> = []
  ipc: string[] = []
  /** The arguments of each call, in order. */
  args: Array<Record<string, unknown>> = []
  fetches: string[] = []
  missing: Array<{ locale: string; page: string; key: string }> = []
  /** `false`: `report_missing` is not registered (a release build). */
  dev = true
  /** An HTTP status every protocol read answers instead of the chunk (a broken handler). */
  status: number | undefined
  /** Runs once after the next `meta` answer is built (to change the catalog under a read). */
  afterMeta: (() => void) | undefined

  constructor(public source: Source, locale: string) {
    this.locale = locale
  }

  install() {
    mockConvertFileSrc('macos')
    mockIPC(
      (cmd, args) => {
        const a = (args ?? {}) as Record<string, unknown>
        this.ipc.push(cmd)
        this.args.push(a)
        switch (cmd) {
          case 'plugin:i18n-next|meta': {
            const m = this.source.meta(this.locale, a.page as string | undefined)
            if (!m) throw { kind: 'UnknownPage', message: `unknown page: ${a.page}` }
            const out = raw({ ...m, version: this.version })
            const after = this.afterMeta
            this.afterMeta = undefined
            after?.()
            return out
          }
          case 'plugin:i18n-next|load':
            return raw({ ...(this.source.load(this.locale, a.page as string | undefined) as Tree), version: this.version })
          case 'plugin:i18n-next|set_locale':
            if (!this.source.locales().includes(a.locale as string)) throw { kind: 'UnknownLocale', message: `unknown locale: ${a.locale}` }
            this.switch(a.locale as string)
            return this.version
          case 'plugin:i18n-next|report_missing':
            if (!this.dev) throw new Error('command report_missing not found')
            this.missing.push(...(a.keys as typeof this.missing))
            return null
          default:
            throw new Error(`unexpected command ${cmd}`)
        }
      },
      { shouldMockEvents: true },
    )
    globalThis.fetch = (async (input: string) => {
      this.fetches.push(input)
      if (!this.protocol) throw new TypeError('Load failed')
      if (this.status !== undefined) return new Response('{}', { status: this.status })
      const prefix = 'i18n-next://localhost/'
      if (!input.startsWith(prefix)) throw new TypeError(`unexpected URL ${input}`)
      const path = decodeURIComponent(input.slice(prefix.length))
      const body = this.source.chunk(path)
      if (body === undefined) {
        const key = path.slice(17, -5)
        return new Response(JSON.stringify({ kind: 'Gone', tag: this.source.tag(key), version: this.version }), { status: 410 })
      }
      return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
    }) as typeof fetch
  }

  uninstall() {
    clearMocks()
  }

  /** A switch by anyone (this window, another one, the tray): a new version and its event. */
  switch(locale: string) {
    this.locale = locale
    this.bump()
  }

  /** Any change: a new version and its event, delivered later. */
  bump() {
    this.version++
    this.pending.push({ locale: this.locale, version: this.version })
  }

  /** Delivers the pending event at `index` (late ones included). */
  async deliver(index: number) {
    const [e] = this.pending.splice(index, 1)
    if (e) await emit('i18n-next://changed', e)
  }
}

/** FNV-1a 64 of a string, as 16 hex digits: a content tag. */
export function tagOf(s: string): string {
  let h = 0xcbf29ce484222325n
  for (let i = 0; i < s.length; i++) {
    h ^= BigInt(s.charCodeAt(i))
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn
  }
  return h.toString(16).padStart(16, '0')
}

/** locale → page → nested tree (`index`: the root). */
export type Catalog = Record<string, Record<string, Tree>>

/**
 * A plugin over a mutable catalog, as the Rust side serves it: `meta` tags the chunks of the
 * chain that hold data (content tags), `load` sends them, a stale tag answers 410.
 */
export function catalogSource(
  cat: Catalog,
  chains: Record<string, string[]>,
  pages: string[],
  extra: Partial<Pick<Meta, 'locales' | 'numberFormats' | 'datetimeFormats' | 'timeZone'>> = {},
): Source {
  const body = (l: string, p: string) => JSON.stringify({ data: cat[l]?.[p] ?? {} })
  const has = (l: string, p: string) => Object.keys(cat[l]?.[p] ?? {}).length > 0
  const keys = (l: string, page: string | undefined) =>
    chains[l]!.flatMap((c) => ['index', ...(page && page !== 'index' ? [page] : [])].filter((p) => has(c, p)).map((p) => `${c}/${p}`))
  const split = (k: string) => k.split('/') as [string, string]
  return {
    meta(l, page) {
      if (page && page !== 'index' && !pages.includes(page)) return undefined
      const tags = Object.fromEntries(keys(l, page).map((k) => [k, tagOf(body(...split(k)))]))
      const locales = Object.keys(chains).map((code) => ({ code, ...(code === 'ar' ? { dir: 'rtl' as const } : {}) }))
      return { version: 0, locale: l, defaultLocale: 'en', fallbackLocale: 'en', locales, chains, numberFormats: {}, datetimeFormats: {}, timeZone: null, tags, bundle: '0'.repeat(16), ...extra }
    },
    chunk(path) {
      const [tag, l, file] = path.split('/') as [string, string, string]
      const p = file.slice(0, -5)
      return tagOf(body(l, p)) === tag ? JSON.parse(body(l, p)) : undefined
    },
    tag: (key) => tagOf(body(...split(key))),
    load: (l, page) => ({
      version: 0,
      locale: l,
      chunks: keys(l, page).map((k) => {
        const [c, p] = split(k)
        return { locale: c, page: p, data: cat[c]![p] }
      }),
    }),
    locales: () => Object.keys(chains),
  }
}
