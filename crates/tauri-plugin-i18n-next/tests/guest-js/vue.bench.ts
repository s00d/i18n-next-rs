// @vitest-environment happy-dom
// Benches of `tauri-plugin-i18n-next-api/vue` (PLAN: a bench per public function): the start of
// `createTauriI18n` (one meta, the root, the instance), `useI18nLink` inside an app and the
// plugin's `dispose`. Against the fake plugin of api.bench.ts; `BENCH_TIME` shortens it.
import { Bench } from 'tinybench'
import { it } from 'vitest'
import { createApp } from 'vue'
import { createTauriI18n, useI18nLink } from '../../guest-js/vue.js'
import { FakePlugin, benchSource } from './fake.js'

const time = Number((globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.BENCH_TIME ?? 300)

it('public API of guest-js/vue', async () => {
  const fake = new FakePlugin(benchSource(1_000, 'namespaced', 2), 'en')
  fake.install()
  const quiet = { document: null, reportMissing: false } as const
  const bench = new Bench({ time })
  bench.add('api/createTauriI18n/start_1k', async () => (await createTauriI18n(quiet)).dispose())
  const plugin = await createTauriI18n(quiet)
  const app = createApp({ render: () => null })
  app.use(plugin)
  bench.add('api/useI18nLink/inject', () => void app.runWithContext(useI18nLink))
  // a hook per iteration would cost more than the call: dispose is measured with its start
  bench.add('i18n_plugin/dispose/after_start_1k', async () => (await createTauriI18n(quiet)).dispose())
  await bench.run()
  console.table(bench.table())
  plugin.dispose()
  fake.uninstall()
}, 600_000)
