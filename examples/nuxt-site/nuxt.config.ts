import { readFileSync } from 'node:fs'

// The same files as playground-rust, tauri-app and leptos-app (DESIGN 8a, 8b.19):
// examples/i18n.json for locales and fallback, examples/locales (root + pages/) for messages.
const config = JSON.parse(readFileSync(new URL('../i18n.json', import.meta.url), 'utf8'))
const pages = ['', '/settings', '/about']

export default defineNuxtConfig({
  compatibilityDate: '2026-11-01',
  modules: ['nuxt-i18n-micro'],
  i18n: {
    locales: config.locales,
    defaultLocale: config.defaultLocale,
    fallbackLocale: config.fallbackLocale,
    // relative to the project root: ../locales is examples/locales
    translationDir: '../locales',
    linkedMessages: true,
    strategy: 'prefix',
    // never write `{}` files for pages the catalog lacks (`index`, `ar` settings): the
    // shared folder is the Rust tests' fixture too
    disableWatcher: true,
  },
  nitro: {
    prerender: {
      routes: config.locales.flatMap((l: { code: string }) => pages.map((p) => `/${l.code}${p}`)),
    },
  },
})
