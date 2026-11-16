// A fake Tauri plugin for the `tauri_guest` wasm tests (tests/tauri_guest.rs), the twin of
// guest-js's tests/guest-js/fake.ts: `window.__TAURI_INTERNALS__` (invoke, convertFileSrc,
// transformCallback and the event plugin), the `i18n-next://` protocol through a stubbed
// `fetch`, and `changed` events the test delivers when and in what order it likes. No
// timers anywhere. Evaluated as a function body: it returns its classes and sources.
const raw = (v) => new TextEncoder().encode(JSON.stringify(v)).buffer

class Fake {
  constructor(source, locale) {
    this.source = source
    this.locale = locale
    this.version = 0
    // false: every protocol fetch fails as in a WebView without the scheme
    this.protocol = true
    // events emitted by Rust and not yet delivered
    this.pending = []
    this.ipc = []
    this.fetches = []
    this.missing = []
    // false: `report_missing` is not registered (a release build)
    this.dev = true
    // an HTTP status every protocol read answers instead of the chunk
    this.status = undefined
    // runs once after the next `meta` answer is built (to change the catalog under a read)
    this.afterMeta = undefined
    this.listeners = new Map()
    this.callbacks = new Map()
    this.ids = 0
  }

  install() {
    this.realFetch ??= globalThis.fetch
    globalThis.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => {
        const id = ++this.ids
        this.callbacks.set(id, cb)
        return id
      },
      convertFileSrc: (path, proto) => `${proto}://localhost/${encodeURIComponent(path)}`,
      invoke: async (cmd, args) => this.handle(cmd, args ?? {}),
    }
    globalThis.fetch = async (input) => {
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
    }
  }

  uninstall() {
    delete globalThis.__TAURI_INTERNALS__
    if (this.realFetch) globalThis.fetch = this.realFetch
  }

  handle(cmd, a) {
    this.ipc.push(cmd)
    switch (cmd) {
      case 'plugin:event|listen': {
        this.listeners.set(a.event, [...(this.listeners.get(a.event) ?? []), a.handler])
        return a.handler
      }
      case 'plugin:event|unlisten': {
        this.listeners.set(a.event, (this.listeners.get(a.event) ?? []).filter((h) => h !== a.eventId))
        return null
      }
      case 'plugin:i18n-next|meta': {
        this.lastMetaArgs = a
        const m = this.source.meta(this.locale, a.page)
        if (!m) throw { kind: 'UnknownPage', message: `unknown page: ${a.page}` }
        const out = raw({ ...m, version: this.version })
        const after = this.afterMeta
        this.afterMeta = undefined
        after?.()
        return out
      }
      case 'plugin:i18n-next|load':
        return raw({ ...this.source.load(this.locale, a.page), version: this.version })
      case 'plugin:i18n-next|set_locale':
        if (!this.source.locales().includes(a.locale)) throw { kind: 'UnknownLocale', message: `unknown locale: ${a.locale}` }
        this.switch(a.locale)
        return this.version
      case 'plugin:i18n-next|report_missing':
        if (!this.dev) throw new Error('command report_missing not found')
        this.missing.push(...a.keys)
        return null
      default:
        throw new Error(`unexpected command ${cmd}`)
    }
  }

  // a switch by anyone (this window, another one, the tray): a new version and its event
  switch(locale) {
    this.locale = locale
    this.bump()
  }

  // any change: a new version and its event, delivered later
  bump() {
    this.version++
    this.pending.push({ locale: this.locale, version: this.version })
  }

  // delivers the pending event at `index` (late ones included) to every listener
  deliver(index) {
    const [e] = this.pending.splice(index, 1)
    if (!e) return
    for (const h of this.listeners.get('i18n-next://changed') ?? []) {
      this.callbacks.get(h)?.({ event: 'i18n-next://changed', id: h, payload: e })
    }
  }

  count(suffix) {
    return this.ipc.filter((c) => c.endsWith(suffix)).length
  }
}

// The plugin as tests/guest_contract.rs recorded it (crates/tauri-plugin-i18n-next).
function contractSource(fx) {
  const locales = [...new Set(Object.keys(fx.metas).map((id) => id.split('|')[0]))]
  return {
    meta: (l, p) => fx.metas[`${l}|${p ?? 'index'}`],
    chunk: (path) => fx.chunks[path],
    tag: () => '0000000000000000',
    load: (l, p) => fx.loads[`${l}|${p ?? 'index'}`],
    locales: () => locales,
  }
}

// One catalog of tests/guest_random.rs: the tags of a view are its chain's chunks (root, page).
function randomSource(base, c) {
  const keys = (l, p = 'index') =>
    base.chains[l].flatMap((loc) => (p === 'index' ? ['index'] : ['index', p]).map((pg) => `${loc}/${pg}`)).filter((k) => c.chunks[k])
  const known = (p = 'index') => p === 'index' || Object.keys(c.chunks).some((k) => k.endsWith(`/${p}`))
  const body = ({ data, linked }) => (linked ? { data, linked } : { data })
  return {
    meta: (l, p) =>
      known(p) ? { ...base, version: 0, locale: l, tags: Object.fromEntries(keys(l, p).map((k) => [k, c.chunks[k].tag])), bundle: '0'.repeat(16) } : undefined,
    chunk: (path) => {
      const key = path.slice(17, -5)
      return c.chunks[key]?.tag === path.slice(0, 16) ? body(c.chunks[key]) : undefined
    },
    tag: (key) => c.chunks[key].tag,
    load: (l, p) => ({
      version: 0,
      locale: l,
      chunks: keys(l, p).map((k) => {
        const [locale, page] = k.split('/')
        return { locale, page, ...body(c.chunks[k]) }
      }),
    }),
    locales: () => Object.keys(base.chains),
  }
}

// A source over Rust closures (a live `I18n` encoded by `payload`): JSON text in, objects out.
function rustSource(f) {
  const parse = (t) => (t === undefined || t === null ? undefined : JSON.parse(t))
  return {
    meta: (l, p) => parse(f.meta(l, p ?? null)),
    chunk: (path) => parse(f.chunk(path)),
    tag: (key) => f.tag(key),
    load: (l, p) => parse(f.load(l, p ?? null)),
    locales: () => JSON.parse(f.locales()),
  }
}

return { Fake, contractSource, randomSource, rustSource }
