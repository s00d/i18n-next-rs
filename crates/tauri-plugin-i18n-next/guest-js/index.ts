/**
 * Frontend API of tauri-plugin-i18n-next.
 *
 * M7: the `meta` command (what the front end needs to start and to build protocol URLs).
 * The full API (`connect`, `setPage`, `setLocale`, `preload`) lands in M8.
 */
import { invoke } from '@tauri-apps/api/core'

/** One locale of `meta().locales` (`Locale` of `@i18n-micro/types`). */
export interface Locale {
  code: string
  iso?: string
  dir?: string
  displayName?: string
}

/** What the `meta` command returns (DESIGN 6.1). */
export interface Meta {
  /** The catalog version: grows with every change (event `i18n-next://changed`). */
  version: number
  /** The locale the tags are for. */
  locale: string
  defaultLocale: string
  fallbackLocale: string | null
  locales: Locale[]
  /** The fallback chain of every locale, strongest first: premerge a view in this order. */
  chains: Record<string, string[]>
  numberFormats: Record<string, unknown>
  datetimeFormats: Record<string, unknown>
  /** The content tag of each chunk of the chain, by `"<locale>/<page>"`: `/{tag}/{locale}/{page}.json`. */
  tags: Record<string, string>
  /** The tag of the protocol bundle: `/{bundle}/bundle?locale=…&page=…`. */
  bundle: string
}

/**
 * Asks the plugin for the meta of `locale` (the active one by default) and `page`. The
 * command answers raw JSON bytes (no serde round trip): they are parsed here.
 */
export async function meta(options: { locale?: string; page?: string } = {}): Promise<Meta> {
  const raw = await invoke<ArrayBuffer>('plugin:i18n-next|meta', options)
  return JSON.parse(new TextDecoder().decode(raw)) as Meta
}
