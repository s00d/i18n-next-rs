// What guest-js pays per chunk besides the transport (DESIGN 6.1: 70–85% of a load is
// `JSON.parse`): parsing a chunk of 1k / 10k keys, `Response.json()` (what `fetch().json()`
// does), the raw-invoke path (`TextDecoder` + `JSON.parse`), and the premerge of a
// `de-at → de → en` view. `pnpm bench` prints the table (tinybench, medians).
import { Bench } from 'tinybench'
import { it } from 'vitest'
import { premerge } from '../../guest-js/merge.js'

function chunk(keys: number, tag: string): string {
  const data: Record<string, Record<string, string>> = {}
  for (let i = 0; i < keys; i++) (data[`group${i % 50}`] ??= {})[`key${i}`] = `${tag} text number ${i} with {param}`
  return JSON.stringify({ data })
}

it('JSON.parse and premerge per chunk', async () => {
  const bench = new Bench({ time: 300 })
  for (const keys of [1_000, 10_000]) {
    const text = chunk(keys, 'en')
    const bytes = new TextEncoder().encode(text)
    const layers = [JSON.parse(text), JSON.parse(chunk(keys / 2, 'de')), JSON.parse(chunk(keys / 10, 'at'))]
    const size = `${keys} keys, ${(bytes.length / 1024).toFixed(0)} KB`
    bench
      .add(`JSON.parse (${size})`, () => void JSON.parse(text))
      .add(`TextDecoder + JSON.parse, raw invoke (${size})`, () => void JSON.parse(new TextDecoder().decode(bytes)))
      .add(`Response.json(), protocol fetch (${size})`, async () => void (await new Response(bytes).json()))
      .add(`premerge de-at → de → en (${size})`, () => void premerge(layers))
  }
  await bench.run()
  console.table(bench.table())
}, 120_000)
