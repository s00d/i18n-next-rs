// CLDR generator (DESIGN 5.2). M2: cardinal plural rules.
//
// Reads cldr-core `supplemental/plurals.json` and writes `crates/i18n-next/src/plural/rules.rs`:
// one Rust function per distinct rule set, the locales Intl.PluralRules supports with the rule
// each uses, the language aliases and region aliases that change which rule a tag gets. Node's
// Intl (ICU 78.3, CLDR 48.0, the same CLDR as the data) decides only what the CLDR files cannot
// say: which locales `supportedLocalesOf` knows and where an alias leads. It also writes the
// oracle of `tests/plural_props.rs`, `fixtures/compat/__golden__/plural-categories.md`.
//
// Run: pnpm -C scripts cldr. Deterministic: a second run gives byte-identical files.
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const here = (p) => fileURLToPath(new URL(p, import.meta.url))
const CLDR_VERSION = JSON.parse(readFileSync(here('./node_modules/cldr-core/package.json'), 'utf8')).version
const NODE_CLDR = `${process.versions.cldr.split('.')[0]}.${process.versions.cldr.split('.')[1]}`
if (!CLDR_VERSION.startsWith(`${NODE_CLDR}.`)) throw new Error(`cldr-core ${CLDR_VERSION} but Node has CLDR ${process.versions.cldr}`)

const plurals = JSON.parse(readFileSync(here('./node_modules/cldr-core/supplemental/plurals.json'), 'utf8')).supplemental['plurals-type-cardinal']
const aliases = JSON.parse(readFileSync(here('./node_modules/cldr-core/supplemental/aliases.json'), 'utf8')).supplemental.metadata.alias

const ORDER = ['zero', 'one', 'two', 'few', 'many', 'other']
const CAT = { zero: 'Zero', one: 'One', two: 'Two', few: 'Few', many: 'Many', other: 'Other' }
const LETTER = { zero: 'z', one: 'o', two: 't', few: 'f', many: 'm', other: 'x' }
const INT_OPERANDS = new Set(['i', 'v', 'w', 'f', 't', 'e', 'c'])
const utf8 = (a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b))

// ---- rule syntax (UTS 35 plural rules) → Rust ----
// condition = and ('or' and)*; and = relation ('and' relation)*; relation = expr ('=' | '!=') list;
// expr = operand ('%' value)?; list = (value | value '..' value) (',' ...)*
function parseCondition(text) {
  const toks = text.trim().split(/\s+/).filter(Boolean)
  if (toks.length === 0) return null
  let p = 0
  const next = () => toks[p++]
  const ors = []
  let ands = []
  while (p < toks.length) {
    const operand = next()
    if (!/^[nivwftec]$/.test(operand)) throw new Error(`operand ${operand} in ${text}`)
    let mod = null
    let op = next()
    if (op === '%') {
      mod = Number(next())
      op = next()
    }
    if (op !== '=' && op !== '!=') throw new Error(`relation ${op} in ${text}`)
    const items = next().split(',').map((r) => {
      const [a, b] = r.split('..').map(Number)
      return b === undefined ? [a, a] : [a, b]
    })
    ands.push({ operand, mod, negate: op === '!=', items })
    const join = toks[p]
    if (join === 'and') p++
    else {
      ors.push(ands)
      ands = []
      if (join === 'or') p++
      else if (join !== undefined) throw new Error(`join ${join} in ${text}`)
    }
  }
  return ors
}

function rustRelation({ operand, mod, negate, items }) {
  let body
  let atomic = true
  if (INT_OPERANDS.has(operand)) {
    const x = mod === null ? `o.${operand}` : `o.${operand} % ${mod}`
    body = `matches!(${x}, ${items.map(([a, b]) => (a === b ? `${a}` : `${a}..=${b}`)).join(' | ')})`
  } else {
    // n is the absolute value as f64: `n = 2..4` holds for whole values only (2.5 is in no range)
    const x = mod === null ? 'o.n' : `o.n % ${mod}.0`
    body = items.map(([a, b]) => (a === b ? `${x} == ${a}.0` : `in_range(${x}, ${a}.0, ${b}.0)`)).join(' || ')
    atomic = items.length === 1
  }
  // a negation is always parenthesised: an unbracketed `!a || b` negated only `a` (spike S8)
  if (negate) return { body: `!(${body})`, atomic: true }
  return { body, atomic }
}
function rustCondition(ors) {
  const many = ors.length > 1 || ors[0].length > 1
  return ors.map((ands) => ands.map(rustRelation).map((r) => (r.atomic || !many ? r.body : `(${r.body})`)).join(' && ')).join(' || ')
}

// ---- rule sets, deduplicated ----
const ruleSets = new Map() // signature → { locales, conditions: [[category, text]] }
for (const locale of Object.keys(plurals).sort(utf8)) {
  const conditions = []
  for (const category of ORDER) {
    const raw = plurals[locale][`pluralRule-count-${category}`]
    if (raw === undefined) continue
    conditions.push([category, raw.split('@')[0].trim()])
  }
  const signature = JSON.stringify(conditions)
  if (!ruleSets.has(signature)) ruleSets.set(signature, { locales: [], conditions })
  ruleSets.get(signature).locales.push(locale)
}
const rules = [...ruleSets.values()]
const ruleOf = new Map()
rules.forEach((r, i) => r.locales.forEach((l) => ruleOf.set(l, i)))

