import { defineConfig } from 'vitest/config'

// `--mode bench` (pnpm bench) runs the tinybench files instead of the tests.
export default defineConfig(({ mode }) => ({
  test: {
    environment: 'node',
    include: [mode === 'bench' ? 'tests/guest-js/**/*.bench.ts' : 'tests/guest-js/**/*.test.ts'],
    clearMocks: true,
    silent: mode === 'bench' ? false : undefined,
    // solid's client build (the `browser` condition) for the DOM tests: inlined, so vite
    // resolves it instead of node (which would pick the server build)
    server: { deps: { inline: [/solid-js/, /@i18n-micro\/solid/] } },
  },
  resolve: { conditions: ['browser'] },
}))
