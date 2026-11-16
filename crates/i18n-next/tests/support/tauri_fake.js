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
    // false: no `__TAURI_EVENT_PLUGIN_INTERNALS__` (an older Tauri): `unregisterCallback` alone
    this.eventInternals = true
    // a predicate on protocol paths: a matching fetch waits until `release()`
    this.hold = undefined
    this.held = []
    // 'Error' / 'DOMException': every protocol fetch rejects with it (not a missing scheme)
    this.reject = undefined
    // answered once by the next `meta` instead of the catalog's
    this.metaAnswer = undefined
    // the next `load`s answer a body one version older (a load that lost a race)
    this.staleLoads = 0
    // `changed` listeners by event: `{ id, handler }`, as the event plugin keeps them
    this.listeners = new Map()
    // `transformCallback` registry, as `__TAURI_INTERNALS__` keeps it
    this.callbacks = new Map()
    this.ids = 0
    this.eventIds = 0
  }

  install() {
    // a test that failed before its cleanup left its fake installed: take it down first
    globalThis.__i18nNextFake?.uninstall()
    globalThis.__i18nNextFake = this
    this.realFetch ??= globalThis.fetch
    globalThis.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => {
        const id = ++this.ids
        this.callbacks.set(id, cb)
        return id
      },
      unregisterCallback: (id) => this.callbacks.delete(id),
      convertFileSrc: (path, proto) => `${proto}://localhost/${encodeURIComponent(path)}`,
      invoke: async (cmd, args) => this.handle(cmd, args ?? {}),
    }
    // the event plugin's init script: drops the JS handler of a listener (tauri's `_unlisten`)
    if (this.eventInternals) {
      globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (event, eventId) => {
          const l = (this.listeners.get(event) ?? []).find((l) => l.id === eventId)
          if (l) this.callbacks.delete(l.handler)
        },
      }
    }
    globalThis.fetch = async (input) => {
      this.fetches.push(input)
      if (!this.protocol) throw new TypeError('Load failed')
      if (this.reject === 'Error') throw new Error('fetch broke')
      if (this.reject === 'DOMException') throw new DOMException('aborted', 'AbortError')
      if (this.status !== undefined) return new Response('{}', { status: this.status })
      const prefix = 'i18n-next://localhost/'
      if (!input.startsWith(prefix)) throw new TypeError(`unexpected URL ${input}`)
      const path = decodeURIComponent(input.slice(prefix.length))
      const body = this.source.chunk(path)
      let res
      if (body === undefined) {
        const key = path.slice(17, -5)
        res = new Response(JSON.stringify({ kind: 'Gone', tag: this.source.tag(key), version: this.version }), { status: 410 })
      } else {
        res = new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
      }
      if (this.hold?.(path)) return new Promise((resolve) => this.held.push(() => resolve(res)))
      return res
    }
  }

  uninstall() {
    if (globalThis.__i18nNextFake === this) delete globalThis.__i18nNextFake
    delete globalThis.__TAURI_INTERNALS__
    delete globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__
    if (this.realFetch) globalThis.fetch = this.realFetch
  }

  // answers the held fetches
  release() {
    this.hold = undefined
    const held = this.held
    this.held = []
    for (const r of held) r()
  }

  // listeners of `changed` the plugin holds
  listenerCount() {
    return (this.listeners.get('i18n-next://changed') ?? []).length
  }

  // the JS handler of the first `changed` listener, as an event already on its way holds it
  handlerOf() {
    const l = (this.listeners.get('i18n-next://changed') ?? [])[0]
    return l && this.callbacks.get(l.handler)
  }

  handle(cmd, a) {
    this.ipc.push(cmd)
    switch (cmd) {
      case 'plugin:event|listen': {
        const id = ++this.eventIds
        this.listeners.set(a.event, [...(this.listeners.get(a.event) ?? []), { id, handler: a.handler }])
        return id
      }
      case 'plugin:event|unlisten': {
        this.listeners.set(a.event, (this.listeners.get(a.event) ?? []).filter((l) => l.id !== a.eventId))
        return null
      }
      case 'plugin:i18n-next|meta': {
        this.lastMetaArgs = a
        if (this.metaAnswer !== undefined) {
          const answer = this.metaAnswer
          this.metaAnswer = undefined
          return answer
        }
        const m = this.source.meta(this.locale, a.page)
        if (!m) throw { kind: 'UnknownPage', message: `unknown page: ${a.page}` }
        const out = raw({ ...m, version: this.version })
        const after = this.afterMeta
        this.afterMeta = undefined
        after?.()
        return out
      }
      case 'plugin:i18n-next|load': {
        const lag = this.staleLoads > 0 ? 1 : 0
        if (lag) this.staleLoads--
        return raw({ ...this.source.load(this.locale, a.page), version: this.version - lag })
      }
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
    for (const l of this.listeners.get('i18n-next://changed') ?? []) {
      this.callbacks.get(l.handler)?.({ event: 'i18n-next://changed', id: l.id, payload: e })
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

// One turn of the event loop, no timer: a promise a MessageChannel message (a task) resolves.
// Chrome finishes a `Response` body read (`arrayBuffer`) in a task, never in microtasks
// (Node does it in a few microtasks), so the tests' `until` waits a task per round.
function tick() {
  return new Promise((resolve) => {
    const c = new MessageChannel()
    c.port1.onmessage = () => {
      c.port1.close()
      resolve()
    }
    c.port2.postMessage(null)
  })
}

return { Fake, contractSource, randomSource, rustSource, tick }