// ---- what Intl supports, and where it resolves a tag ----
function intl(tag) {
  try {
    if (Intl.PluralRules.supportedLocalesOf(tag).length === 0) return { kind: 'none' }
    return { kind: 'ok', locale: new Intl.PluralRules(tag).resolvedOptions().locale }
  } catch {
    return { kind: 'invalid' }
  }
}
// The locales Intl knows by their own name; the CLDR names it aliases (`mo` → `ro`) and `und` drop out.
const available = Object.keys(plurals).filter((l) => intl(l).locale === l).sort(utf8)
// What Rust does without aliases: strip region / script until a known locale is left.
function naive(tag) {
  const parts = tag.split('-')
  for (let n = Math.min(parts.length, 3); n > 0; n--) {
    const candidate = parts.slice(0, n).join('-')
    if (available.includes(candidate)) return ruleOf.get(candidate)
  }
  return null
}
const ruleIndex = (r) => (r.kind === 'ok' ? ruleOf.get(r.locale.split('-u-')[0]) ?? ruleOf.get(r.locale.split('-')[0]) : null)
// Language aliases (simple `iw`, or `lang-variant` like `art-lojban`) where the alias changes the rule.
const languageAliases = []
for (const [key, { _replacement }] of Object.entries(aliases.languageAlias)) {
  const tag = key.replace(/_/g, '-')
  const r = intl(tag)
  if (r.kind === 'invalid') continue
  // V8 takes a bare lowercase two-letter tag as it is, unless it is one of its legacy codes
  // (`iw`, `in`, `mo`…): `bh` stays unsupported, `bh-IN` and `BH` become `bho`.
  const alone = ruleIndex(r)
  const longer = /^[a-z]{2,3}$/.test(tag) ? ruleIndex(intl(`${tag}-US`)) : alone
  if (alone === naive(tag) && longer === naive(`${tag}-US`)) continue
  languageAliases.push([tag.toLowerCase(), _replacement.replace(/_/g, '-'), alone === longer])
}
languageAliases.sort((a, b) => utf8(a[0], b[0]))
// Region aliases onto a region some known locale has (`620` → `PT`, for `pt-PT`).
const knownRegions = new Set(available.flatMap((l) => l.split('-').slice(1).filter((s) => /^([A-Z]{2}|\d{3})$/.test(s))))
const regionAliases = Object.entries(aliases.territoryAlias)
  .filter(([key, { _replacement }]) => /^([A-Z]{2}|\d{3})$/.test(key) && knownRegions.has(_replacement.split(' ')[0]))
  .map(([key, { _replacement }]) => [key, _replacement.split(' ')[0]])
  .sort((a, b) => utf8(a[0], b[0]))

// ---- rules.rs ----
const fnName = (i) => `r_${rules[i].locales[0].toLowerCase().replace(/-/g, '_')}`
const out = []
out.push(`// generated by scripts/cldr.mjs from cldr-core ${CLDR_VERSION} — do not edit`)
out.push('//! Cardinal plural rules of CLDR, one function per distinct rule set, and the tables that')
out.push('//! resolve a BCP 47 tag the way `Intl.PluralRules.supportedLocalesOf` does (see `super`).')
out.push('')
out.push('use super::{Category, Operands};')
out.push('')
out.push('/// One rule set: its function and the categories it has (bit `1 << Category as u8`).')
out.push('pub(crate) struct Rule {')
out.push('    pub(crate) select: fn(&Operands) -> Category,')
out.push('    pub(crate) categories: u8,')
out.push('}')
out.push('')
out.push('/// `x = a..b` of the rule syntax: a whole value in the range.')
out.push('fn in_range(x: f64, a: f64, b: f64) -> bool {')
out.push('    x.fract() == 0.0 && x >= a && x <= b')
out.push('}')
rules.forEach((r, i) => {
  out.push('')
  out.push(`/// ${r.locales.join(', ')}`)
  for (const [category, text] of r.conditions) if (text) out.push(`/// - ${category}: \`${text}\``)
  const usesO = r.conditions.some(([, text]) => text !== '')
  out.push(`fn ${fnName(i)}(${usesO ? 'o' : '_'}: &Operands) -> Category {`)
  for (const [category, text] of r.conditions) {
    const ors = parseCondition(text)
    if (ors === null) continue
    out.push(`    if ${rustCondition(ors)} {`)
    out.push(`        return Category::${CAT[category]};`)
    out.push('    }')
  }
  if (!r.conditions.some(([c]) => c === 'other')) throw new Error(`no other in ${r.locales[0]}`)
  out.push('    Category::Other')
  out.push('}')
})
out.push('')
out.push(`/// The rule sets, in the order of their first locale.`)
out.push(`pub(crate) static RULES: [Rule; ${rules.length}] = [`)
rules.forEach((r, i) => {
  const mask = r.conditions.reduce((m, [c]) => m | (1 << ORDER.indexOf(c)), 0)
  out.push(`    Rule { select: ${fnName(i)}, categories: 0b${mask.toString(2).padStart(6, '0')} },`)
})
out.push('];')
out.push('')
// String tables are one concatenated `&str` and the end offset of each entry: a `&str` per entry
// would cost 8 bytes of pointer and length (16 on 64-bit) besides its text.
const ends = (list) => {
  let at = 0
  return list.map((s) => (at += s.length))
}
const strings = (name, doc, list) => {
  if (list.join('').length > 65535) throw new Error(`${name} does not fit u16 offsets`)
  out.push(doc)
  out.push(`pub(crate) static ${name}: &str = ${JSON.stringify(list.join(''))};`)
  out.push(`pub(crate) static ${name}_ENDS: [u16; ${list.length}] = [${ends(list).join(', ')}];`)
}
strings('LOCALES', '/// The locales `Intl.PluralRules` supports by their own name, sorted by bytes.', available)
out.push('/// The rule set of each of [`LOCALES`].')
out.push(`pub(crate) static LOCALE_RULES: [u8; ${available.length}] = [${available.map((l) => ruleOf.get(l)).join(', ')}];`)
out.push('')
strings(
  'ALIAS_KEYS',
  '/// Language aliases that change the rule (`iw` → `he`, `art-lojban` → `jbo`): the lowercase\n/// language, or `language-variant`, sorted by bytes.',
  languageAliases.map(([k]) => k),
)
strings('ALIAS_VALUES', '/// The CLDR replacement of each of [`ALIAS_KEYS`].', languageAliases.map(([, v]) => v))
out.push('/// Whether each alias applies to the bare lowercase tag (V8 passes `bh` through, not `bh-IN`).')
out.push(`pub(crate) static ALIAS_BARE: [bool; ${languageAliases.length}] = [${languageAliases.map(([, , b]) => b).join(', ')}];`)
out.push('')
out.push('/// Region aliases onto a region a locale above has (`620` → `PT`). Sorted by bytes.')
out.push(`pub(crate) static REGION_ALIASES: [(&str, &str); ${regionAliases.length}] = [`)
for (const [k, v] of regionAliases) out.push(`    (${JSON.stringify(k)}, ${JSON.stringify(v)}),`)
out.push('];')
out.push('')
writeFileSync(here('../crates/i18n-next/src/plural/rules.rs'), out.join('\n'))
console.log(`rules.rs: ${rules.length} rule sets, ${available.length} locales, ${languageAliases.length} language aliases, ${regionAliases.length} region aliases`)

