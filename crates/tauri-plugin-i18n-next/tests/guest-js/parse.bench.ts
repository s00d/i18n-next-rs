// What guest-js pays per chunk besides the transport (DESIGN 6.1: 70–85% of a load is
// `JSON.parse`): parsing a chunk of 1k / 10k keys, `Response.json()` (what `fetch().json()`
// does), the raw-invoke path (`TextDecoder` + `JSON.parse`), and the premerge of a
// `de-at → de → en` view and a page view on it. `pnpm bench` prints the table (tinybench,
// medians); `BENCH_TIME` (ms per task) shortens it for the CI smoke.
import { Bench } from 'tinybench'
import { it } from 'vitest'
import { pageView, premerge } from '../../guest-js/merge.js'

const time = Number((globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.BENCH_TIME ?? 300)

function chunk(keys: number, tag: string): string {
  const data: Record<string, Record<string, string>> = {}
  for (let i = 0; i < keys; i++) (data[`group${i % 50}`] ??= {})[`key${i}`] = `${tag} text number ${i} with {param}`
  return JSON.stringify({ data })
}

it('JSON.parse and premerge per chunk', async () => {
  const bench = new Bench({ time })
  for (const keys of [1_000, 10_000]) {
    const text = chunk(keys, 'en')
    const bytes = new TextEncoder().encode(text)
    const layers = [JSON.parse(text), JSON.parse(chunk(keys / 2, 'de')), JSON.parse(chunk(keys / 10, 'at'))]
    const size = `${keys / 1000}k_keys_${(bytes.length / 1024).toFixed(0)}kb`
    const chain = [...layers.map((data) => ({ data, page: false })), { data: { title: 'Page', group0: { key0: 'page' } }, page: true }]
    bench
      .add(`chunk/json_parse/${size}`, () => void JSON.parse(text))
      // the raw-invoke path: TextDecoder + JSON.parse
      .add(`chunk/text_decoder_parse/${size}`, () => void JSON.parse(new TextDecoder().decode(bytes)))
      // what the protocol fetch does: Response.json()
      .add(`chunk/response_json/${size}`, async () => void (await new Response(bytes).json()))
      .add(`merge/premerge/de_at_de_en_${size}`, () => void premerge(layers))
      .add(`merge/pageView/de_at_de_en_${size}`, () => void pageView(chain))
  }
  await bench.run()
  console.table(bench.table())
}, 120_000)
