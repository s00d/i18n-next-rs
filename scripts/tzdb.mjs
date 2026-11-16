// tzdb generator (DESIGN 5.4): the bundled time-zone data of feature `tzdb` (always on Windows).
//
//   node scripts/tzdb.mjs            → crates/i18n-next/src/tz/tzdb.bin and names.rs (the zone
//                                      names Intl accepts, checked against Node)
//   node scripts/tzdb.mjs fixtures   → fixtures/tzif/* (hand-made TZif files and an Android
//                                      tzdata container for tests/tz_fixtures.rs)
//
// The IANA release VERSION is downloaded once into target/tzdb/ (needs network the first time,
// never into the repo), compiled with the system `zic` (`-b fat`), and every compiled zone is
// re-encoded compactly: transitions since 1970 only, the offset in effect at 1970, the POSIX
// footer, and the trailing transitions the footer reproduces dropped (what `-b slim` does; done
// here because older system zic, e.g. macOS's 2022g, cuts slim files too early for zones whose
// last change was a no-op, like America/Ojinaga or Asia/Gaza in 2026f). Abbreviations and pre-1970 history are
// dropped (the formatter needs neither). Links and identical zones share one blob. The first
// line of the output records the release, the tarball's sha256 and its URL. Layout: the doc of
// crates/i18n-next/src/tz/tzdb.rs. VERSION follows the jiff-tzdb release that tests/tz_tzdb.rs
// compares against; bump both together. Deterministic: a second run gives identical bytes.
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const VERSION = '2026f'
const URL_ = `https://data.iana.org/time-zones/releases/tzdata${VERSION}.tar.gz`
// The source files of a default `make` (backzone is not part of it).
const SOURCES = ['africa', 'antarctica', 'asia', 'australasia', 'europe', 'northamerica', 'southamerica', 'etcetera', 'backward', 'factory']

const root = fileURLToPath(new URL('..', import.meta.url))
const work = join(root, 'target', 'tzdb')

// ---- TZif (RFC 8536) → { times, types, initial, footer }, v2+ 64-bit data ----
function readTzif(buf) {
  if (buf.toString('latin1', 0, 4) !== 'TZif') throw new Error('not TZif')
  const counts = (at) => Array.from({ length: 6 }, (_, i) => buf.readUInt32BE(at + 20 + 4 * i))
  const blockLen = ([isut, isstd, leap, time, typ, chars], ts) => time * (ts + 1) + typ * 6 + chars + leap * (ts + 4) + isstd + isut
  if (buf[4] < 0x32) throw new Error('zic wrote a v1 file')
  const at2 = 44 + blockLen(counts(0), 4)
  const c = counts(at2)
  const [, , , timecnt, typecnt] = c
  let p = at2 + 44
  const times = []
  for (let i = 0; i < timecnt; i++) times.push(Number(buf.readBigInt64BE(p + 8 * i)))
  p += 8 * timecnt
  const idx = [...buf.subarray(p, p + timecnt)]
  p += timecnt
  const utoff = Array.from({ length: typecnt }, (_, i) => buf.readInt32BE(p + 6 * i))
  const end = at2 + 44 + blockLen(c, 8)
  const rest = buf.toString('latin1', end)
  const footer = rest.startsWith('\n') ? rest.slice(1, rest.indexOf('\n', 1)) : ''
  return { times, offsets: idx.map((i) => utoff[i]), initial: utoff[0], footer }
}

// ---- encoding ----
const uint = (out, v) => {
  if (!Number.isSafeInteger(v) || v < 0) throw new Error(`varint ${v}`)
  do {
    let b = v % 128
    v = Math.floor(v / 128)
    if (v > 0) b |= 0x80
    out.push(b)
  } while (v > 0)
}
const int = (out, v) => uint(out, v < 0 ? -2 * v - 1 : 2 * v)
const str = (out, s) => {
  const b = Buffer.from(s, 'utf8')
  uint(out, b.length)
  out.push(...b)
}

