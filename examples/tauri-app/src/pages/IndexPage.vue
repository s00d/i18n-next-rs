<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '@i18n-micro/vue'
import { ref, watch } from 'vue'

const { locale } = useI18n()
const count = ref(1)
const greeting = ref('')
const start = Date.now() - 3 * 60_000
// a Rust command that returns a `Text`: translated in the active locale on the Rust side
watch(locale, async () => (greeting.value = await invoke<string>('greeting', { name: 'Tauri' })), { immediate: true })
</script>

<template>
  <h1>{{ greeting }}</h1>
  <p>{{ $t('shout') }}</p>
  <p>
    <button @click="count = Math.max(0, count - 1)">−</button>
    <i18n-t keypath="demo.items" :plural="count" tag="span" />
    <button @click="count++">+</button>
  </p>
  <p>{{ $tc('apples', count) }}</p>
  <p><i18n-t keypath="demo.price" :number="1234.5" :params="{ price: $tn(1234.5, 'currency') }" /></p>
  <p><i18n-t keypath="demo.today" :date="new Date()" /></p>
  <p><i18n-t keypath="demo.updated" :relative-date="start" /></p>
  <p>
    <i18n-t keypath="demo.terms">
      <template #link><a href="#">{{ $t('demo.terms_link') }}</a></template>
    </i18n-t>
  </p>
</template>
