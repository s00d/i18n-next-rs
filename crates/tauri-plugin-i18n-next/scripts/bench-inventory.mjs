// Bench inventory of guest-js (DESIGN 7.7, PLAN: a bench per public function), the JS twin of
// crates/i18n-next/tests/support/inventory.rs. The public API comes from the `.d.ts` snapshot of
// the entries (tests/__golden__/dts/{index,vue}.d.ts: `pnpm build` fails when it lags behind
// guest-js): every exported function, and every method of what those functions return
// (`connect` → `Link`, `createTauriI18n` → `TauriI18nPlugin`).
//
// Bench names in tests/guest-js/*.bench.ts are `group/function/case` (`bench.add('…')`, a
// template literal counts by its text before the first `${`):
// - a function `name` is covered by `<group>/name/<case>` with any group that is not an owner;
// - a method `Owner.name` only by `<owner>/name/<case>`, the owner in lower snake case with a
//   leading `Tauri` dropped (`Link` → `link`, `TauriI18nPlugin` → `i18n_plugin`).
// Comments are stripped first, so a commented-out bench does not count. Any other name shape
// fails. Exits 1 with the list of what no bench names.
import { readFileSync, readdirSync } from 'node:fs'

const dir = (p) => new URL(`../${p}`, import.meta.url)
const dts = ['index.d.ts', 'vue.d.ts'].map((n) => readFileSync(dir(`tests/__golden__/dts/${n}`), 'utf8')).join('\n')

/** `src` without `//` and `/* *\/` comments; string and template literals kept as they are. */
function strip(src) {
  let out = ''
  for (let i = 0; i < src.length; i++) {
    const c = src[i]
    if (c === '/' && src[i + 1] === '/') {
      while (i < src.length && src[i] !== '\n') i++
      out += '\n'
    } else if (c === '/' && src[i + 1] === '*') {
      i = src.indexOf('*/', i + 2)
      if (i < 0) break
      i++
    } else if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < src.length && src[j] !== c) j += src[j] === '\\' ? 2 : 1
      out += src.slice(i, j + 1)
      i = j
    } else out += c
  }
  return out
}

const owner = (type) =>
  type
    .replace(/^Tauri/, '')
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .toLowerCase()

/** The members with a call signature of `interface Name {…}` or `type Name = … & {…}`. */
function methods(src, name) {
  const at = src.search(new RegExp(`export (interface ${name}\\b|type ${name}\\b)`))
  if (at < 0) return []
  const open = src.indexOf('{', at)
  let depth = 0
  let end = open
  for (; end < src.length; end++) {
    if (src[end] === '{') depth++
    else if (src[end] === '}' && --depth === 0) break
  }
  const body = src.slice(open + 1, end)
  const out = []
  let d = 0
  for (const line of body.split('\n')) {
    const m = d === 0 && /^\s*(\w+)\??\(/.exec(line)
    if (m) out.push(m[1])
    for (const ch of line) d += ch === '{' ? 1 : ch === '}' ? -1 : 0
  }
  return out
}

const api = new Set()
const owners = new Set()
const src = strip(dts)
for (const m of src.matchAll(/export declare function (\w+)\([^]*?\):\s*(?:Promise<)?(\w+)/g)) {
  api.add(m[1])
  for (const meth of methods(src, m[2])) {
    api.add(`${m[2]}.${meth}`)
    owners.add(owner(m[2]))
  }
}

const covered = new Set()
const bad = []
const benches = readdirSync(dir('tests/guest-js')).filter((n) => n.endsWith('.bench.ts'))
for (const file of benches) {
  const text = strip(readFileSync(dir(`tests/guest-js/${file}`), 'utf8'))
  for (const m of text.matchAll(/\.add\(\s*(['"`])((?:(?!\1)[^$\\]|\\.)*)(\$\{)?/g)) {
    const parts = m[2].split('/')
    // a case may be all `${…}`
    if (parts.length !== 3 || parts.some((p, i) => i < 2 && !/^\w+$/.test(p)) || (parts[2] === '' && !m[3])) {
      bad.push(`${file}: ${m[2]}`)
      continue
    }
    const [group, fn] = parts
    const method = [...api].find((a) => a.includes('.') && owner(a.split('.')[0]) === group && a.split('.')[1] === fn)
    if (method) covered.add(method)
    else if (!owners.has(group) && api.has(fn)) covered.add(fn)
  }
}

const missing = [...api].filter((a) => !covered.has(a)).sort()
if (bad.length) console.error(`bench names not group/function/case:\n  ${bad.join('\n  ')}`)
if (missing.length) console.error(`public API without a bench:\n  ${missing.join('\n  ')}`)
if (bad.length || missing.length) process.exit(1)
console.log(`bench inventory ok: ${api.size} public functions and methods, ${benches.length} bench files`)
