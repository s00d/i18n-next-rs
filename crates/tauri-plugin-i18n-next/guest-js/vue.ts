/**
 * Vue integration (`tauri-plugin-i18n-next-api/vue`) on top of the optional peer `@i18n-micro/vue`.
 *
 * M0 skeleton: the entry point exists so the package layout is final. `createTauriI18n` lands in M8.
 * The core API is re-exported, so a Vue app needs one import path. Relative imports keep the `.js`
 * extension: the emitted `.d.ts` must resolve under node16 ESM (the `.d.cts` copy rewrites it to `.cjs`).
 */
export { meta } from './index.js'
export type { Locale, Meta } from './index.js'

/** Reserved for M8. Always rejects for now. */
export async function createTauriI18n(): Promise<never> {
  throw new Error('tauri-plugin-i18n-next-api/vue: createTauriI18n lands in M8')
}