// ---- plural-categories.md: the oracle of tests/plural_props.rs ----
const DECIMALS = [
  0.001, 0.01, 0.1, 0.2, 0.3, 0.5, 0.9, 1.001, 1.01, 1.1, 1.2, 1.25, 1.5, 2.1, 2.2, 2.5, 3.3, 3.7, 4.4, 5.5, 6.6, 7.7, 10.1,
  10.5, 11.1, 12.12, 14.5, 21.1, 21.21, 22.2, 100.5, 101.1, 111.1, 1000.5, 1.234, 0.123, 2.011, 5.105,
]
const BIG = [
  1000000, 2000000, 10000000, 1e9, 1e12, 1e15, 2 ** 53, 2 ** 53 + 2, 1e17, 1e18, 2e18, 1e19, 1e20, 1e21, 2 ** 60, 2 ** 61,
  1.5e22, 123456789012345680000, 1e100, 1.7976931348623157e308,
]
const head = [
  `> oracle: Intl.PluralRules of node ${process.versions.node.split('.')[0]}, icu ${process.versions.icu}, cldr ${process.versions.cldr}; rules from cldr-core ${CLDR_VERSION}`,
  '> generated by scripts/cldr.mjs (pnpm -C scripts cldr), do not edit; read by tests/plural_props.rs',
  '> categories: z zero, o one, t two, f few, m many, x other; one letter per number of the list',
]
const lines = []
lines.push(['decimal_list()', JSON.stringify(DECIMALS.map(String).join(' '))])
lines.push(['big_list()', JSON.stringify(BIG.map(String).join(' '))])
for (const l of available) {
  const rules = new Intl.PluralRules(l)
  const letters = (list) => JSON.stringify(list.map((n) => LETTER[rules.select(n)]).join(''))
  lines.push([`categories(${JSON.stringify(l)})`, JSON.stringify(ORDER.filter((c) => rules.resolvedOptions().pluralCategories.includes(c)).join(' '))])
  lines.push([`integers(${JSON.stringify(l)})`, letters(Array.from({ length: 1001 }, (_, n) => n))])
  lines.push([`decimals(${JSON.stringify(l)})`, letters(DECIMALS)])
  lines.push([`big(${JSON.stringify(l)})`, letters(BIG)])
}
// Tags: every known locale and alias, with region / script / extensions, odd case, malformed ones.
const tags = new Set()
for (const l of available) {
  tags.add(l)
  tags.add(`${l}-US`)
  tags.add(l.toUpperCase())
}
for (const key of Object.keys(aliases.languageAlias)) {
  const tag = key.replace(/_/g, '-')
  tags.add(tag)
  tags.add(tag.toUpperCase())
  tags.add(`${tag}-PT`)
}
for (const key of Object.keys(aliases.territoryAlias)) tags.add(`pt-${key}`)
for (const t of [
  'de-AT', 'zh-Hant-TW', 'en_US', 'ua', 'uk-UA', 'xx', 'iw-IL', 'pt-PT', 'pt-pt', 'PT-pt', 'pt-Latn-PT', 'pt-620', 'pt-PT-1996',
  'pt-PT-u-nu-latn', 'pt-PT-x-foo', 'pt-PT-t-en', 'pt-u-rg-ptzzzz', 'kok-Latn', 'kok-Latn-IN', 'kok-IN', 'kok-Deva', 'sr-Latn-RS',
  'zh-Hans-CN', 'en-GB-oed', 'en-US-POSIX', 'root', 'und', 'x-foo', 'en-x-foo', 'i-klingon', 'zh-min-nan', '', ' en', 'en ',
  'en-', '-en', 'en--US', 'a', 'abcdefghi', 'abcde', 'en-a', 'en-a-bc', 'en-a-bc-a-bc', 'en-variant-variant', 'en-1996-1996',
  'en-US-1996', 'en-123', 'en-1234', 'en-Latn-US-1abc', 'en-Latn-US-abcd', 'en-Latnn', 'en-u', 'en-u-ca', 'en-u-ca-gregory',
  'en-t-a1', 'en-x', 'en-x-', 'EN-us', 'eN', 'ar-001', 'es-419', 'qaa', 'tlh', 'no', 'nb', 'nn', 'no-bok', 'sh-RS', 'mo-MD',
  'jw', 'tl-PH', 'in', 'ji', 'iw', 'hy-arevmda', 'zh-hakka', 'zh-guoyu', 'zh-yue', 'art-lojban', 'aa-saaho', 'cel-gaulish',
  'en-ab-cd', 'en-u-nu', 'de-DE-u-co-phonebk', 'de-CH-1901', 'sl-rozaj-biske-1994', 'ca-ES-valencia', 'en-Qaai', 'ja-JP-u-ca-japanese',
  'th-TH-u-nu-thai', 'ü', 'en-ü', 'ru-RU', 'ru-UA', 'uk', 'pl-PL', 'he-IL', 'ar-EG', 'cy-GB', 'ga-IE', 'br-FR', 'mt-MT', 'lv-LV',
  // a variant alias after other variants; a repeat past the 8th variant or in other case; repeats inside `-t-`
  'aa-1996-abcde-saaho', 'hy-1996-abcde-arevmda', 'aa-saaho-1996', 'aa-SAAHO', 'art-1996-lojban',
  'en-a1234-b1234-c1234-d1234-e1234-f1234-g1234-h1234-i1234-a1234',
  'en-a1234-b1234-c1234-d1234-e1234-f1234-g1234-h1234-I1234-i1234', 'en-1996-abcde-ABCDE',
  'en-t-zh-1996-1996', 'en-t-zh-1996-1996a', 'en-t-zh-abcde-ABCDE', 'en-t-zh-1996-1996-h0-hybrid', 'en-t-zh-1996-h0-hybrid',
  // singletons: a digit and the letter 36 code points later are different (`6` / `v`, `0` / `p`)
  'en-v-aa-6-bb', 'en-6-aa-v-bb', 'en-0-aa-p-bb', 'en-9-aa-y-bb', 'en-Latn-z-Bd-v-0Ad-6-50x6324', 'ru-p-5656-nu-ddfd-0-mkhduq',
  'en-6-aa-6-bb', 'en-6-aa-x-6', 'ru-u-nu-latn', 'ru-t-en-h0-hybrid',
  // known difference (fixtures/compat/README.md): ICU accepts this after the repeated key
  'ru-u-rg-rg-a1',
]) tags.add(t)
for (const t of [...tags].sort(utf8)) {
  const r = intl(t)
  lines.push([`tag(${JSON.stringify(t)})`, r.kind === 'ok' ? JSON.stringify(r.locale) : `<${r.kind}>`])
}
const byCall = new Map(lines.map(([c, r]) => [c, `${c} → ${r}`]))
const body = [...byCall.values()].sort(utf8)
writeFileSync(
  here('../fixtures/compat/__golden__/plural-categories.md'),
  ['# compat: plural categories', '', ...head, '', '```text', ...body, '```', ''].join('\n'),
)
console.log(`plural-categories.md: ${body.length} lines`)