// ---- POSIX TZ footers: the same evaluation as crates/i18n-next/src/tz/posix.rs ----
const daysFromCivil = (y, m, d) => {
  y -= m <= 2 ? 1 : 0
  const era = Math.floor(y / 400)
  const yoe = y - era * 400
  const doy = Math.floor((153 * ((m + 9) % 12) + 2) / 5) + d - 1
  return era * 146097 + yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy - 719468
}
const yearOf = (t) => {
  const z = Math.floor(t / 86400) + 719468
  const era = Math.floor(z / 146097)
  const doe = z - era * 146097
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365)
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100))
  const mp = Math.floor((5 * doy + 2) / 153)
  return yoe + era * 400 + (mp >= 10 ? 1 : 0)
}
const leap = (y) => y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0)
const yearStart = (y) => daysFromCivil(y, 1, 1) * 86400
const hms = (s) => {
  const [h, m = 0, sec = 0] = s.replace(/^[+-]/, '').split(':').map(Number)
  return (s.startsWith('-') ? -1 : 1) * (h * 3600 + m * 60 + sec)
}
function parsePosix(s) {
  const m = /^(?:<[^>]+>|[A-Za-z]+)([+-]?[\d:]+)(?:(?:<[^>]+>|[A-Za-z]+)([+-]?[\d:]+)?,([^,]+),([^,]+))?$/.exec(s)
  if (!m) throw new Error(`footer ${s}`)
  const std = -hms(m[1])
  if (!m[3]) return { std }
  const dt = (x) => {
    const [day, time] = x.split('/')
    return { day, time: time === undefined ? 7200 : hms(time) }
  }
  return { std, dst: { off: m[2] ? -hms(m[2]) : std + 3600, start: dt(m[3]), end: dt(m[4]) } }
}
function dayOf(day, y) {
  const jan1 = daysFromCivil(y, 1, 1)
  if (day[0] === 'J') {
    const n = Number(day.slice(1))
    return jan1 + n - 1 + (leap(y) && n >= 60 ? 1 : 0)
  }
  if (day[0] === 'M') {
    const [mo, w, wd] = day.slice(1).split('.').map(Number)
    const first = daysFromCivil(y, mo, 1)
    const dim = [31, leap(y) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][mo - 1]
    let d = (((wd - ((first + 4) % 7 + 7) % 7) % 7) + 7) % 7 + 7 * (w - 1)
    while (d >= dim) d -= 7
    return first + d
  }
  const n = Number(day)
  return n < 365 + (leap(y) ? 1 : 0) ? jan1 + n : null
}
// [unix seconds, at the end of the year (jiff's 23:59:59.999999999)]
function toUtc({ day, time }, y, off) {
  const max = [yearStart(y + 1), true]
  const d = dayOf(day, y)
  if (d === null) return max
  const t = d * 86400 + time - off
  return t < yearStart(y) ? [yearStart(y), false] : t >= max[0] ? max : [t, false]
}
function posixOffset(p, t) {
  if (!p.dst) return p.std
  const y = yearOf(t)
  const s = toUtc(p.dst.start, y, p.std)
  let e = toUtc(p.dst.end, y, p.dst.off)
  if (s[0] === yearStart(y) && yearOf(e[0] - (e[1] ? 1 : 0) + p.std) !== y) e = [yearStart(y + 1), true]
  const inDst = s[0] <= e[0] ? s[0] <= t && t < e[0] : !(e[0] <= t && t < s[0])
  return inDst ? p.dst.off : p.std
}
// Does the footer give offset `o` on all of [a, b)? It changes only at its transitions
// (and the year bounds it clamps them to), so those points and `a` decide.
function footerHolds(p, a, b, o) {
  if (posixOffset(p, a) !== o) return false
  if (!p.dst) return true
  for (let y = yearOf(a) - 1; y <= yearOf(b) + 1; y++) {
    for (const t of [yearStart(y), toUtc(p.dst.start, y, p.std)[0], toUtc(p.dst.end, y, p.dst.off)[0]]) {
      if (t > a && t < b && posixOffset(p, t) !== o) return false
    }
  }
  return true
}
// How many transitions to keep. A reader applies the footer from the last kept transition
// on (already true of the last one), so the last kept moves back while the footer also
// reproduces the interval before it.
function slimEnd({ times, offsets, footer }) {
  if (!footer || times.length === 0) return times.length
  const p = parsePosix(footer)
  let j = times.length - 1
  while (j > 0 && footerHolds(p, times[j - 1], times[j], offsets[j - 1])) j--
  return j + 1
}

