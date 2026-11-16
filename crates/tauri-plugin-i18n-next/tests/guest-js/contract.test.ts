// The contract with the Rust side (DESIGN 6.2): `tests/guest_contract.rs` records what the
// plugin serves for a catalog with a regional chain (de-at → de → en), pages, linked
// messages, `null`s, own dotted keys and leaf/object conflicts, and what Rust `t` answers in
// every view. Here `connect` replays it into a real `@i18n-micro/runtime` instance, over the
// protocol and over the raw-invoke fallback, and every key must read what Rust reads.
import { createI18n } from '@i18n-micro/runtime'
import { afterEach, describe, expect, it } from 'vitest'
import { connect, type Meta } from '../../guest-js/index.js'
import { FakePlugin, type Source } from './fake.js'
import fixture from './fixtures/contract.json'

interface Fixture {
  keys: string[]
  metas: Record<string, Meta>
  loads: Record<string, unknown>
  chunks: Record<string, unknown>
  answers: Record<string, Record<string, string | null>>
  /** An instant, and Rust `td` / `td_as(…, "stamp")` of it per locale, in the catalog's zone. */
  at: number
  dates: Record<string, [string, string]>
}

const fx = fixture as unknown as Fixture
const locales = [...new Set(Object.keys(fx.metas).map((id) => id.split('|')[0]!))]
const pages = [...new Set(Object.keys(fx.metas).map((id) => id.split('|')[1]!))]

const source: Source = {
  meta: (l, p) => fx.metas[`${l}|${p ?? 'index'}`],
  chunk: (path) => fx.chunks[path],
  tag: () => '0000000000000000',
  load: (l, p) => fx.loads[`${l}|${p ?? 'index'}`],
  locales: () => locales,
}

/** What the instance reads for `key`; a subtree (`t` of a node) is no text, as in Rust. */
function read(i18n: ReturnType<typeof createI18n>, key: string): string | null {
  const v = i18n.t(key, undefined, '\u0000missing')
  if (v === '\u0000missing' || (typeof v === 'object' && v !== null)) return null
  return String(v)
}

let fake: FakePlugin
afterEach(() => fake.uninstall())

describe('guest-js reads what Rust t reads, over a real plugin payload', () => {
  for (const transport of ['protocol', 'invoke'] as const) {
    it(`every view of every locale (${transport})`, async () => {
      fake = new FakePlugin(source, 'en')
      fake.protocol = transport === 'protocol'
      fake.install()
      const i18n = createI18n({ locale: 'en', missingWarn: false })
      const link = await connect(i18n, { document: null, reportMissing: false })
      expect(link.fallback).toBe(transport === 'invoke')
      for (const l of locales) {
        await link.setLocale(l)
        for (const p of pages) {
          await link.setPage(p)
          const got = Object.fromEntries(fx.keys.map((k) => [k, read(i18n, k)]))
          expect({ view: `${l}|${p}`, ...got }).toEqual({ view: `${l}|${p}`, ...fx.answers[`${l}|${p}`] })
        }
        // dates in the zone meta sends (Asia/Tokyo), not the host's
        expect([l, i18n.td(fx.at), i18n.td(fx.at, 'stamp')]).toEqual([l, ...fx.dates[l]!])
      }
      link.close()
    })
  }
})
