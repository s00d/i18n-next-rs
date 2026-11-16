import { I18nTypesPlugin } from '@i18n-micro/types-generator'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

// The Tauri playground front end (DESIGN 8a). `@i18n-micro/types-generator` reads the same
// catalog Rust embeds (examples/locales) and writes src/i18n-micro.d.ts, so `$t` / `t()` /
// `<i18n-t keypath>` autocomplete and check the keys (8b.6).
export default defineConfig({
  plugins: [vue(), I18nTypesPlugin({ srcDir: import.meta.dirname, translationDir: '../locales', outputFile: 'src/i18n-micro.d.ts' })],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true },
})