// A zone from 1970 on: the offset in effect at 0 (RFC semantics: the type of the last
// transition <= 0, else type 0), then the transitions >= 0.
function encodeZone(zone) {
  const { times, offsets, initial, footer } = zone
  const end = slimEnd(zone)
  let at0 = initial
  const kept = []
  times.forEach((t, i) => {
    if (t <= 0) at0 = offsets[i]
    if (t >= 0 && i < end) kept.push([t, offsets[i]])
  })
  // No transitions left: the reader applies the rule to every instant, so at0 must agree
  // with it (it does for every zone: zic ends a zone's transitions where the rule starts).
  const types = [...new Set(kept.map(([, o]) => o))]
  const out = []
  str(out, footer)
  int(out, at0)
  uint(out, types.length)
  for (const o of types) int(out, o)
  uint(out, kept.length)
  let prev = 0
  for (const [t, o] of kept) {
    uint(out, t - prev)
    uint(out, types.indexOf(o))
    prev = t
  }
  return Buffer.from(out)
}

function walk(dir, base = '') {
  const out = []
  for (const name of readdirSync(dir).sort()) {
    const full = join(dir, name)
    const rel = base ? `${base}/${name}` : name
    if (statSync(full).isDirectory()) out.push(...walk(full, rel))
    else out.push(rel)
  }
  return out
}

async function tzdb() {
  mkdirSync(work, { recursive: true })
  const tarball = join(work, `tzdata${VERSION}.tar.gz`)
  if (!existsSync(tarball)) {
    const res = await fetch(URL_)
    if (!res.ok) throw new Error(`${URL_}: HTTP ${res.status}`)
    writeFileSync(tarball, Buffer.from(await res.arrayBuffer()))
  }
  const sha = createHash('sha256').update(readFileSync(tarball)).digest('hex')
  const src = join(work, `tzdata${VERSION}`)
  const out = join(work, `zoneinfo${VERSION}`)
  for (const d of [src, out]) rmSync(d, { recursive: true, force: true })
  mkdirSync(src)
  execFileSync('tar', ['-xzf', tarball, '-C', src])
  const got = readFileSync(join(src, 'version'), 'utf8').trim()
  if (got !== VERSION) throw new Error(`tarball says ${got}, expected ${VERSION}`)
  execFileSync('zic', ['-b', 'fat', '-d', out, ...SOURCES.map((f) => join(src, f))])

  const blobs = []
  const blobIndex = new Map()
  const names = []
  for (const name of walk(out)) {
    const blob = encodeZone(readTzif(readFileSync(join(out, name))))
    const key = blob.toString('hex')
    if (!blobIndex.has(key)) {
      blobIndex.set(key, blobs.reduce((n, b) => n + b.length, 0))
      blobs.push(blob)
    }
    names.push([name, blobIndex.get(key)])
  }
  names.sort(([a], [b]) => Buffer.compare(Buffer.from(a), Buffer.from(b)))
  const body = []
  uint(body, names.length)
  for (const [name, off] of names) {
    str(body, name)
    uint(body, off)
  }
  const header = Buffer.from(`i18n-next tzdb ${VERSION} sha256:${sha} ${URL_}\n`)
  const file = Buffer.concat([header, Buffer.from(body), ...blobs])
  const dest = join(root, 'crates/i18n-next/src/tz/tzdb.bin')
  writeFileSync(dest, file)
  console.log(`${dest}: ${file.length} bytes, ${names.length} names, ${blobs.length} distinct zones (tzdata ${VERSION}, sha256 ${sha})`)
  writeNames(names.map(([n]) => n))
}