// ==== M5: number data (DESIGN 5.2, 8b.14) ====
//
// Reads cldr-numbers-full `numbers.json` / `currencies.json` of every locale and cldr-core
// supplemental data, writes `crates/i18n-next/src/format/data/numbers.txt`: one section per
// locale with only the fields that differ from its CLDR parent (parentLocales, else
// truncation, else root), plus the tables that resolve a tag. `build.rs` reads it, keeps the
// locales of `I18N_NEXT_LOCALES` (default: CLDR `modern`) with their parent chains, and
// writes them as Rust statics. Autonyms come from Node's ICU (`Intl.DisplayNames`, the same
// CLDR 48): cldr-localenames-full is not a dependency.
//
// It also checks the tag resolution `format/locale.rs` does (mirrored below as `resolveTag`)
// against `Intl.NumberFormat` on every locale and the tags derived from likelySubtags and
// aliases, and writes the oracle `fixtures/compat/__golden__/number-locales.md`.

const NUM = here('./node_modules/cldr-numbers-full/main')
const supp = (name) => JSON.parse(readFileSync(here(`./node_modules/cldr-core/supplemental/${name}.json`), 'utf8')).supplemental
const parentLocale = supp('parentLocales').parentLocales.parentLocale
const numberingSystems = supp('numberingSystems').numberingSystems
const fractions = supp('currencyData').currencyData.fractions
const likely = supp('likelySubtags').likelySubtags
const coverage = JSON.parse(readFileSync(here('./node_modules/cldr-core/coverageLevels.json'), 'utf8')).effectiveCoverageLevels
const defaultContent = new Set(JSON.parse(readFileSync(here('./node_modules/cldr-core/defaultContent.json'), 'utf8')).defaultContent)
// Only the locales ICU has: Intl formats a CLDR locale it lacks (`ht`, `la`) as `en`.
const all = readdirSync(NUM)
  .filter((l) => l === 'und' || Intl.NumberFormat.supportedLocalesOf(l).length > 0)
  .sort(utf8)
const have = new Set(all)
const modern = Object.keys(coverage).filter((l) => coverage[l] === 'modern' && have.has(l)).sort(utf8)
const parentOf = (l) => {
  if (l === 'und') return null
  if (parentLocale[l]) return parentLocale[l] === 'root' ? 'und' : parentLocale[l]
  // truncation, past default-content locales that have no data (`ca-ES-valencia` → `ca`)
  for (let p = l; p.includes('-'); ) {
    p = p.slice(0, p.lastIndexOf('-'))
    if (have.has(p)) return p
  }
  return 'und'
}
for (const l of all) if (l !== 'und' && !have.has(parentOf(l))) throw new Error(`parent ${parentOf(l)} of ${l} has no data`)

const SPACING = JSON.stringify({
  beforeCurrency: { currencyMatch: '[[:^S:]&[:^Z:]]', surroundingMatch: '[:digit:]', insertBetween: ' ' },
  afterCurrency: { currencyMatch: '[[:^S:]&[:^Z:]]', surroundingMatch: '[:digit:]', insertBetween: ' ' },
})
const CATS = ['zero', 'one', 'two', 'few', 'many', 'other']
const edge = new Set()
const clean = (v, what) => {
  if (typeof v !== 'string' || /[\t\n\r]/.test(v)) throw new Error(`bad ${what}: ${JSON.stringify(v)}`)
  return v
}
// Every field of a locale, resolved (cldr-json is resolved: a locale carries what it inherits).
function fieldsOf(l) {
  const n = JSON.parse(readFileSync(`${NUM}/${l}/numbers.json`, 'utf8')).main[l].numbers
  const cur = JSON.parse(readFileSync(`${NUM}/${l}/currencies.json`, 'utf8')).main[l].numbers.currencies
  const ns = n.defaultNumberingSystem
  const nsys = numberingSystems[ns]
  if (nsys._type !== 'numeric') throw new Error(`${l}: numbering system ${ns} is not numeric`)
  const sym = n[`symbols-numberSystem-${ns}`]
  const dec = n[`decimalFormats-numberSystem-${ns}`]
  const cf = n[`currencyFormats-numberSystem-${ns}`]
  if (JSON.stringify(cf.currencySpacing) !== SPACING) throw new Error(`${l}: other currency spacing`)
  const f = new Map()
  const set = (k, v) => f.set(k, clean(v, `${l} ${k}`))
  if (ns !== 'latn') set('digits', nsys._digits)
  set('decimal', sym.decimal)
  set('group', sym.group)
  set('minus', sym.minusSign)
  set('percent', sym.percentSign)
  set('nan', sym.nan)
  // ICU's monetary separators: what currency formats use (`de-AT` groups `1.234,50 €`)
  if (sym.currencyDecimal) set('currencyDecimal', sym.currencyDecimal)
  if (sym.currencyGroup) set('currencyGroup', sym.currencyGroup)
  set('infinity', sym.infinity)
  set('decimalPattern', dec.standard)
  set('percentPattern', n[`percentFormats-numberSystem-${ns}`].standard)
  set('currencyPattern', cf.standard)
  set('minGrouping', n.minimumGroupingDigits)
  const compact = []
  for (const [k, v] of Object.entries(dec.short.decimalFormat)) {
    const m = /^1(0*)-count-(zero|one|two|few|many|other)$/.exec(k)
    if (!m) throw new Error(`${l}: compact key ${k}`)
    compact.push([m[1].length, m[2], clean(v, `${l} ${k}`)])
  }
  compact.sort((a, b) => a[0] - b[0] || CATS.indexOf(a[1]) - CATS.indexOf(b[1]))
  f.set('compact', compact.map((c) => c.join('\t')))
  const units = CATS.filter((c) => cf[`unitPattern-count-${c}`] !== undefined).map((c) => `${c}\t${clean(cf[`unitPattern-count-${c}`], 'unit')}`)
  f.set('unitPattern', units)
  // `name` is the autonym (8b.14); a locale ICU has no name for gets none.
  let name
  try {
    name = new Intl.DisplayNames([l], { type: 'language', fallback: 'none' }).of(l)
  } catch {}
  if (l !== 'und' && name && name !== l) set('name', name)
  const currencies = new Map()
  const names = new Map()
  for (const code of Object.keys(cur).sort(utf8)) {
    if (!/^[A-Z]{3}$/.test(code)) throw new Error(`${l}: currency ${code}`)
    const c = cur[code]
    const symbol = c.symbol !== undefined && c.symbol !== code ? clean(c.symbol, 'symbol') : ''
    const narrow = c['symbol-alt-narrow'] !== undefined && c['symbol-alt-narrow'] !== (symbol || code) ? clean(c['symbol-alt-narrow'], 'narrow') : ''
    for (const s of [symbol, narrow]) {
      for (const ch of s ? [[...s][0], [...s].at(-1)] : []) if (/[\p{S}\p{Z}]/u.test(ch)) edge.add(ch)
    }
    // a currency's own format (`EUR` in `en-150`): a pattern, with `.` and `,` unless given
    const pattern = c.pattern !== undefined ? clean(c.pattern, 'pattern') : ''
    const decimal = c.decimal !== undefined ? clean(c.decimal, 'decimal') : ''
    const group = c.group !== undefined ? clean(c.group, 'group') : ''
    if (symbol || narrow || pattern || decimal || group) currencies.set(code, `${symbol}\t${narrow}\t${pattern}\t${decimal}\t${group}`)
    // `currencyDisplay: name` reads count-<category>, then count-other, then displayName: keep
    // only what changes the answer
    const other = c['displayName-count-other']
    if (other !== undefined) names.set(`${code}\tother`, clean(other, 'name'))
    else if (c.displayName !== undefined) names.set(`${code}\tother`, clean(c.displayName, 'name'))
    for (const k of CATS) {
      const v = c[`displayName-count-${k}`]
      if (k !== 'other' && v !== undefined && v !== names.get(`${code}\tother`)) names.set(`${code}\t${k}`, clean(v, 'name'))
    }
  }
  return { f, currencies, names }
}
const fields = new Map(all.map((l) => [l, fieldsOf(l)]))

