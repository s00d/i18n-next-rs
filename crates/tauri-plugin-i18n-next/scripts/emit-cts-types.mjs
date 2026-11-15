// dist-js/*.d.ts -> *.d.cts for the `require` condition. A plain copy is wrong once an entry imports another:
// `from './index.js'` inside a .d.cts would resolve to the ESM types, so relative `.js` specifiers become `.cjs`.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'

const dir = new URL('../dist-js/', import.meta.url)
const relative = /(\bfrom\s+|\bimport\s*\(\s*)(['"])(\.{1,2}\/[^'"]+?)\2/g
for (const name of readdirSync(dir).filter((n) => n.endsWith('.d.ts'))) {
  const cts = readFileSync(new URL(name, dir), 'utf8').replace(relative, (_, lead, q, spec) => {
    if (!spec.endsWith('.js')) throw new Error(`${name}: relative import without .js: ${spec} (write './x.js' in guest-js)`)
    return `${lead}${q}${spec.slice(0, -3)}.cjs${q}`
  })
  writeFileSync(new URL(name.replace(/\.d\.ts$/, '.d.cts'), dir), cts)
}