// ---- names.rs: every zone name `Intl` accepts, for one case-insensitive spelling (src/tz/mod.rs) ----
// The IANA zones and links of the release but `Factory` (Intl throws on it), plus the legacy
// aliases ICU keeps that IANA dropped, each with the IANA zone it reads. SystemV zones with DST
// (`SystemV/EST5EDT`, …) are left out: ICU gives them rules of its own no IANA zone has.
const ICU_ALIASES = [
  'ACT', 'AET', 'AGT', 'ART', 'AST', 'BET', 'BST', 'CAT', 'CNT', 'CST', 'CTT', 'EAT', 'ECT', 'IET', 'IST', 'JST',
  'MIT', 'NET', 'NST', 'PLT', 'PNT', 'PRT', 'PST', 'SST', 'VST', 'US/Pacific-New', 'Canada/East-Saskatchewan',
]
// ICU names these after themselves: the fixed IANA zone with the same offsets
const SYSTEMV = { AST4: 'Etc/GMT+4', CST6: 'Etc/GMT+6', EST5: 'Etc/GMT+5', HST10: 'Etc/GMT+10', MST7: 'Etc/GMT+7', PST8: 'Etc/GMT+8', YST9: 'Etc/GMT+9' }
const intlZone = (tz) => {
  try {
    return new Intl.DateTimeFormat('en-US', { timeZone: tz }).resolvedOptions().timeZone
  } catch {
    return null
  }
}
const intlOffset = (tz, t) => {
  const f = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' })
  const p = Object.fromEntries(f.formatToParts(t).map((x) => [x.type, x.value]))
  return Date.UTC(+p.year, p.month - 1, +p.day, +p.hour, +p.minute, +p.second) - Math.floor(t / 1000) * 1000
}
function writeNames(iana) {
  const set = new Set(iana)
  const rows = iana.filter((n) => n !== 'Factory').map((n) => [n, ''])
  const aliases = [...ICU_ALIASES.map((a) => [a, intlZone(a)]), ...Object.entries(SYSTEMV).map(([k, v]) => [`SystemV/${k}`, v])]
  for (const [alias, target] of aliases) {
    if (set.has(alias)) throw new Error(`${alias} is an IANA name now: drop it from ICU_ALIASES`)
    if (intlZone(alias) === null || !set.has(target)) throw new Error(`${alias}: Intl rejects it or ${target} is no IANA zone`)
    for (let y = 1970; y <= 2100; y++) {
      for (const m of [0, 6]) {
        const t = Date.UTC(y, m, 1)
        if (intlOffset(alias, t) !== intlOffset(target, t)) throw new Error(`${alias} differs from ${target} in ${y}`)
      }
    }
    rows.push([alias, target])
  }
  const lower = (n) => Buffer.from(n.toLowerCase())
  rows.sort(([a], [b]) => Buffer.compare(lower(a), lower(b)))
  const body = rows.map(([n, t]) => `    (${JSON.stringify(n)}, ${JSON.stringify(t)}),`).join('\n')
  const out = `// generated by scripts/tzdb.mjs from tzdata ${VERSION} and Node ${process.versions.node} (ICU ${process.versions.icu}) — do not edit
//! The zone names \`Intl\` accepts (src/tz/mod.rs \`canonical\`): the IANA zones and links of tzdata
//! ${VERSION} but \`Factory\`, and ICU's legacy aliases with the IANA zone they read (\`""\`: the name
//! itself). Sorted by ASCII lowercase for a case-insensitive binary search.

pub(crate) static NAMES: [(&str, &str); ${rows.length}] = [
${body}
];
`
  const dest = join(root, 'crates/i18n-next/src/tz/names.rs')
  writeFileSync(dest, out)
  console.log(`${dest}: ${rows.length} names (${aliases.length} ICU aliases)`)
}