// ---- currency formats as ICU has them ----
// A few locales format currencies unlike their resolved cldr-json (`en-DE`, `en-NL`: like
// `en-001`, `-US$1,234.50`; `fr-CH`): the generic currency pattern and the monetary
// separators are read back from ICU (`formatToParts` with a currency whose symbol starts and
// ends with a symbol character, so no currency spacing is in the affixes) and written as the
// locale's own when they differ. The pattern parser mirrors build.rs.
const splitQ = (p) => {
  let q = false
  for (let i = 0; i < p.length; i++) {
    if (p[i] === "'") q = !q
    else if (p[i] === ';' && !q) return [p.slice(0, i), p.slice(i + 1)]
  }
  return [p, null]
}
const subPat = (p) => {
  let q = false
  let start = -1
  let end = 0
  for (let i = 0; i < p.length; i++) {
    const c = p[i]
    if (c === "'") q = !q
    else if (!q && '#0,.@'.includes(c)) {
      if (start < 0) start = i
      end = i + 1
    } else if (start >= 0 && !q) break
  }
  if (start < 0) start = p.length
  end = Math.max(end, start)
  return [p.slice(0, start), p.slice(start, end), p.slice(end)]
}
const affixJS = (raw, minus, percent) => {
  let out = ''
  let q = false
  for (let i = 0; i < raw.length; i++) {
    const c = raw[i]
    if (c === "'" && raw[i + 1] === "'") {
      out += "'"
      i++
    } else if (c === "'") q = !q
    else if (c === '-' && !q) out += minus
    else if (c === '%' && !q) out += percent
    else out += c
  }
  return out
}
const parsePat = (p, minus, percent) => {
  const [pos, neg] = splitQ(p)
  const [pre, num, suf] = subPat(pos)
  const groups = num.split('.')[0].split(',')
  const digits = (g) => [...g].filter((c) => c === '#' || c === '0').length
  const g1 = groups.length > 1 ? digits(groups.at(-1)) : 0
  const g2 = groups.length > 2 ? digits(groups.at(-2)) : g1
  const pp = affixJS(pre, minus, percent)
  const ps = affixJS(suf, minus, percent)
  const [np, ns] = neg === null ? [minus + pp, ps] : ((n) => [affixJS(n[0], minus, percent), affixJS(n[2], minus, percent)])(subPat(neg))
  return JSON.stringify({ pp, ps, np, ns, g1, g2 })
}
const resolvedOf = (l) => {
  const chain = []
  for (let x = l; x; x = parentOf(x)) chain.push(x)
  const get = (k) => chain.map((x) => fields.get(x).f.get(k)).find((v) => v !== undefined)
  const cur = (code) => chain.map((x) => fields.get(x).currencies.get(code)).find((v) => v !== undefined)
  return { get, cur }
}
const quote = (t) => (t === '' ? '' : `'${t.replace(/'/g, "''")}'`)
const affixPattern = (a) => a.split('¤').map(quote).join('¤')
let currencyFixes = 0
for (const l of all) {
  if (l === 'und') continue
  const r = resolvedOf(l)
  const code = ['GBP', 'USD', 'JPY', 'EUR', 'ILS', 'INR', 'KRW'].find((c) => {
    const [symbol, , pattern, decimal, group] = (r.cur(c) ?? '\t\t\t\t').split('\t')
    return symbol && !pattern && !decimal && !group && /^[\p{S}].*[\p{S}]$|^[\p{S}]$/u.test(symbol) && !/[\p{L}\p{N}]/u.test(symbol)
  })
  if (!code) continue
  const shape = (v) => {
    const parts = new Intl.NumberFormat(l, { style: 'currency', currency: code, minimumFractionDigits: 2, maximumFractionDigits: 2 }).formatToParts(v)
    const first = parts.findIndex((p) => p.type === 'integer')
    const last = parts.findLastIndex((p) => p.type === 'integer' || p.type === 'fraction')
    const text = (list) => list.map((p) => (p.type === 'currency' ? '¤' : p.value)).join('')
    const ints = parts.filter((p) => p.type === 'integer').map((p) => p.value.length)
    return {
      pre: text(parts.slice(0, first)),
      suf: text(parts.slice(last + 1)),
      group: parts.find((p) => p.type === 'group')?.value,
      decimal: parts.find((p) => p.type === 'decimal')?.value,
      g1: ints.length > 1 ? ints.at(-1) : 0,
      g2: ints.length > 2 ? ints.at(-2) : ints.length > 1 ? ints.at(-1) : 0,
    }
  }
  const pos = shape(1234567.25)
  const neg = shape(-1234567.25)
  const minus = r.get('minus')
  const icu = JSON.stringify({ pp: pos.pre, ps: pos.suf, np: neg.pre, ns: neg.suf, g1: pos.g1, g2: pos.g2 })
  const f = fields.get(l).f
  if (parsePat(r.get('currencyPattern'), minus, r.get('percent')) !== icu) {
    const num = pos.g1 === 0 ? '0.00' : pos.g1 === pos.g2 ? `#,${'#'.repeat(pos.g1 - 1)}0.00` : `#,${'#'.repeat(pos.g2)},${'#'.repeat(pos.g1 - 1)}0.00`
    const pattern = `${affixPattern(pos.pre)}${num}${affixPattern(pos.suf)};${affixPattern(neg.pre)}${num}${affixPattern(neg.suf)}`
    if (parsePat(pattern, minus, r.get('percent')) !== icu) throw new Error(`${l}: cannot spell ICU's currency pattern ${icu}`)
    f.set('currencyPattern', pattern)
    currencyFixes++
  }
  if (pos.group !== undefined && pos.group !== (r.get('currencyGroup') ?? r.get('group'))) f.set('currencyGroup', pos.group)
  if (pos.decimal !== undefined && pos.decimal !== (r.get('currencyDecimal') ?? r.get('decimal'))) f.set('currencyDecimal', pos.decimal)
}
// `currencyDisplay: name` as ICU lays it out (`my`: `{0} {1}`, where cldr-json says `{1} {0}`).
let unitFixes = 0
for (const l of all) {
  if (l === 'und') continue
  const parts = new Intl.NumberFormat(l, { style: 'currency', currency: 'USD', currencyDisplay: 'name' }).formatToParts(1234.5)
  let icu = ''
  let inNumber = false
  for (const p of parts) {
    const num = ['integer', 'group', 'decimal', 'fraction', 'minusSign'].includes(p.type)
    if (num && !inNumber) icu += '{0}'
    inNumber = num
    if (p.type === 'currency') icu += '{1}'
    else if (!num) icu += p.value
  }
  const chain = []
  for (let x = l; x; x = parentOf(x)) chain.push(x)
  const units = chain.map((x) => fields.get(x).f.get('unitPattern')).find((u) => u && u.length > 0) ?? []
  const other = units.find((u) => u.startsWith('other\t'))?.slice(6)
  if (other !== undefined && other !== icu) {
    fields.get(l).f.set('unitPattern', [`other\t${icu}`])
    unitFixes++
  }
}
// The plain separators too (`fr-CH` groups by `'` in ICU, by U+202F in cldr-json).
let separatorFixes = 0
for (const l of all) {
  if (l === 'und') continue
  const r = resolvedOf(l)
  const parts = new Intl.NumberFormat(l, { minimumFractionDigits: 1 }).formatToParts(1234567.5)
  const f = fields.get(l).f
  for (const [type, key] of [['group', 'group'], ['decimal', 'decimal']]) {
    const v = parts.find((p) => p.type === type)?.value
    if (v !== undefined && v !== r.get(key)) {
      f.set(key, v)
      separatorFixes++
    }
  }
}
// A currency's own format (`EUR` in `en-150`) applies where ICU's lookup of that currency
// ends at an entry with one: cldr-json is resolved and cannot tell, ICU can (`en-AT` keeps its
// own `€ 1.234,50`). Kept when ICU shows it, dropped when ICU shows the generic one.
let ownFormats = 0
for (const l of all) {
  if (l === 'und') continue
  const r = resolvedOf(l)
  for (const [code, v] of fields.get(l).currencies) {
    const [symbol, narrow, pattern, decimal, group] = v.split('\t')
    if (!pattern) continue
    const minus = r.get('minus')
    const parts = (n) => new Intl.NumberFormat(l, { style: 'currency', currency: code, minimumFractionDigits: 2, maximumFractionDigits: 2 }).formatToParts(n)
    const strip = (t) => t.replace(/[   ]/g, '')
    const shapeOf = (n) => {
      const ps = parts(n)
      const first = ps.findIndex((p) => p.type === 'integer')
      const last = ps.findLastIndex((p) => p.type === 'integer' || p.type === 'fraction')
      const text = (list) => strip(list.map((p) => (p.type === 'currency' ? '¤' : p.value)).join(''))
      return [text(ps.slice(0, first)), text(ps.slice(last + 1)), ps.find((p) => p.type === 'group')?.value, ps.find((p) => p.type === 'decimal')?.value]
    }
    const icu = JSON.stringify([shapeOf(1234567.25), shapeOf(-1234567.25)])
    const expect = (pat, dec, grp) => {
      const p = JSON.parse(parsePat(pat, minus, r.get('percent')))
      return JSON.stringify([[strip(p.pp), strip(p.ps), grp, dec], [strip(p.np), strip(p.ns), grp, dec]])
    }
    const own = expect(pattern, decimal || '.', group || ',')
    const generic = expect(r.get('currencyPattern'), r.get('currencyDecimal') ?? r.get('decimal'), r.get('currencyGroup') ?? r.get('group'))
    if (icu === own) ownFormats++
    else if (icu === generic) fields.get(l).currencies.set(code, `${symbol}\t${narrow}\t\t\t`)
    else throw new Error(`${l} ${code}: ICU ${icu}, own format ${own}, generic ${generic}`)
  }
}

