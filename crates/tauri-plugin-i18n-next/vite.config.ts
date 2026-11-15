import { defineConfig } from 'vite'

// Library build of guest-js: ESM + CJS for `.` and `./vue`; @tauri-apps/api and @i18n-micro/* stay external.
// Type declarations come from `tsc -p tsconfig.build.json`.
export default defineConfig({
  build: {
    outDir: 'dist-js',
    emptyOutDir: true,
    minify: false,
    sourcemap: false,
    target: 'es2021',
    lib: {
      entry: { index: 'guest-js/index.ts', vue: 'guest-js/vue.ts' },
      formats: ['es', 'cjs'],
      fileName: (format, name) => `${name}.${format === 'es' ? 'js' : 'cjs'}`
    },
    rolldownOptions: {
      external: [/^@tauri-apps\/api/, /^@i18n-micro\//]
    }
  }
})
