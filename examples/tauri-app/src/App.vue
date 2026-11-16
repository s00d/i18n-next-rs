<script setup lang="ts">
import { useI18n } from '@i18n-micro/vue'
import { useI18nLink } from 'tauri-plugin-i18n-next-api/vue'
import Inspector from './Inspector.vue'

const { getLocales, locale } = useI18n()
const link = useI18nLink()
// a switch reaches Rust, the native menu and tray, and the other window (`changed`)
const switchTo = (code: string) => link.setLocale(code)
</script>

<template>
  <header>
    <nav>
      <RouterLink to="/">{{ $t('nav.home') }}</RouterLink>
      <RouterLink to="/settings" @mouseenter="link.preload('settings')">{{ $t('nav.settings') }}</RouterLink>
      <RouterLink to="/about">{{ $t('nav.about') }}</RouterLink>
    </nav>
    <select :value="locale" @change="switchTo(($event.target as HTMLSelectElement).value)">
      <option v-for="l in getLocales()" :key="l.code" :value="l.code">{{ l.displayName ?? l.code }}</option>
    </select>
  </header>
  <main>
    <RouterView />
  </main>
  <footer>{{ $t('footer') }}</footer>
  <Inspector />
</template>

<style>
body { font: 15px/1.5 system-ui, sans-serif; margin: 2rem; }
header { display: flex; gap: 1rem; align-items: center; justify-content: space-between; }
nav { display: flex; gap: 1rem; }
nav a.router-link-exact-active { font-weight: 600; }
pre { background: #8881; padding: .75rem; overflow: auto; max-height: 30vh; }
</style>
