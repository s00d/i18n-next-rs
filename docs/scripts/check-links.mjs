// Checks every internal link of the generated site (docs/.output/public after `pnpm generate`):
// each href / src that stays on the site must name a file there, and each #anchor an id in that page.
// External links (http:, mailto:, …) are not fetched.
//
//   node docs/scripts/check-links.mjs [dir]     # exits 1 and lists the broken links
//
// The base URL is NUXT_APP_BASE_URL, else `/i18n-next-rs/` (what `nuxt generate` uses, see nuxt.config.ts).
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const dir = process.argv[2] ?? fileURLToPath(new URL('../.output/public/', import.meta.url))
const base = process.env.NUXT_APP_BASE_URL || '/i18n-next-rs/'
if (!existsSync(dir)) {
  console.error(`check-links: ${dir} does not exist; run \`pnpm -C docs generate\` first`)
  process.exit(1)
}

const pages = []
;(function walk(d) {
  for (const name of readdirSync(d)) {
    const p = join(d, name)
    if (statSync(p).isDirectory()) walk(p)
    else if (name.endsWith('.html')) pages.push(p)
  }
})(dir)

const html = new Map(pages.map((p) => [p, readFileSync(p, 'utf8')]))
const idCache = new Map()
const ids = (file) => {
  if (!idCache.has(file)) {
    const text = html.get(file) ?? readFileSync(file, 'utf8')
    idCache.set(file, new Set([...text.matchAll(/\sid="([^"]+)"/g)].map((m) => decode(m[1]))))
  }
  return idCache.get(file)
}
const decode = (s) => {
  const t = s.replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>')
  try {
    return decodeURIComponent(t)
  } catch {
    return t
  }
}

// `/i18n-next-rs/guides/pages` -> `<dir>/guides/pages.html` (autoSubfolderIndex: false), `/i18n-next-rs/` -> index.html.
function target(path) {
  const rel = path.slice(base.length).replace(/\/$/, '')
  const candidates = rel === '' ? ['index.html'] : [rel, `${rel}.html`, `${rel}/index.html`]
  return candidates.map((c) => join(dir, c)).find((f) => existsSync(f) && statSync(f).isFile())
}

const broken = []
let checked = 0
for (const [page, text] of html) {
  // The 404 and SPA fallback pages are rendered without content.
  if (/(^|\/)(404|200)\.html$/.test(page)) continue
  const from = relative(dir, page)
  for (const m of text.matchAll(/\s(?:href|src)="([^"]*)"/g)) {
    const raw = decode(m[1])
    if (!raw || /^[a-z][a-z0-9+.-]*:/i.test(raw) || raw.startsWith('//')) continue
    checked++
    const [pathPart, hash] = raw.split('#')
    const path = pathPart.split('?')[0]
    let file = page
    if (path) {
      if (!path.startsWith(base)) {
        broken.push(`${from}: ${raw} (outside the base URL ${base})`)
        continue
      }
      file = target(path)
      if (!file) {
        broken.push(`${from}: ${raw} (no such page or file)`)
        continue
      }
    }
    if (hash && file.endsWith('.html') && !ids(file).has(hash)) broken.push(`${from}: ${raw} (no id "${hash}")`)
  }
}

if (broken.length) {
  console.error(`check-links: ${broken.length} broken link(s) of ${checked} in ${html.size} pages`)
  for (const b of broken) console.error(`  ${b}`)
  process.exit(1)
}
console.log(`check-links: ${checked} internal links in ${html.size} pages, none broken`)