// ---- tag resolution, the mirror of format/locale.rs ----
const fullParts = (tag) => {
  const [lang, ...rest] = tag.split('-')
  let script = null
  let region = null
  for (const p of rest) {
    if (/^[A-Z][a-z]{3}$/.test(p)) script = p
    else if (/^([A-Z]{2}|\d{3})$/.test(p)) region = p
  }
  return { lang, script, region }
}
const likelyScript = (lang, region) => {
  const v = likely[`${lang}-${region}`] ?? likely[lang]
  return v ? fullParts(v).script : null
}
function pick({ lang, script, region }, set, scripts) {
  if (!script && region) script = scripts.get(`${lang}-${region}`) ?? null
  const cands = []
  if (script && region) cands.push(`${lang}-${script}-${region}`)
  if (script) cands.push(`${lang}-${script}`)
  if (region) cands.push(`${lang}-${region}`)
  cands.push(lang)
  return cands.find((c) => c !== 'und' && set.has(c)) ?? 'en'
}
// Script insertion (`zh-TW` → `zh-Hant-TW`, `sr-ME` → `sr-Latn-ME`): where the likely script
// of `lang-REGION` leads to another locale than the tag without it.
const scripts = new Map()
{
  const langs = new Set(all.filter((l) => fullParts(l).script).map((l) => fullParts(l).lang))
  const regions = new Set()
  for (const l of all) if (fullParts(l).region) regions.add(fullParts(l).region)
  for (const k of Object.keys(likely)) if (fullParts(k).region && !k.startsWith('und')) regions.add(fullParts(k).region)
  for (const lang of [...langs].sort(utf8)) {
    for (const region of [...regions].sort(utf8)) {
      const s = likelyScript(lang, region)
      // ICU aliases `lang-REGION` only onto a locale it has (with data or as default content)
      if (!s || !(have.has(`${lang}-${s}-${region}`) || defaultContent.has(`${lang}-${s}-${region}`))) continue
      const without = pick({ lang, script: null, region }, have, new Map())
      const withS = pick({ lang, script: s, region }, have, new Map())
      if (without !== withS) scripts.set(`${lang}-${region}`, s)
    }
  }
}
// Language and region aliases that lead elsewhere (`sh` → `sr-Latn`, `iw` → `he`, `620` → `PT`).
const languageAliasTable = []
for (const key of Object.keys(aliases.languageAlias).filter((k) => /^[a-z]{2,3}$/.test(k)).sort(utf8)) {
  let canon
  try {
    canon = Intl.getCanonicalLocales(key)[0]
  } catch {
    continue
  }
  if (canon === key) continue
  if (pick(fullParts(canon), have, scripts) !== pick({ lang: key, script: null, region: null }, have, scripts)) languageAliasTable.push([key, canon])
}
const dataRegions = new Set(all.map((l) => fullParts(l).region).filter(Boolean))
const regionAliasTable = Object.entries(aliases.territoryAlias)
  .filter(([key, { _replacement }]) => /^([A-Z]{2}|\d{3})$/.test(key) && dataRegions.has(_replacement.split(' ')[0]))
  .map(([key, { _replacement }]) => [key, _replacement.split(' ')[0]])
  .sort((a, b) => utf8(a[0], b[0]))
