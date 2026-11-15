// Smoke check of the built package: every entry point loads as CJS and ESM and exports the same API.
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const names = (m) => Object.keys(m).filter((k) => k !== 'default').sort()
const expected = { index: ['ping'], vue: ['createTauriI18n', 'ping'] }

for (const [entry, fns] of Object.entries(expected)) {
  const cjs = require(`../dist-js/${entry}.cjs`)
  const esm = await import(`../dist-js/${entry}.js`)
  const [a, b] = [names(cjs), names(esm)]
  if (a.join() !== b.join()) throw new Error(`${entry}: CJS/ESM exports differ:\n${a}\n${b}`)
  for (const fn of fns) {
    if (typeof esm[fn] !== 'function') throw new Error(`${entry}: missing export: ${fn}`)
  }
  console.log(`dist ok: ${entry} (${a.length} exports)`)
}
