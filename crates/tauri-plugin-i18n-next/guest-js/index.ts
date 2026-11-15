/**
 * Frontend API of tauri-plugin-i18n-next.
 *
 * M0 skeleton: only `ping` exists, to prove the package, the IPC wiring and the tests.
 * The full API (`connect`, `setPage`, `setLocale`, `preload`) lands in M8.
 */
import { invoke } from '@tauri-apps/api/core'

/** Calls the plugin's `ping` command and resolves to its reply. Checks that the plugin is registered. */
export async function ping(): Promise<string> {
  return invoke<string>('plugin:i18n-next|ping')
}
