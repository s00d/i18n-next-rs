// Writes the benchmark tables of docs/content/5.performance.md (DESIGN 7.7) between its
// `<!-- bench-table:<name> -->` … `<!-- /bench-table:<name> -->` markers; the text around them
// stays hand-written. Same inputs, same page: a second run changes nothing.
//
// Inputs (run them first, on a quiet machine):
// - criterion reports in target/criterion (CRITERION_HOME of .cargo/config.toml): the
//   criterion benches of i18n-next and the plugin, and `cargo bench -p compare` (ids
//   `versus/*`); the commands are in the page's Reproduce section. Start from an empty
//   target/criterion so renamed benches do not linger;
// - target/criterion/compare_memory.json, written by the `memory` bench of benches/compare;
// - the guest-js tinybench output: `pnpm -C crates/tauri-plugin-i18n-next bench` saved to
//   node_modules/.cache/logs/guest-js-bench.log (or the path given as the first argument).
//
// Usage: node scripts/bench-table.mjs [guest-js-bench.log]

import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const criterion = join(root, 'target/criterion')
const page = join(root, 'docs/content/5.performance.md')
const jsLog = process.argv[2] ?? join(root, 'node_modules/.cache/logs/guest-js-bench.log')

const ns = (v) =>
  v < 1e3 ? `${v.toPrecision(3)} ns` : v < 1e6 ? `${(v / 1e3).toPrecision(3)} µs` : `${(v / 1e6).toPrecision(3)} ms`
const bytes = (v) =>
  v < 2 ** 20 ? `${(v / 2 ** 10).toFixed(0)} KiB` : v < 2 ** 30 ? `${(v / 2 ** 20).toFixed(1)} MiB` : `${(v / 2 ** 30).toFixed(0)} GiB`
const sh = (cmd, args, cwd = root) => {
  try {
    return execFileSync(cmd, args, { cwd, encoding: 'utf8' }).trim()
  } catch {
    return 'unknown'
  }
}
const table = (head, rows) =>
  [`| ${head.join(' | ')} |`, `| ${head.map((_, i) => (i ? '---:' : '---')).join(' | ')} |`, ...rows.map((r) => `| ${r.join(' | ')} |`)].join('\n')

// criterion: id → median ns of the last run (`new/`)
const results = new Map()
let newest = 0
for (const dir of existsSync(criterion) ? readdirSync(criterion) : []) {
  const est = join(criterion, dir, 'new/estimates.json')
  if (!existsSync(est)) continue
  const { full_id: id } = JSON.parse(readFileSync(join(criterion, dir, 'new/benchmark.json'), 'utf8'))
  results.set(id, JSON.parse(readFileSync(est, 'utf8')).median.point_estimate)
  newest = Math.max(newest, statSync(est).mtimeMs)
}
if (!results.size) throw new Error(`no criterion results in ${criterion}: run the benches first`)

// the machine and toolchains of the run
const cpu = os.cpus()[0]?.model.trim() ?? 'unknown'
const osName = os.platform() === 'darwin' ? `macOS ${sh('sw_vers', ['-productVersion'])}` : `${os.type()} ${os.release()}`
const machine = [
  `Measured ${new Date(newest).toISOString().slice(0, 10)} on ${cpu} (${os.cpus().length} threads, ${bytes(os.totalmem())} RAM), ${osName} ${os.arch()};`,
  `${sh('rustc', ['-V'])}, Node ${process.version}. Medians of criterion (\`cargo bench\`, release with thin LTO) and`,
  'tinybench. Absolute numbers depend on the machine; the ratios within a table are what carries over.',
].join('\n')

// versus/<workload>/<lib>
const libs = [
  ['i18n_next', 'i18n-next'],
  ['rust_i18n', 'rust-i18n'],
  ['fluent', 'fluent-bundle'],
  ['icu4x', 'ICU4X'],
]
const workloads = [
  ['lookup_hit', 'lookup, hit'],
  ['lookup_fallback', 'lookup, `de` miss → `en`'],
  ['lookup_miss', 'lookup, miss everywhere'],
  ['interpolate_1', 'one `{name}` argument'],
  ['plural_message', 'plural message, `ru` 21'],
  ['plural_category', 'plural category alone, `ru` 21'],
  ['number_float', 'number `1234567.891`, `en`'],
  ['number_int', 'number `1234567`, `en`'],
  ['date_numeric', 'date `1/5/2024`, `en`'],
  ['date_long', 'date `January 5, 2024`, `en`'],
  ['cold_1k', 'build a catalog of 1k keys'],
  ['cold_10k', 'build a catalog of 10k keys'],
]
const versusRows = workloads
  .filter(([w]) => libs.some(([l]) => results.has(`versus/${w}/${l}`)))
  .map(([w, label]) => [label, ...libs.map(([l]) => (results.has(`versus/${w}/${l}`) ? ns(results.get(`versus/${w}/${l}`)) : '—'))])
const memoryFile = join(criterion, 'compare_memory.json')
if (existsSync(memoryFile)) {
  for (const [size, row] of Object.entries(JSON.parse(readFileSync(memoryFile, 'utf8')))) {
    versusRows.push([`heap held by ${size} keys`, ...libs.map(([l]) => (l in row ? bytes(row[l]) : '—'))])
  }
}
const versus = table(['', ...libs.map(([, name]) => name)], versusRows)

// the main crates, one table per first id segment
const groups = new Map()
for (const [id, v] of [...results].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
  if (id.startsWith('versus/')) continue
  const [group, ...rest] = id.split('/')
  if (!groups.has(group)) groups.set(group, [])
  groups.get(group).push([`\`${rest.join('/') || group}\``, ns(v)])
}
const crates = [...groups]
  .map(([group, rows]) => `#### \`${group}\`\n\n${table(['benchmark', 'median'], rows)}`)
  .join('\n\n')

// guest-js: the `console.table` of tinybench (Task name, Latency med (ns) = "p50 ± mad")
let guest = '_Not measured in this run._'
if (existsSync(jsLog)) {
  const rows = []
  let cols = null
  for (const line of readFileSync(jsLog, 'utf8').split('\n')) {
    if (!line.includes('│')) continue
    const cells = line.split('│').slice(1, -1).map((c) => c.trim().replace(/^'(.*)'$/, '$1'))
    if (cells.includes('Task name')) cols = { name: cells.indexOf('Task name'), med: cells.indexOf('Latency med (ns)') }
    else if (cols && cols.med >= 0) {
      const med = Number.parseFloat(cells[cols.med])
      if (Number.isFinite(med)) rows.push([`\`${cells[cols.name]}\``, ns(med)])
    }
  }
  if (rows.length) guest = table(['benchmark', 'median'], rows)
}

let text = readFileSync(page, 'utf8')
for (const [name, body] of [
  ['machine', machine],
  ['versus', versus],
  ['crates', crates],
  ['guest-js', guest],
]) {
  const re = new RegExp(`(<!-- bench-table:${name} -->)[\\s\\S]*?(<!-- /bench-table:${name} -->)`)
  if (!re.test(text)) throw new Error(`${page}: no <!-- bench-table:${name} --> … <!-- /bench-table:${name} --> markers`)
  text = text.replace(re, (_, open, close) => `${open}\n${body}\n${close}`)
}
writeFileSync(page, text)
console.log(`${page}: ${results.size} criterion results, ${groups.size} groups`)