// ---- fixtures: TZif files written by hand, so every version and corner is covered ----
// zone = { version: 0 | '2' | '3', types: [utoff...], abbrs: [name per type],
//          trans: [[time, typeIndex]...], footer }
function writeTzif({ version, types, abbrs, trans, footer = '' }) {
  const chars = Buffer.from(abbrs.map((a) => `${a}\0`).join(''), 'latin1')
  const abbrAt = abbrs.map((_, i) => abbrs.slice(0, i).reduce((n, a) => n + a.length + 1, 0))
  const block = (ts, list) => {
    const head = Buffer.alloc(44)
    head.write('TZif', 0, 'latin1')
    head[4] = version === 0 ? 0 : version.charCodeAt(0)
    ;[0, 0, 0, list.length, types.length, chars.length].forEach((n, i) => head.writeUInt32BE(n, 20 + 4 * i))
    const t = Buffer.alloc(list.length * ts)
    list.forEach(([time], i) => (ts === 8 ? t.writeBigInt64BE(BigInt(time), 8 * i) : t.writeInt32BE(time, 4 * i)))
    const idx = Buffer.from(list.map(([, ty]) => ty))
    const ty = Buffer.alloc(types.length * 6)
    types.forEach((o, i) => {
      ty.writeInt32BE(o, 6 * i)
      ty[6 * i + 4] = i > 0 && o > types[0] ? 1 : 0
      ty[6 * i + 5] = abbrAt[i]
    })
    return Buffer.concat([head, t, idx, ty, chars])
  }
  if (version === 0) return block(4, trans)
  // the v1 block of a "fat" file: the transitions that fit in 32 bits
  const fits = trans.filter(([t]) => t >= -(2 ** 31) && t < 2 ** 31)
  return Buffer.concat([block(4, fits), block(8, trans), Buffer.from(`\n${footer}\n`, 'latin1')])
}

// Android `ZoneInfoDB`: "tzdata" + version + NUL, offsets of index / data / zonetab, 52-byte
// entries (name padded to 40, start, length, raw offset), then the TZif bytes.
function writeAndroid(zones) {
  const head = Buffer.alloc(24)
  head.write('tzdata2026f\0', 0, 'latin1')
  const index = Buffer.alloc(52 * zones.length)
  let data = 0
  zones.forEach(([name, bytes], i) => {
    index.write(name, 52 * i, 'latin1')
    index.writeUInt32BE(data, 52 * i + 40)
    index.writeUInt32BE(bytes.length, 52 * i + 44)
    data += bytes.length
  })
  const dataAt = 24 + index.length
  head.writeUInt32BE(24, 12)
  head.writeUInt32BE(dataAt, 16)
  head.writeUInt32BE(dataAt + data, 20)
  return Buffer.concat([head, index, ...zones.map(([, b]) => b)])
}

function fixtures() {
  const dir = join(root, 'fixtures/tzif')
  mkdirSync(dir, { recursive: true })
  const zones = {
    // v1: 32-bit times, no footer: after the last transition its offset stays
    'v1.tzif': { version: 0, types: [3600, 7200], abbrs: ['CET', 'CEST'], trans: [[100000000, 1], [110000000, 0], [120000000, 1], [130000000, 0]] },
    // v2: times outside 32 bits (1883, 2065), LMT with seconds, a footer for after 2065 that
    // agrees with the last transition (jiff checks that)
    'v2.tzif': {
      version: '2',
      types: [-17762, -18000, -14400],
      abbrs: ['LMT', 'EST', 'EDT'],
      trans: [[-2717650800, 1], [-1633280400, 2], [-1615140000, 1], [3003721200, 2], [3024280800, 1]],
      footer: 'EST5EDT,M3.2.0,M11.1.0',
    },
    // v3: rule times outside 0..24 h (America/Nuuk since 2023)
    'v3.tzif': { version: '3', types: [-10800, -3600], abbrs: ['-03', '-01'], trans: [[1679792400, 1]], footer: '<-02>2<-01>,M3.5.0/-1,M10.5.0/0' },
    // no transitions: only the footer, with a quoted name and minutes
    'posix-only.tzif': { version: '2', types: [12600], abbrs: ['+0330'], trans: [], footer: '<+0330>-3:30' },
    // no transitions, empty footer: time type 0 always
    'empty.tzif': { version: '2', types: [-36000], abbrs: ['HST'], trans: [], footer: '' },
  }
  const bytes = {}
  for (const [file, z] of Object.entries(zones)) {
    bytes[file] = writeTzif(z)
    writeFileSync(join(dir, file), bytes[file])
  }
  writeFileSync(join(dir, 'tzdata'), writeAndroid([['Test/PosixOnly', bytes['posix-only.tzif']], ['Test/V2', bytes['v2.tzif']]]))
  console.log(`${dir}: ${Object.keys(zones).length + 1} files`)
}

if (process.argv[2] === 'fixtures') fixtures()
else await tzdb()
