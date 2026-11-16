// Smoke check of the built package: every entry point loads as CJS and ESM and exports the same API,
// and the emitted declarations match the snapshot in tests/__golden__/dts (UPDATE_GOLDEN=1 rewrites it).
import { createRequire } from 'node:module'
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'

const require = createRequire(import.meta.url)
// an entry and the chunks it imports, transitively
function bytes(file, seen = new Set()) {
  if (seen.has(file)) return 0
  seen.add(file)
  const url = new URL(`../dist-js/${file}`, import.meta.url)
  const deps = [...readFileSync(url, 'utf8').matchAll(/from\s*["']\.\/([^"']+\.js)["']/g)].map((m) => m[1])
  return statSync(url).size + deps.reduce((n, d) => n + bytes(d, seen), 0)
}
const names = (m) => Object.keys(m).filter((k) => k !== 'default').sort()
const expected = { index: ['connect', 'meta'], vue: ['connect', 'createTauriI18n', 'meta', 'useI18nLink'] }

for (const [entry, fns] of Object.entries(expected)) {
  const cjs = require(`../dist-js/${entry}.cjs`)
  const esm = await import(`../dist-js/${entry}.js`)
  const [a, b] = [names(cjs), names(esm)]
  if (a.join() !== b.join()) throw new Error(`${entry}: CJS/ESM exports differ:\n${a}\n${b}`)
  for (const fn of fns) {
    if (typeof esm[fn] !== 'function') throw new Error(`${entry}: missing export: ${fn}`)
  }
  console.log(`dist ok: ${entry} (${a.length} exports, ${(bytes(`${entry}.js`) / 1024).toFixed(1)} kB ESM with its chunks, unminified)`)
}

const dist = new URL('../dist-js/', import.meta.url)
const golden = new URL('../tests/__golden__/dts/', import.meta.url)
const dts = readdirSync(dist).filter((n) => n.endsWith('.d.ts')).sort()
const read = (dir, n) => {
  try {
    return readFileSync(new URL(n, dir), 'utf8')
  } catch {
    return undefined
  }
}
if (process.env.UPDATE_GOLDEN) {
  mkdirSync(golden, { recursive: true })
  for (const n of dts) writeFileSync(new URL(n, golden), read(dist, n))
}
const want = readdirSync(golden).filter((n) => n.endsWith('.d.ts')).sort()
const differ = [...new Set([...dts, ...want])].filter((n) => read(dist, n) !== read(golden, n))
if (differ.length) throw new Error(`.d.ts differ from tests/__golden__/dts (UPDATE_GOLDEN=1 rewrites it): ${differ}`)
console.log(`d.ts snapshot ok: ${dts.join(', ')}`)
