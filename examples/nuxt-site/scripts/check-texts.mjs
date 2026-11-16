// The generated site shows what the Rust tour does (DESIGN 8b.19): every `data-k` text of
// `.output/public/<locale>/<page>/index.html` equals `playground-rust --texts` for that
// locale, page and key, every key of the tour is on the page, and every `data-k` of the page
// is a key of the tour.
// Usage: node scripts/check-texts.mjs [texts.json]   (without a file it runs cargo)
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

const root = new URL('../../../', import.meta.url)
const want = JSON.parse(
  process.argv[2]
    ? readFileSync(process.argv[2], 'utf8')
    : execFileSync('cargo', ['run', '-q', '--locked', '-p', 'playground-rust', '--', '--texts'], {
        cwd: root,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'inherit'],
      }),
)

const entities = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' }
const decode = (s) =>
  s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e) =>
    e[0] !== '#' ? (entities[e] ?? m) : String.fromCodePoint(e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : Number(e.slice(1))),
  )

let problems = 0
let compared = 0
for (const [locale, pages] of Object.entries(want)) {
  for (const [page, texts] of Object.entries(pages)) {
    const file = new URL(`.output/public/${locale}/${page === 'index' ? '' : `${page}/`}index.html`, new URL('../', import.meta.url))
    const html = readFileSync(file, 'utf8')
    const shown = new Map([...html.matchAll(/data-k="([^"]+)"[^>]*>([^<]*)</g)].map((m) => [decode(m[1]), decode(m[2])]))
    for (const [key, text] of Object.entries(texts)) {
      compared++
      if (!shown.has(key)) {
        problems++
        console.error(`${locale}/${page}: ${key} is not on the page`)
      } else if (shown.get(key) !== text) {
        problems++
        console.error(`${locale}/${page}: ${key}: site ${JSON.stringify(shown.get(key))}, Rust ${JSON.stringify(text)}`)
      }
    }
    // every `data-k` attribute, also on an element whose text the match above cannot read
    for (const key of new Set([...html.matchAll(/data-k="([^"]+)"/g)].map((m) => decode(m[1])))) {
      if (!Object.hasOwn(texts, key)) {
        problems++
        console.error(`${locale}/${page}: data-k ${key} is on the page but not in --texts`)
      }
    }
  }
}
if (problems > 0 || compared === 0) {
  console.error(`check-texts: ${problems} problem(s) in ${compared} text(s)`)
  process.exit(1)
}
console.log(`check-texts: ${compared} texts of the Nuxt site equal the Rust tour's`)