// What format/locale.rs reads from a tag: language, script and region as written (cased), with
// the aliases above applied. Only for tags Intl accepts; others go to `en` (FormatService).
function resolveTag(tag, set) {
  tag = tag.split('_').join('-')
  try {
    if (Intl.NumberFormat.supportedLocalesOf(tag).length === 0) return 'en'
  } catch {
    return 'en'
  }
  const subs = tag.split('-')
  let lang = subs[0].toLowerCase()
  let script = null
  let region = null
  for (const p of subs.slice(1)) {
    if (p.length === 1) break
    if (!script && !region && /^[a-zA-Z]{4}$/.test(p)) script = p[0].toUpperCase() + p.slice(1).toLowerCase()
    else if (!region && /^([a-zA-Z]{2}|\d{3})$/.test(p)) region = p.toUpperCase()
  }
  const alias = languageAliasTable.find(([k]) => k === lang)
  if (alias) {
    const a = fullParts(alias[1])
    lang = a.lang
    script = script ?? a.script
    region = region ?? a.region
  }
  const ra = region && regionAliasTable.find(([k]) => k === region)
  if (ra) region = ra[1]
  return pick({ lang, script, region }, set, scripts)
}
// the spelling of fixtures/compat/README.md: JSON plus invisible code units as \uXXXX
const INVISIBLE = /[\u007f-\u00a0\u00ad\u1680\u180e\u2000-\u200f\u2028-\u202f\u205f-\u206f\u3000\ufe00-\ufe0f\ufeff\ufff9-\ufffb]/g
const spell = (str) => JSON.stringify(str).replace(INVISIBLE, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`)
const fmt = (tag, o, v) => new Intl.NumberFormat(tag, o).format(v)
const sig = (tag) =>
  [
    fmt(tag, {}, -1234567.891),
    fmt(tag, { style: 'percent' }, 0.256),
    fmt(tag, { style: 'currency', currency: 'EUR' }, 1234.5),
    fmt(tag, { style: 'currency', currency: 'USD' }, -1234.5),
    fmt(tag, { style: 'currency', currency: 'JPY', currencyDisplay: 'code' }, 1234),
    fmt(tag, { style: 'currency', currency: 'USD', currencyDisplay: 'narrowSymbol' }, 12),
    fmt(tag, { notation: 'compact' }, 1234567),
    fmt(tag, { notation: 'compact' }, 1234),
    fmt(tag, {}, NaN),
  ].join(' | ')
const intlTag = (tag) => {
  const t = tag.split('_').join('-')
  try {
    return Intl.NumberFormat.supportedLocalesOf(t).length > 0 ? t : 'en'
  } catch {
    return 'en'
  }
}
const closure = (list) => {
  const out = new Set(['und', 'en'])
  for (let l of list) for (; l; l = parentOf(l)) out.add(l)
  return out
}
const modernSet = closure(modern)
const sigTags = new Set()
for (const l of all) {
  sigTags.add(l)
  sigTags.add(l.toLowerCase())
}
for (const k of Object.keys(likely)) if (!k.startsWith('und') && have.has(k.split('-')[0])) sigTags.add(k)
for (const [k] of languageAliasTable) {
  sigTags.add(k)
  sigTags.add(`${k}-US`)
}
for (const [k] of regionAliasTable) {
  sigTags.add(`pt-${k}`)
  sigTags.add(`en-${k}`)
}
for (const t of ['zh-TW', 'zh-HK', 'zh-MO', 'zh-SG', 'sr-ME', 'sr-RS', 'sr-BA', 'pa-PK', 'uz-AF', 'az-IR', 'yue-CN', 'en_US', 'de_CH', 'xx', 'root', 'und', 'en-US-x-foo', 'de-AT-1996', 'ZH-hant-tw', 'iw-IL', 'no', 'nb-NO', 'zh-Hant-XX', 'de-XX', '']) sigTags.add(t)
const localeLines = []
let skipped = 0
for (const tag of [...sigTags].sort(utf8)) {
  const js = sig(intlTag(tag))
  const m = resolveTag(tag, modernSet)
  if (sig(m) === js) localeLines.push([`locale(${spell(tag)})`, spell(js)])
  // A tag that resolves inside the modern set must agree; below it (ICU lacks or aliases some
  // `basic` locales its own way: `ku-IQ` gives root) the default build has no data anyway.
  else if (modernSet.has(resolveTag(tag, have))) throw new Error(`resolution of ${JSON.stringify(tag)}: ${m} gives ${sig(m)}, Intl ${js}`)
  else skipped++
}
// `language_name` names the data locale a tag resolves to (`ca-ES-valencia` → `ca`)
for (const l of modern.filter((l) => have.has(l) && l !== 'und')) {
  const name = fields.get(resolveTag(l, modernSet)).f.get('name')
  localeLines.push([`autonym(${spell(l)})`, name === undefined ? '<none>' : spell(name)])
}

// ---- numbers.txt: each locale as the difference from its parent ----
const txt = []
txt.push(`# generated by scripts/cldr.mjs from cldr-core / cldr-numbers-full ${CLDR_VERSION} and Node ICU ${process.versions.icu} (autonyms) — do not edit`)
txt.push('# read by crates/i18n-next/build.rs; format: line kinds below, a locale section holds the fields that differ from its parent')
txt.push(`modern ${modern.join(' ')}`)
txt.push(`edge ${[...edge].sort(utf8).join('')}`)
for (const code of Object.keys(fractions).sort(utf8)) {
  const d = fractions[code]._digits
  if (code !== 'DEFAULT' && d !== fractions.DEFAULT._digits) txt.push(`fraction ${code} ${d}`)
}
if (fractions.DEFAULT._digits !== '2') throw new Error('default currency digits are not 2')
for (const [k, v] of languageAliasTable) txt.push(`alias ${k} ${v}`)
for (const [k, v] of regionAliasTable) txt.push(`region ${k} ${v}`)
for (const [k, v] of [...scripts].sort((a, b) => utf8(a[0], b[0]))) txt.push(`script ${k} ${v}`)
let diffLines = 0
// currency display names (feature `currency-names`), as their own file: build.rs reads it only then
const names = [
  `# generated by scripts/cldr.mjs from cldr-numbers-full ${CLDR_VERSION} — do not edit`,
  '# read by crates/i18n-next/build.rs with the `currency-names` feature: per locale, `CODE<tab>category<tab>name` that differ from the parent',
]
for (const l of all) {
  const own = fields.get(l)
  const p = parentOf(l)
  const base = p === null ? null : fields.get(p)
  txt.push(`locale ${l} ${p ?? '-'}`)
  const before = txt.length
  for (const [k, v] of own.f) {
    const pv = base?.f.get(k)
    if (Array.isArray(v)) {
      if (base && JSON.stringify(v) === JSON.stringify(pv)) continue
      for (const e of v) txt.push(`${k}\t${e}`)
      // an empty list still overrides
      if (v.length === 0) txt.push(k)
    } else if (v !== pv) txt.push(`${k}\t${v}`)
  }
  // a field the parent has and the locale lacks (only `digits`, `name`): reset it
  if (base) for (const k of base.f.keys()) if (!own.f.has(k)) txt.push(`${k}\t`)
  for (const [code, v] of own.currencies) if (base?.currencies.get(code) !== v) txt.push(`currency\t${code}\t${v}`)
  if (base) for (const code of base.currencies.keys()) if (!own.currencies.has(code)) txt.push(`currency\t${code}\t\t\t\t\t`)
  diffLines += txt.length - before
  names.push(`locale ${l} ${p ?? '-'}`)
  for (const [k, v] of own.names) if (base?.names.get(k) !== v) names.push(`${k}\t${v}`)
  if (base) for (const k of base.names.keys()) if (!own.names.has(k)) names.push(`${k}\t`)
}
txt.push('')
mkdirSync(here('../crates/i18n-next/src/format/data'), { recursive: true })
writeFileSync(here('../crates/i18n-next/src/format/data/numbers.txt'), txt.join('\n'))
names.push('')
writeFileSync(here('../crates/i18n-next/src/format/data/currency-names.txt'), names.join('\n'))
console.log(`numbers.txt: ${all.length} locales (${modern.length} modern), ${diffLines} field lines, ${currencyFixes} currency patterns from ICU, ${ownFormats} own currency formats, ${separatorFixes} separators, ${unitFixes} unit patterns from ICU, ${scripts.size} script, ${languageAliasTable.length} alias, ${regionAliasTable.length} region entries`)
writeFileSync(
  here('../fixtures/compat/__golden__/number-locales.md'),
  [
    '# compat: number locales',
    '',
    `> oracle: Intl.NumberFormat / Intl.DisplayNames of node ${process.versions.node.split('.')[0]}, icu ${process.versions.icu}, cldr ${process.versions.cldr}; data from cldr-numbers-full ${CLDR_VERSION}`,
    '> generated by scripts/cldr.mjs (pnpm -C scripts cldr), do not edit; read by tests/number_compat.rs',
    '> locale(tag): decimal -1234567.891 | percent 0.256 | EUR 1234.5 | USD -1234.5 | JPY code 1234 | USD narrow 12 | compact 1234567 | compact 1234 | NaN',
    '',
    '```text',
    ...localeLines.map(([c, r]) => `${c} → ${r}`).sort(utf8),
    '```',
    '',
  ].join('\n'),
)
console.log(`number-locales.md: ${localeLines.length} lines (${skipped} tags outside the modern set skipped)`)
