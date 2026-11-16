<script setup lang="ts">
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useI18n } from '@i18n-micro/vue'
import { EVENT_MISSING } from 'tauri-plugin-i18n-next-api'
import { useI18nLink } from 'tauri-plugin-i18n-next-api/vue'
import { computed, onUnmounted, ref } from 'vue'
import { useRoute } from 'vue-router'

// The inspector (DESIGN 8a, 8b.7): what is loaded, at which version and over which channel,
// the keys anyone missed (dev builds report them: event `i18n-next://missing`), and a
// measure of the transport for the active root chunk.
const { instance, locale } = useI18n()
const link = useI18nLink()
const missing = ref<Array<{ locale: string; page: string; key: string; count: number }>>([])
const stop = listen<(typeof missing.value)[number]>(EVENT_MISSING, (e) => missing.value.push(e.payload))
onUnmounted(() => void stop.then((f) => f()))

// the link is plain JS: re-read it on every change of the instance (views, locale, route)
const changes = ref(0)
const unsubscribe = instance.subscribeToChanges(() => changes.value++)
onUnmounted(unsubscribe)
const route = useRoute()
const state = computed(() => {
  void [changes.value, locale.value, route.fullPath]
  return {
    version: link.version,
    locale: link.locale,
    page: link.page,
    fallback: link.fallback,
    chunks: [...instance.storage.translations.keys()].sort(),
  }
})
const timing = ref('')
async function measure() {
  const t0 = performance.now()
  const raw = await invoke<ArrayBuffer>('plugin:i18n-next|meta', {})
  const t1 = performance.now()
  const meta = JSON.parse(new TextDecoder().decode(raw))
  const key = `${meta.locale}/index`
  const res = await fetch(convertFileSrc(`${meta.tags[key]}/${key}.json`, 'i18n-next'))
  const bytes = await res.arrayBuffer()
  const t2 = performance.now()
  JSON.parse(new TextDecoder().decode(bytes))
  const t3 = performance.now()
  const ms = (a: number, b: number) => (b - a).toFixed(2)
  timing.value = `meta IPC ${ms(t0, t1)} ms · ${key} ${(bytes.byteLength / 1024).toFixed(1)} KB: fetch ${ms(t1, t2)} ms, JSON.parse ${ms(t2, t3)} ms`
}

// Transport bench (DESIGN 6.1): synthetic `en` pages of 1k / 10k / 100k keys (Rust command
// `bench_page`) read through the plugin's own paths, median of RUNS after a warm-up. The
// result is a Markdown table for the performance page, one per platform (WebView2,
// WebKitGTK, Android WebView, WKWebView).
const RUNS = 15
const SIZES = [1_000, 10_000, 100_000]
const benchTable = ref('')
const benchRunning = ref(false)
async function median(f: () => Promise<unknown>): Promise<number> {
  await f()
  const times: number[] = []
  for (let i = 0; i < RUNS; i++) {
    const t0 = performance.now()
    await f()
    times.push(performance.now() - t0)
  }
  times.sort((a, b) => a - b)
  return times[RUNS >> 1]
}
const copy = () => navigator.clipboard.writeText(benchTable.value)
async function bench() {
  benchRunning.value = true
  benchTable.value = 'running…'
  try {
    const rows: Array<[string, (page: string, keys: number, url: string, text: string) => Promise<unknown>]> = [
      ['invoke raw (`ipc::Response`) → `TextDecoder` → `JSON.parse`', async (page) =>
        JSON.parse(new TextDecoder().decode(await invoke<ArrayBuffer>('plugin:i18n-next|load_page', { page })))],
      ['invoke raw, transfer only', (page) => invoke<ArrayBuffer>('plugin:i18n-next|load_page', { page })],
      ['invoke with `serde_json::Value`', (_, keys) => invoke('bench_value', { keys })],
      ['**custom protocol: `fetch().json()`**', (_p, _k, url) => fetch(url, { cache: 'no-store' }).then((r) => r.json())],
      ['custom protocol, transfer only (`arrayBuffer`)', (_p, _k, url) => fetch(url, { cache: 'no-store' }).then((r) => r.arrayBuffer())],
      ['string already in JS: `JSON.parse` (JSON in the bundle)', async (_p, _k, _u, text) => JSON.parse(text)],
    ]
    const columns: string[] = []
    const cells: string[][] = rows.map(() => [])
    for (const keys of SIZES) {
      const page = await invoke<string>('bench_page', { keys })
      const meta = JSON.parse(new TextDecoder().decode(await invoke<ArrayBuffer>('plugin:i18n-next|meta', { locale: 'en', page })))
      const url = convertFileSrc(`${meta.tags[`en/${page}`]}/en/${page}.json`, 'i18n-next')
      const body = await (await fetch(url, { cache: 'no-store' })).arrayBuffer()
      const size = body.byteLength >= 1 << 20 ? `${(body.byteLength / (1 << 20)).toFixed(1)} MB` : `${Math.round(body.byteLength / 1024)} KB`
      columns.push(`${size} (${keys / 1000}k keys)`)
      const text = new TextDecoder().decode(body)
      for (const [i, [, run]] of rows.entries()) cells[i].push((await median(() => run(page, keys, url, text))).toFixed(2))
    }
    benchTable.value = [
      `<!-- ${navigator.userAgent}; median of ${RUNS}, ms -->`,
      `| Method | ${columns.join(' | ')} |`,
      `| --- |${columns.map(() => ' --- |').join('')}`,
      ...rows.map(([name], i) => `| ${name} | ${cells[i].join(' | ')} |`),
    ].join('\n')
  } catch (e) {
    benchTable.value = `bench failed: ${e}`
  } finally {
    benchRunning.value = false
  }
}
</script>

<template>
  <details open>
    <summary>Inspector</summary>
    <p>
      v{{ state.version }} · {{ state.locale }} / {{ state.page }} ·
      {{ state.fallback ? 'raw invoke (no protocol)' : 'i18n-next:// protocol' }}
    </p>
    <p>Loaded views: {{ state.chunks.join(', ') }}</p>
    <p><button @click="measure">Measure transport</button> {{ timing }}</p>
    <p>
      <button :disabled="benchRunning" @click="bench">Transport bench (1k / 10k / 100k keys)</button>
      <button v-if="benchTable && !benchRunning" @click="copy">Copy</button>
    </p>
    <pre v-if="benchTable">{{ benchTable }}</pre>
    <p>Missing keys ({{ missing.length }}):</p>
    <pre>{{ missing.map((m) => `${m.locale}/${m.page}: ${m.key} ×${m.count}`).join('\n') || '—' }}</pre>
  </details>
</template>
