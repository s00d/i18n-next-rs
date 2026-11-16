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
    <p>Missing keys ({{ missing.length }}):</p>
    <pre>{{ missing.map((m) => `${m.locale}/${m.page}: ${m.key} ×${m.count}`).join('\n') || '—' }}</pre>
  </details>
</template>
