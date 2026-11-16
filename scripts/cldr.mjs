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
// aliases, and writes the oracles `fixtures/compat/__golden__/number-locales.md` (the `modern` set)
// and `number-locales-all.md` (every locale, for a build with `I18N_NEXT_LOCALES=all`).

const NUM = here('./node_modules/cldr-numbers-full/main')
const supp = (name) => JSON.parse(readFileSync(here(`./node_modules/cldr-core/supplemental/${name}.json`), 'utf8')).supplemental
const parentLocale = supp('parentLocales').parentLocales.parentLocale
const numberingSystems = supp('numberingSystems').numberingSystems
const fractions = supp('currencyData').currencyData.fractions
const likely = supp('likelySubtags').likelySubtags
const coverage = JSON.parse(readFileSync(here('./node_modules/cldr-core/coverageLevels.json'), 'utf8')).effectiveCoverageLevels
const defaultContent = new Set(JSON.parse(readFileSync(here('./node_modules/cldr-core/defaultContent.json'), 'utf8')).defaultContent)
// Only the locales ICU has: Intl formats a CLDR locale it lacks (`ht`, `la`) as `en`, and one
// it resolves to a shorter name (`az-Arab` → `az`, `be-tarask` → `be`, `nb-SJ` → `nb`) as that.
const all = readdirSync(NUM)
  .filter((l) => l === 'und' || (Intl.NumberFormat.supportedLocalesOf(l).length > 0 && new Intl.NumberFormat(l).resolvedOptions().locale === l))
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
// default numbering systems other than `latn`: name → digits (the `numberingSystem` option)
const numbering = new Map()
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
  if (ns !== 'latn') {
    set('digits', nsys._digits)
    numbering.set(ns, nsys._digits)
  }
  // the rule set `Intl.PluralRules` gives the locale (rules.rs order; 255: none): compact
  // patterns and currency names by plural form
  set('plural', String(ruleIndex(intl(l)) ?? 255))
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
    const ints = parts.filter((p) => p.type === 'integer').map((p) => [...p.value].length)
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

// ---- `currencyDisplay: name` as ICU lays it out ----
// The unit pattern of each plural form (`my`: `{0} {1}`, where cldr-json says `{1} {0}`). Some
// locales (`en-DE`, `en-AT`, `pt-CV`) show the number with the currency pattern and symbol there
// (`-US$1,234.50 US dollars`): their `{0}` is the number as `currencyDisplay: symbol` shows it,
// flagged by `nameCurrency`; so is a currency with its own format (`EUR` in `en-150`). ICU joins
// the unit pattern's text and the name into one part (`de yeni japonezi`): the name is found by
// the data, the pattern is what is left.
const nameParts = (l, code, v) => new Intl.NumberFormat(l, { style: 'currency', currency: code, currencyDisplay: 'name' }).formatToParts(v)
const nameOut = (l, code, v) => nameParts(l, code, v).map((p) => p.value).join('')
const symbolParts = (l, code, v) => new Intl.NumberFormat(l, { style: 'currency', currency: code }).formatToParts(v)
const symbolOut = (l, code, v) => symbolParts(l, code, v).map((p) => p.value).join('')
const chainOf = (l) => {
  const chain = []
  for (let x = l; x; x = parentOf(x)) chain.push(x)
  return chain
}
const ownPattern = (l, code) => (resolvedOf(l).cur(code) ?? '').split('\t')[2] !== '' && resolvedOf(l).cur(code) !== undefined
const symbolNumber = (l, code) => fields.get(l).f.get('nameCurrency') === '1' || ownPattern(l, code)
// ICU's known garble (fixtures/compat/README.md): a number with the symbol where currency
// spacing goes in (`CHF 1,234.5 Swiss francs0`, `en-DE`): the name lands one char early.
const garbled = (l, code, v) => {
  if (!symbolNumber(l, code)) return false
  const ps = symbolParts(l, code, v)
  const at = ps.findIndex((p) => p.type === 'currency')
  return /\p{L}/u.test(ps[at].value.at(0) + ps[at].value.at(-1)) && ps.some((p, i) => p.type === 'literal' && Math.abs(i - at) === 1)
}
const pluralOf = (l, code) => {
  const digits = new Intl.NumberFormat(l, { style: 'currency', currency: code }).resolvedOptions().minimumFractionDigits
  // a locale without plural rules (`kgp`) names by `other`, as ICU's root rules
  if (Intl.PluralRules.supportedLocalesOf(l).length === 0) return () => 'other'
  const rules = new Intl.PluralRules(l, { minimumFractionDigits: digits, maximumFractionDigits: digits })
  return (v) => rules.select(v)
}
const labelOf = (names, code, cat) => names(`${code}\t${cat}`) || names(`${code}\tother`) || code
const namesOf = (l) => (k) => chainOf(l).map((x) => fields.get(x).names.get(k)).find((n) => n !== undefined)
const numberText = (l, code, v) => {
  if (symbolNumber(l, code)) return symbolOut(l, code, v)
  const ps = nameParts(l, code, v)
  // the number, with the bidi marks around its sign (`ar`: U+200E before `-`)
  const num = (p) => ['integer', 'group', 'decimal', 'fraction', 'minusSign', 'nan', 'infinity'].includes(p.type) || (p.type === 'literal' && /^[\u200e\u200f\u061c]+$/u.test(p.value))
  return ps.slice(ps.findIndex(num), ps.findLastIndex(num) + 1).map((p) => p.value).join('')
}
const unitsOf = (l) => new Map((chainOf(l).map((x) => fields.get(x).f.get('unitPattern')).find((u) => u && u.length > 0) ?? []).map((u) => u.split('\t')))
// What Rust shows with `names`: the unit pattern of the plural form, the number, the name.
const predicted = (l, code, v, names = namesOf(l)) => {
  const cat = pluralOf(l, code)(v)
  const units = unitsOf(l)
  return (units.get(cat) ?? units.get('other') ?? '{0} {1}').replace('{0}', numberText(l, code, v)).replace('{1}', labelOf(names, code, cat))
}
const UNIT_SAMPLES = [0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 15, 20, 21, 22, 100, 101, 102, 1000000, 1234.5, 0.5, 1.5]
let unitFixes = 0
let nameCurrency = 0
for (const l of all) {
  if (l === 'und') continue
  if ([1234.5, -1234.5].every((v) => nameOut(l, 'USD', v).includes(symbolOut(l, 'USD', v)))) {
    fields.get(l).f.set('nameCurrency', '1')
    nameCurrency++
  }
  const rules = pluralOf(l, 'USD')
  const units = unitsOf(l)
  const names = namesOf(l)
  const fixed = new Map()
  let differs = false
  for (const v of UNIT_SAMPLES) {
    const cat = rules(v)
    if (fixed.has(cat) || garbled(l, 'USD', v)) continue
    const out = nameOut(l, 'USD', v)
    const rest = out.replace(numberText(l, 'USD', v), '\u0000')
    const label = labelOf(names, 'USD', cat)
    let icu
    if (rest.includes(label)) icu = rest.replace(label, '{1}').replace('\u0000', '{0}')
    else {
      // names unlike ICU's (`sr-Cyrl-ME`, fixed below): the part `currency` is the name
      icu = ''
      for (const p of nameParts(l, 'USD', v)) icu += p.type === 'currency' ? '{1}' : p.value
      icu = icu.replace(numberText(l, 'USD', v), '{0}')
    }
    if (!icu.includes('{0}') || !icu.includes('{1}')) throw new Error(`${l}: name layout ${icu} of ${out}`)
    fixed.set(cat, icu)
    if (icu !== (units.get(cat) ?? units.get('other'))) differs = true
  }
  if (differs) {
    fields.get(l).f.set('unitPattern', CATS.filter((c) => fixed.has(c)).map((c) => `${c}\t${fixed.get(c)}`))
    unitFixes++
  }
}
// Currency names as ICU has them: `sr-Cyrl-ME` names currencies in Latin (`američkih dolara`), as
// `sr-Latn-ME`. A locale whose names differ from ICU takes the names of the locale ICU shows.
const NAME_SAMPLE = ['USD', 'EUR', 'CHF', 'JPY', 'GBP', 'RUB', 'CNY', 'INR', 'XYZ']
const NAME_VALUES = [1, 2, 5, 21, -1234.5]
const cases = (l) => NAME_SAMPLE.flatMap((c) => NAME_VALUES.map((v) => [c, v])).filter(([c, v]) => !garbled(l, c, v))
const icuNames = (l) => cases(l).map(([c, v]) => nameOut(l, c, v)).join('|')
const ownNames = (l, names = namesOf(l)) => cases(l).map(([c, v]) => predicted(l, c, v, names)).join('|')
let nameDonors = 0
let nameGarbles = 0
for (const l of all) {
  if (l === 'und') continue
  nameGarbles += NAME_SAMPLE.length * NAME_VALUES.length - cases(l).length
  const icu = icuNames(l)
  if (ownNames(l) === icu) continue
  const donor = all.find((m) => m !== 'und' && m !== l && ownNames(l, namesOf(m)) === icu)
  if (!donor) throw new Error(`${l}: currency names differ from ICU and no locale has ICU's:\n${ownNames(l)}\n${icu}`)
  const names = new Map()
  for (const x of chainOf(donor)) for (const [k, v] of fields.get(x).names) if (!names.has(k)) names.set(k, v)
  for (const k of fields.get(l).names.keys()) if (!names.has(k)) names.set(k, '')
  fields.get(l).names = names
  nameDonors++
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
  const v = (region && likely[`${lang}-${region}`]) ?? likely[lang]
  return v ? fullParts(v).script : null
}
// `lang-Script` locales whose CLDR parent is the root (`uz-Cyrl`, `bs-Cyrl`): left out of a
// build, a tag with that script formats with the root, not with `lang` of another script
const rootScripts = new Set(all.filter((l) => /^[a-z]+-[A-Z][a-z]{3}$/.test(l) && parentOf(l) === 'und'))
function pick({ lang, script, region }, set, scripts) {
  // a script written in the tag: ICU truncates and never tries `lang-REGION`
  const explicit = Boolean(script)
  if (!script && region) script = scripts.get(`${lang}-${region}`) ?? null
  if (script === '-') return 'und'
  const cands = []
  if (script && region) cands.push(`${lang}-${script}-${region}`)
  if (script) cands.push(`${lang}-${script}`)
  if (region && !explicit) cands.push(`${lang}-${region}`)
  const hit = cands.find((c) => c !== 'und' && set.has(c))
  if (hit) return hit
  if (script && rootScripts.has(`${lang}-${script}`)) return 'und'
  return set.has(lang) && lang !== 'und' ? lang : 'en'
}
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
// Script insertion (`zh-TW` → `zh-Hant-TW`, `sr-ME` → `sr-Latn-ME`): where the likely script
// of `lang-REGION` leads to another locale than the tag without it, and ICU formats
// `lang-REGION` as that locale (ICU aliases `sr-ME`, not `az-IR`).
const scripts = new Map()
{
  const langs = new Set(all.filter((l) => fullParts(l).script).map((l) => fullParts(l).lang))
  const regions = new Set()
  for (const l of all) if (fullParts(l).region) regions.add(fullParts(l).region)
  for (const k of Object.keys(likely)) if (fullParts(k).region && !k.startsWith('und')) regions.add(fullParts(k).region)
  for (const lang of [...langs].sort(utf8)) {
    for (const region of [...regions].sort(utf8)) {
      const s = likelyScript(lang, region)
      // ICU aliases `lang-REGION` only onto a locale CLDR has (with data or as default content)
      if (!s || !(have.has(`${lang}-${s}-${region}`) || defaultContent.has(`${lang}-${s}-${region}`))) continue
      const without = pick({ lang, script: null, region }, have, new Map())
      const withS = pick({ lang, script: s, region }, have, new Map())
      // a region that implies the language's usual script changes nothing
      if (without === withS && s === likelyScript(lang, null)) continue
      const js = sig(intlTag(`${lang}-${region}`))
      // ICU aliases `lang-REGION` onto `lang-Script-REGION`; where ICU has neither that nor
      // `lang-Script` (`ku-IQ` → `ku-Arab-IQ`), it formats with the root: `-`
      if (without !== withS && js === sig(withS)) scripts.set(`${lang}-${region}`, s)
      else if (js !== sig(without)) scripts.set(`${lang}-${region}`, '-')
    }
  }
}
// Language aliases that lead elsewhere (`sh` → `sr-Latn`, `iw` → `he`, `hy-arevmda` → `hyw`).
// V8 keeps a bare lowercase tag of some (`bh`: unsupported) and aliases it otherwise (`bh-US`,
// `BH` → `bho`): such an entry is marked not bare.
const languageAliasTable = []
const canonical = (t) => {
  try {
    return Intl.getCanonicalLocales(t)[0]
  } catch {
    return null
  }
}
for (const key of Object.keys(aliases.languageAlias).sort(utf8)) {
  const tag = key.split('_').join('-')
  const [lang, variant, more] = tag.split('-')
  if (!/^[a-z]{2,3}$/.test(lang) || more !== undefined || (variant !== undefined && !/^([a-z0-9]{5,8}|[0-9][a-z0-9]{3})$/.test(variant))) continue
  const canon = canonical(variant === undefined ? tag.toUpperCase() : tag)
  if (canon === null || canon.toLowerCase() === tag) continue
  if (pick(fullParts(canon), have, scripts) === pick({ lang, script: null, region: null }, have, scripts)) continue
  languageAliasTable.push([tag, canon, variant !== undefined || canonical(tag) !== tag])
}
const dataRegions = new Set(all.map((l) => fullParts(l).region).filter(Boolean))
const regionAliasTable = Object.entries(aliases.territoryAlias)
  .filter(([key, { _replacement }]) => /^([A-Z]{2}|\d{3})$/.test(key) && dataRegions.has(_replacement.split(' ')[0]))
  .map(([key, { _replacement }]) => [key, _replacement.split(' ')[0]])
  .sort((a, b) => utf8(a[0], b[0]))
// What format/locale.rs reads from a tag: language, script and region as written (cased),
// variants, with the aliases above applied; a tag `Intl` rejects goes to `en` (FormatService),
// as one with no data. Rust checks the shape itself (`plural::parse`), the mirror asks `Intl`.
function resolveTag(tag, set) {
  tag = tag.split('_').join('-')
  if (canonical(tag) === null) return 'en'
  const subs = tag.split('-')
  let lang = subs[0].toLowerCase()
  let script = null
  let region = null
  const variants = []
  for (const p of subs.slice(1)) {
    if (p.length === 1) break
    if (!script && !region && variants.length === 0 && /^[a-zA-Z]{4}$/.test(p)) script = p[0].toUpperCase() + p.slice(1).toLowerCase()
    else if (!region && variants.length === 0 && /^([a-zA-Z]{2}|\d{3})$/.test(p)) region = p.toUpperCase()
    else variants.push(p.toLowerCase())
  }
  const alias =
    variants.map((v) => languageAliasTable.find(([k]) => k === `${lang}-${v}`)).find(Boolean) ??
    languageAliasTable.find(([k, , bare]) => k === lang && (bare || tag !== lang))
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
  sigTags.add(k.toUpperCase())
  sigTags.add(`${k}-US`)
  const [lang, variant] = k.split('-')
  if (variant) sigTags.add(`${lang}-AM-1996-${variant}`)
}
for (const [k] of regionAliasTable) {
  sigTags.add(`pt-${k}`)
  sigTags.add(`en-${k}`)
}
// a script spelled out: the language's usual one before a region (`de-Latn-AT`, `en-Latn-IN`),
// and every script locale with a region it lacks (`uz-Cyrl-XX`)
for (const l of all) {
  const { lang, script, region } = fullParts(l)
  const usual = likelyScript(lang, null)
  if (!script && region && usual) sigTags.add(`${lang}-${usual}-${region}`)
  if (!script && !region && usual) sigTags.add(`${lang}-${usual}`)
  if (script && !region) sigTags.add(`${l}-XX`)
}
for (const t of ['zh-TW', 'zh-HK', 'zh-MO', 'zh-SG', 'sr-ME', 'sr-RS', 'sr-BA', 'pa-PK', 'uz-AF', 'az-IR', 'az-IQ', 'ku-IQ', 'ku-IR', 'az-Arab', 'ku-Arab', 'mn-Mong', 'ms-Arab', 'sat-Deva', 'bh', 'bh-US', 'BH', 'hy-arevmda', 'hy-arevela', 'zh-hakka', 'yue-CN', 'en_US', 'de_CH', 'xx', 'root', 'und', 'en-US-x-foo', 'de-AT-1996', 'ZH-hant-tw', 'iw-IL', 'no', 'nb-NO', 'zh-Hant-XX', 'de-XX', '']) sigTags.add(t)
// Known differences (fixtures/compat/README.md): ICU's own `en_US_POSIX` data (no grouping,
// `INF`) for `en-US-POSIX` / `en-US-u-va-posix`; Rust formats those as `en-US`.
const KNOWN = new Set(['en-US-POSIX', 'en-us-posix'])
const localeLines = []
const allLines = []
const failed = []
let skipped = 0
for (const tag of [...sigTags].sort(utf8)) {
  if (KNOWN.has(tag)) continue
  const js = sig(intlTag(tag))
  const a = resolveTag(tag, have)
  // the root (`ku-IQ`): Intl has no tag for it, tests/number_compat.rs checks Rust's output
  if (a !== 'und' && sig(a) !== js) failed.push(`${JSON.stringify(tag)}: ${a} gives ${sig(a)}, Intl ${js}`)
  allLines.push([`locale(${spell(tag)})`, spell(js)])
  const m = resolveTag(tag, modernSet)
  // a tag whose locale the modern set lacks resolves to a parent, or `en`, there
  if (sig(m) === js) localeLines.push([`locale(${spell(tag)})`, spell(js)])
  else skipped++
}
if (failed.length > 0) throw new Error(`tag resolution differs from Intl:\n${failed.join('\n')}`)
// `language_name` names the data locale a tag resolves to (`ca-ES-valencia` → `ca`)
const autonymLine = (l, set) => {
  const name = fields.get(resolveTag(l, set)).f.get('name')
  return [`autonym(${spell(l)})`, name === undefined ? '<none>' : spell(name)]
}
for (const l of modern.filter((l) => l !== 'und')) localeLines.push(autonymLine(l, modernSet))
for (const l of all.filter((l) => l !== 'und')) allLines.push(autonymLine(l, have))

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
for (const [name, digits] of [...numbering].sort((a, b) => utf8(a[0], b[0]))) txt.push(`numbering ${name} ${digits}`)
// `alias KEY TO [lone]`: `lone` applies to the bare lowercase key too
for (const [k, v, bare] of languageAliasTable) txt.push(`alias ${k} ${v}${bare ? ' lone' : ''}`)
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
console.log(`numbers.txt: ${all.length} locales (${modern.length} modern), ${diffLines} field lines, ${currencyFixes} currency patterns from ICU, ${ownFormats} own currency formats, ${separatorFixes} separators, ${unitFixes} unit patterns from ICU, ${nameCurrency} names after the currency pattern, ${nameDonors} name sets from ICU (${nameGarbles} garbled names of ICU skipped), ${scripts.size} script, ${languageAliasTable.length} alias, ${regionAliasTable.length} region entries`)
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
writeFileSync(
  here('../fixtures/compat/__golden__/number-locales-all.md'),
  [
    '# compat: number locales, I18N_NEXT_LOCALES=all',
    '',
    `> oracle: Intl.NumberFormat / Intl.DisplayNames of node ${process.versions.node.split('.')[0]}, icu ${process.versions.icu}, cldr ${process.versions.cldr}; data from cldr-numbers-full ${CLDR_VERSION}`,
    '> generated by scripts/cldr.mjs (pnpm -C scripts cldr), do not edit; read by tests/number_compat.rs in a build with I18N_NEXT_LOCALES=all',
    '> locale(tag): as in number-locales.md, every tag; autonym(locale): every locale ICU has',
    '',
    '```text',
    ...allLines.map(([c, r]) => `${c} → ${r}`).sort(utf8),
    '```',
    '',
  ].join('\n'),
)
console.log(`number-locales-all.md: ${allLines.length} lines`)

// ---- dates (M6) ----
// DESIGN 5.2 / 5.3, spikes S9 / S10. Names (months, weekdays, day periods, eras) and relative-time
// patterns come from cldr-dates-full; the pattern of every supported option combination comes
// from Node's ICU: `formatToParts` on probe dates, each field's symbol picked as the one that
// renders the field on every probe (single-digit values fix the width, the months and weekdays
// of the year fix width and context, hours 0 and 12 the cycle). `renderDate` below mirrors
// `format/date.rs`; every derived pattern and every combination the runtime builds from them
// (date and time joined by the locale's glue, forced hour cycles) is re-rendered and compared
// with Intl `format()` here, where V8 shows U+202F as a space. Writes
// `crates/i18n-next/src/format/data/dates.txt`: each locale as the difference from its parent.
// Dates are Gregorian only: ICU's patterns are derived with `calendar: 'gregory'` (`th`, `fa`
// default to other calendars, fixtures/compat/README.md).
const DAT = here('./node_modules/cldr-dates-full/main')
const sp = (s) => s.replace(/\u202f/g, ' ')
const DAYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat']
const WIDTHS = [['a', 'abbreviated'], ['w', 'wide'], ['n', 'narrow']]
const periodRules = supp('dayPeriods').dayPeriodRuleSet
const weekData = supp('weekData').weekData
// The tag ICU formats with the data of locale `l`: the root has none of its own (`ku-IQ` is
// aliased onto it, as for numbers).
const dateTag = (l) => (l === 'und' ? 'ku-IQ' : l)
function dateNames(l) {
  const g = JSON.parse(readFileSync(`${DAT}/${l}/ca-gregorian.json`, 'utf8')).main[l].dates.calendars.gregorian
  const n = new Map()
  const set = (k, list) => n.set(k, list.map((v) => sp(clean(v, `${l} ${k}`))))
  for (const [c, ctx] of [['f', 'format'], ['s', 'stand-alone']]) {
    for (const [w, wn] of WIDTHS) set(`M.${c}${w}`, Array.from({ length: 12 }, (_, i) => g.months[ctx][wn][String(i + 1)]))
    for (const [w, wn] of [...WIDTHS, ['s', 'short']]) set(`E.${c}${w}`, DAYS.map((d) => g.days[ctx][wn][d]))
  }
  const eraKey = { a: 'eraAbbr', w: 'eraNames', n: 'eraNarrow' }
  for (const [w, wn] of WIDTHS) {
    const p = g.dayPeriods.format[wn]
    set(`a.${w}`, [p.am, p.pm])
    set(`G.${w}`, [g.eras[eraKey[w]]['0'], g.eras[eraKey[w]]['1']])
    // flexible day periods (`B`): the period of each hour by the language's rules, then `noon`
    const lang = l.split('-')
    const key = [lang.join('_'), lang.slice(0, 2).join('_'), lang[0]].find((k) => periodRules[k])
    const rules = key ? Object.entries(periodRules[key]) : []
    const hours = Array.from({ length: 24 }, (_, h) => {
      for (const [name, r] of rules) {
        if (!r._from || !p[name]) continue
        const from = Number(r._from.slice(0, 2))
        const before = Number(r._before.slice(0, 2))
        if (from < before ? h >= from && h < before : h >= from || h < before) return p[name]
      }
      return h < 12 ? p.am : p.pm
    })
    const noon = rules.some(([name]) => name === 'noon') && p.noon ? p.noon : ''
    set(`B.${w}`, [...hours, noon])
  }
  return { n, g }
}
const pad = (v, w) => String(v).padStart(w, '0')
const WID = { 3: 'a', 4: 'w', 5: 'n', 6: 's' }
// ISO week-numbering year (`Y`, only in the data of locales whose weeks are ISO ones)
const isoWeekYear = (c) => {
  const x = new Date(0)
  x.setUTCFullYear(c.y, c.mo - 1, c.d)
  const iso = (c.wd + 6) % 7 // Monday 0
  x.setUTCDate(x.getUTCDate() - iso + 3) // the Thursday of the week
  return x.getUTCFullYear()
}
function dateField(f, c, n, digits, ctx) {
  const ch = f[0]
  const len = f.length
  let num
  switch (ch) {
    case 'y': {
      // the year of the era: 2 BC for -1
      const ey = c.y > 0 ? c.y : 1 - c.y
      num = len === 2 ? pad(ey % 100, 2) : pad(ey, len)
      break
    }
    case 'Y': {
      // the ISO week year, signed (ICU's extended year)
      const y = isoWeekYear(c)
      num = pad(len === 2 ? Math.abs(y) % 100 : Math.abs(y), len)
      if (y < 0) return n.get('minus') + (digits ? num.replace(/[0-9]/g, (x) => [...digits][Number(x)]) : num)
      break
    }
    case 'M':
    case 'L':
      if (len > 2) return n.get(`M.${ch === 'M' ? 'f' : 's'}${WID[len]}`)[c.mo - 1]
      num = pad(c.mo, len)
      break
    case 'r':
      // not CLDR's: a numeric month in lowercase Roman numerals (`haw`, `M=romanlow`)
      return ['i', 'ii', 'iii', 'iv', 'v', 'vi', 'vii', 'viii', 'ix', 'x', 'xi', 'xii'][c.mo - 1]
    case 'd':
      num = pad(c.d, len)
      break
    case 'E':
    case 'c':
      return n.get(`E.${ch === 'E' ? 'f' : 's'}${WID[Math.max(len, 3)]}`)[c.wd]
    case 'h':
      num = pad(c.h % 12 || 12, len)
      break
    case 'H':
      num = pad(c.h, len)
      break
    case 'K':
      num = pad(c.h % 12, len)
      break
    case 'k':
      num = pad(c.h || 24, len)
      break
    case 'm':
      num = pad(c.mi, len)
      break
    case 's':
      num = pad(c.s, len)
      break
    case 'a':
      return n.get(`a.${WID[Math.max(len, 3)]}`)[c.h < 12 ? 0 : 1]
    case 'B': {
      const t = n.get(`B.${WID[Math.max(len, 3)]}`)
      if (c.h === 12 && t[24] && (!ctx.m || c.mi === 0) && (!ctx.s || c.s === 0)) return t[24]
      return t[c.h]
    }
    case 'G':
      return n.get(`G.${len <= 3 ? 'a' : WID[len]}`)[c.y > 0 ? 1 : 0]
    default:
      throw new Error(`symbol ${f}`)
  }
  return digits ? num.replace(/[0-9]/g, (x) => [...digits][Number(x)]) : num
}
// tokens: { lit } | { f } | { glue: 0 | 1 }
function renderDate(tokens, c, n, digits, parts = {}) {
  const ctx = { m: tokens.some((t) => t.f?.[0] === 'm'), s: tokens.some((t) => t.f?.[0] === 's') }
  let out = ''
  for (const t of tokens) {
    if (t.lit !== undefined) out += t.lit
    else if (t.glue !== undefined) out += renderDate(t.glue === 0 ? parts.time : parts.date, c, n, digits)
    else out += dateField(t.f, c, n, digits, ctx)
  }
  return out
}
const civilOf = (ms) => {
  const x = new Date(ms)
  return { y: x.getUTCFullYear(), mo: x.getUTCMonth() + 1, d: x.getUTCDate(), wd: x.getUTCDay(), h: x.getUTCHours(), mi: x.getUTCMinutes(), s: x.getUTCSeconds() }
}
const utcAt = (y, mo, d, h, mi, s) => {
  const x = new Date(0)
  x.setUTCFullYear(y, mo - 1, d)
  x.setUTCHours(h, mi, s, 0)
  return x.getTime()
}
// every month and weekday, every hour, minutes and seconds 0 and 4, years 5, -1 (2 BC), the
// proleptic 1500 and the last one of JS
const DATE_PROBES = []
for (let k = 0; k < 24; k++) DATE_PROBES.push(utcAt(2024, ((k * 5) % 12) + 1, k < 12 ? 5 : 8, (k * 7) % 24, k % 2 ? 4 : 0, k % 3 ? 4 : 0))
DATE_PROBES.push(utcAt(5, 3, 5, 12, 0, 0), utcAt(-1, 11, 25, 0, 0, 0), utcAt(1500, 7, 9, 23, 59, 59), utcAt(275760, 9, 12, 12, 30, 0), utcAt(2024, 12, 30, 12, 0, 4))
const DATE_CIVIL = DATE_PROBES.map(civilOf)
const CAND = {
  year: ['y', 'yy', 'yyyy', 'Y', 'YY'],
  month: ['M', 'MM', 'MMM', 'MMMM', 'MMMMM', 'LLL', 'LLLL', 'LLLLL', 'r'],
  day: ['d', 'dd'],
  weekday: ['EEE', 'EEEE', 'EEEEE', 'EEEEEE', 'ccc', 'cccc', 'ccccc', 'cccccc'],
  hour: ['h', 'hh', 'H', 'HH', 'K', 'KK', 'k', 'kk'],
  minute: ['m', 'mm'],
  second: ['s', 'ss'],
  dayPeriod: ['a', 'aaaa', 'aaaaa', 'B', 'BBBB', 'BBBBB'],
  era: ['G', 'GGGG', 'GGGGG'],
}
const checkRender = (tokens, f, n, digits) => DATE_PROBES.every((p, k) => sp(f.format(p)) === renderDate(tokens, DATE_CIVIL[k], n, digits))
// The pattern of a formatter, from its parts on the probes.
function deriveParts(f, n, digits) {
  const parts = DATE_PROBES.map((p) => f.formatToParts(p))
  const sig = (ps) => ps.map((p) => p.type + (p.type === 'literal' ? `:${sp(p.value)}` : '')).join('|')
  const first = sig(parts[0])
  for (const ps of parts) if (sig(ps) !== first) throw new Error(`structure ${first} vs ${sig(ps)}`)
  const tokens = parts[0].map((p, i) => {
    if (p.type === 'literal') return { lit: sp(p.value) }
    const f = (CAND[p.type] ?? []).find((s) => parts.every((ps, k) => sp(ps[i].value) === renderDate([{ f: s }], DATE_CIVIL[k], n, digits)))
    if (!f) throw new Error(`no symbol for ${p.type} ${parts.slice(0, 4).map((ps) => ps[i].value)}`)
    return { f }
  })
  return tokens
}
// V8 cannot `formatToParts` a pattern with `Y` (it aborts): such a pattern is found from
// `format()` alone, as the one parse of a probe into the asked fields and literals that
// renders every probe.
function deriveText(f, n, digits, types) {
  const outs = DATE_PROBES.map((p) => sp(f.format(p)))
  const s = outs[0]
  const search = (i, left, acc) => {
    if (i === s.length) {
      if (left.length > 0) return null
      const tokens = []
      for (const t of acc) {
        if (t.lit !== undefined && tokens.at(-1)?.lit !== undefined) tokens.at(-1).lit += t.lit
        else tokens.push({ ...t })
      }
      return DATE_PROBES.every((p, k) => outs[k] === renderDate(tokens, DATE_CIVIL[k], n, digits)) ? tokens : null
    }
    for (const type of left) {
      for (const sym of CAND[type]) {
        const v = renderDate([{ f: sym }], DATE_CIVIL[0], n, digits)
        if (v && s.startsWith(v, i)) {
          const r = search(i + v.length, left.filter((x) => x !== type), [...acc, { f: sym }])
          if (r) return r
        }
      }
    }
    return /[0-9]/.test(s[i]) ? null : search(i + 1, left, [...acc, { lit: s[i] }])
  }
  const r = search(0, types, [])
  if (!r) throw new Error(`no parse of ${JSON.stringify(s)}`)
  return r
}
const toAscii = (s, digits) => (digits ? s.replace(/./gu, (ch) => String([...digits].indexOf(ch) >= 0 ? [...digits].indexOf(ch) : ch)) : s)
function derivePattern(f, n, digits, o, weekYears) {
  let tokens
  if (weekYears && o.year) {
    // 2024-12-30 is in the first ISO week of 2025
    const shown = toAscii(sp(f.format(utcAt(2024, 12, 30, 0, 0, 0))), digits)
    if (shown.includes(o.year === '2-digit' ? '25' : '2025')) tokens = deriveText(f, n, digits, ['year', 'month', 'day', 'weekday'].filter((k) => o[k]))
  }
  tokens ??= deriveParts(f, n, digits)
  if (!checkRender(tokens, f, n, digits)) throw new Error(`render differs: ${JSON.stringify(tokens)}`)
  return tokens
}
// CLDR pattern syntax: letters and quotes quoted
const patText = (tokens) =>
  tokens
    .map((t) => {
      if (t.f) return t.f
      if (t.glue !== undefined) return `{${t.glue}}`
      return /[A-Za-z'{}]/.test(t.lit) ? `'${t.lit.replace(/'/g, "''")}'` : t.lit
    })
    .join('')
// the reverse, for glue patterns from CLDR
function parsePattern(p) {
  const tokens = []
  const lit = (s) => {
    if (tokens.at(-1)?.lit !== undefined) tokens.at(-1).lit += s
    else tokens.push({ lit: s })
  }
  for (let i = 0; i < p.length; ) {
    const ch = p[i]
    if (ch === "'") {
      if (p[i + 1] === "'") {
        lit("'")
        i += 2
        continue
      }
      const end = p.indexOf("'", i + 1)
      let text = p.slice(i + 1, end)
      let j = end
      while (p[j + 1] === "'") {
        const next = p.indexOf("'", j + 2)
        text += `'${p.slice(j + 2, next)}`
        j = next
      }
      lit(text)
      i = j + 1
    } else if (ch === '{' && /^\{[01]\}/.test(p.slice(i))) {
      tokens.push({ glue: Number(p[i + 1]) })
      i += 3
    } else if (/[A-Za-z]/.test(ch)) {
      let j = i
      while (p[j] === ch) j++
      tokens.push({ f: ch.repeat(j - i) })
      i = j
    } else {
      lit(sp(ch))
      i++
    }
  }
  return tokens
}
const YEARS = [undefined, 'numeric', '2-digit']
const MONTHS = [undefined, 'numeric', '2-digit', 'long', 'short', 'narrow']
const DAY_OPT = [undefined, 'numeric', '2-digit']
const WEEKDAYS = [undefined, 'long', 'short', 'narrow']
const CLOCK = [undefined, 'numeric', '2-digit']
// combination indices, as format/date.rs computes them
const dateCombo = (i) => ({ year: YEARS[Math.floor(i / 72)], month: MONTHS[Math.floor(i / 12) % 6], day: DAY_OPT[Math.floor(i / 4) % 3], weekday: WEEKDAYS[i % 4] })
const timeCombo = (i) => ({ hour: CLOCK[Math.floor(i / 9)], minute: CLOCK[Math.floor(i / 3) % 3], second: CLOCK[i % 3] })
const defined = (o) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined))
// the glue of a date part (ICU DateTimePatternGenerator): long month with a weekday → full,
// long → long, short → medium, else short
const glueWidth = (i) => {
  const m = Math.floor(i / 12) % 6
  if (m === 3) return i % 4 ? 0 : 1
  return m === 4 ? 2 : 3
}
const HC_CHAR = { h11: 'K', h12: 'h', h23: 'H', h24: 'k' }
// forced hour cycles: each has its own patterns (ICU's generator treats `k` and `K` unlike
// `H` and `h` in odd combinations), `hour12` is the locale's 12- or 24-hour cycle
const CYCLES = ['h11', 'h12', 'h23', 'h24']
const renderGlued = (g, c, n, digits) => renderDate(g.glue, c, n, digits, { date: g.date, time: g.time })
const checkGlued = (g, f, n, digits) => DATE_PROBES.every((p, k) => sp(f.format(p)) === renderGlued(g, DATE_CIVIL[k], n, digits))
// a few locales are checked on every date × time × mode combination
const EXHAUSTIVE = new Set(['en', 'de', 'ja', 'ar', 'zh-Hant', 'ru', 'fr', 'hi', 'he', 'ko'])
const UNITS_REL = ['year', 'month', 'day', 'hour', 'minute', 'second']
const REL_STYLES = ['long', 'short', 'narrow']
const REL_VALUES = [-0, 0, 1, -1, 2, -2, 3, -3, 5, -5, 11, -11, 21, -21, 22, 100, -101, 1000, -1234567, 1e6]
let overrideCount = 0
let derivedCount = 0
let textDerived = 0
function datesOf(l) {
  const tag = dateTag(l)
  const { n, g } = dateNames(l)
  const digits = fields.get(l).f.get('digits')
  // the negative prefix of the locale's numbers (a week year before 1)
  n.set('minus', new Intl.NumberFormat(tag).formatToParts(-1).filter((x) => x.type !== 'integer').map((x) => x.value).join(''))
  const region = fullParts(l).region ?? fullParts(likely[l.split('-')[0]] ?? '').region
  // `Y` in a pattern Intl can use (week-of-year formats aside)
  const avail = Object.entries(g.dateTimeFormats.availableFormats).filter(([k]) => !/[wW]/.test(k))
  const weekYears = [...Object.values(g.dateFormats), ...avail.map(([, v]) => v)].some((v) => typeof v === 'string' && /Y/.test(v.replace(/'[^']*'/g, '')))
  if (weekYears && (weekData.firstDay[region] ?? weekData.firstDay['001']) !== 'mon') throw new Error(`${l}: week years outside ISO weeks`)
  if (weekYears && (weekData.minDays[region] ?? weekData.minDays['001']) !== '4') throw new Error(`${l}: week years outside ISO weeks`)
  const fmt = (o) => new Intl.DateTimeFormat(tag, { timeZone: 'UTC', calendar: 'gregory', ...o })
  const out = new Map()
  const tok = new Map()
  const derive = (key, o) => {
    const t = derivePattern(fmt(o), n, digits, o, weekYears)
    if (t.some((x) => x.lit === undefined && x.f === undefined)) throw new Error('glue in a pattern')
    tok.set(key, t)
    out.set(key, patText(t))
    derivedCount++
    return t
  }
  for (let i = 1; i < 216; i++) derive(`d${i}`, defined(dateCombo(i)))
  for (let i = 1; i < 27; i++) {
    const o = defined(timeCombo(i))
    if (i < 9) derive(`t${i}`, o)
    else {
      derive(`j${i - 9}`, o)
      for (const hc of CYCLES) derive(`${HC_CHAR[hc]}${i - 9}`, { ...o, hourCycle: hc })
    }
  }
  const STYLES = ['full', 'long', 'medium', 'short']
  for (let w = 0; w < 4; w++) derive(`ds${w}`, { dateStyle: STYLES[w] })
  for (const w of [2, 3]) {
    derive(`tsj${w}`, { timeStyle: STYLES[w] })
    for (const hc of CYCLES) derive(`ts${HC_CHAR[hc]}${w}`, { timeStyle: STYLES[w], hourCycle: hc })
  }
  const hc12 = HC_CHAR[fmt({ hour: 'numeric', hour12: true }).resolvedOptions().hourCycle]
  const hc24 = HC_CHAR[fmt({ hour: 'numeric', hour12: false }).resolvedOptions().hourCycle]
  out.set('hc', `${hc12}${hc24}`)
  // `hour12` is the locale's 12- or 24-hour cycle
  for (const [h12, hc] of [[true, hc12], [false, hc24]]) {
    for (let i = 0; i < 18; i++) {
      if (!checkRender(tok.get(`${hc}${i}`), fmt({ ...defined(timeCombo(i + 9)), hour12: h12 }), n, digits)) throw new Error(`${l}: time ${i} with hour12 ${h12}`)
    }
    for (const w of [2, 3]) if (!checkRender(tok.get(`ts${hc}${w}`), fmt({ timeStyle: STYLES[w], hour12: h12 }), n, digits)) throw new Error(`${l}: timeStyle ${w} with hour12 ${h12}`)
  }
  // the glue: what ICU puts around the date and time parts (its data differs from cldr-json in
  // places: `fr-ML`), read off the first probe
  const pickGlue = (date, time, f) => {
    const s = sp(f.format(DATE_PROBES[0]))
    const ds = renderDate(date, DATE_CIVIL[0], n, digits)
    const ts = renderDate(time, DATE_CIVIL[0], n, digits)
    const cands = []
    for (const [a, b, ga, gb] of [[ds, ts, 1, 0], [ts, ds, 0, 1]]) {
      const i = s.indexOf(a)
      const j = i < 0 ? -1 : s.indexOf(b, i + a.length)
      if (j < 0) continue
      cands.push([{ lit: s.slice(0, i) }, { glue: ga }, { lit: s.slice(i + a.length, j) }, { glue: gb }, { lit: s.slice(j + b.length) }].filter((t) => t.lit !== ''))
    }
    return cands.find((glue) => checkGlued({ glue, date, time }, f, n, digits)) ?? null
  }
  const glue = []
  const glueS = []
  for (let w = 0; w < 4; w++) {
    // a date part of this width: year, day and the month (weekday for full)
    const d = 72 + [3 * 12 + 4 + 1, 3 * 12 + 4, 4 * 12 + 4, 1 * 12 + 4][w]
    const time = tok.get('j3') // hour + minute, numeric
    glue[w] = pickGlue(tok.get(`d${d}`), time, fmt({ ...defined(dateCombo(d)), hour: 'numeric', minute: 'numeric' }))
    if (!glue[w]) throw new Error(`${l}: no glue for width ${w}`)
    out.set(`g${w}`, patText(glue[w]))
    glueS[w] = pickGlue(tok.get(`ds${w}`), tok.get('tsj3'), fmt({ dateStyle: STYLES[w], timeStyle: 'short' }))
    if (!glueS[w]) throw new Error(`${l}: no style glue for width ${w}`)
    out.set(`gs${w}`, patText(glueS[w]))
  }
  // date × time: the glue of the date part's width, else an override
  const modes = ['j', ...CYCLES.map((hc) => HC_CHAR[hc])]
  const modeOption = (m) => (m === 'j' ? {} : { hourCycle: CYCLES.find((hc) => HC_CHAR[hc] === m) })
  const timeTokens = (t, m) => (t < 9 ? tok.get(`t${t}`) : tok.get(`${m}${t - 9}`))
  const combined = (d, t, m) => {
    const o = { ...defined(dateCombo(d)), ...defined(timeCombo(t)), ...(t < 9 ? {} : modeOption(m)) }
    const f = fmt(o)
    if (checkGlued({ glue: glue[glueWidth(d)], date: tok.get(`d${d}`), time: timeTokens(t, m) }, f, n, digits)) return
    out.set(`x${d}.${t}.${m}`, patText(derivePattern(f, n, digits, o, weekYears)))
    overrideCount++
  }
  const exhaustive = EXHAUSTIVE.has(l)
  for (let d = 1; d < 216; d++) {
    for (let t = 1; t < 27; t++) {
      for (const m of t < 9 ? ['j'] : modes) {
        if (d < 4 || exhaustive || (t === 13 && m === 'j') || (t === 22 && m === 'H') || t === 4) combined(d, t, m)
      }
    }
  }
  // dateStyle × timeStyle in every mode
  for (let w = 0; w < 4; w++) {
    for (const tw of [2, 3]) {
      for (const m of modes) {
        const f = fmt({ dateStyle: STYLES[w], timeStyle: STYLES[tw], ...modeOption(m) })
        if (checkGlued({ glue: glueS[w], date: tok.get(`ds${w}`), time: tok.get(`ts${m}${tw}`) }, f, n, digits)) continue
        out.set(`y${w}.${tw}.${m}`, patText(deriveParts(f, n, digits)))
        overrideCount++
      }
    }
  }
  // names the patterns use; `B` only where some pattern has it
  const used = [...out.values()].join('')
  const usesB = /B/.test(used.replace(/'[^']*'/g, ''))
  for (const [k, v] of n) if (k !== 'minus' && (!k.startsWith('B.') || usesB)) out.set(k, v.join('\t'))
  // relative time, read back from ICU (its data differs from cldr-json in places: `ccp`): the
  // pattern of each plural form from `formatToParts` of an integer of that form, the words of
  // `numeric: 'auto'` where they differ from the number
  const nf = new Intl.NumberFormat(tag)
  const pr = new Intl.PluralRules(tag)
  const sample = new Map()
  for (const v of [...Array(1101).keys(), 1e6, 1e7, 1e9]) if (!sample.has(pr.select(v))) sample.set(pr.select(v), v)
  const relPattern = (rtf, v, u) =>
    rtf
      .formatToParts(v, u)
      .map((x) => (x.type === 'literal' ? clean(x.value, 'relative') : '\u0000'))
      .join('')
      .replace(/\u0000+/, '{0}')
  for (const u of UNITS_REL) {
    for (const style of REL_STYLES) {
      const always = new Intl.RelativeTimeFormat(tag, { style })
      const auto = new Intl.RelativeTimeFormat(tag, { style, numeric: 'auto' })
      const words = [-2, -1, 0, 1, 2].map((v) => (auto.format(v, u) === always.format(v, u) ? '' : clean(auto.format(v, u), 'relative')))
      const future = CATS.map((c) => (sample.has(c) ? relPattern(always, sample.get(c), u) : ''))
      const past = CATS.map((c) => (sample.has(c) ? relPattern(always, -sample.get(c), u) : ''))
      out.set(`r.${u}.${style}`, [...words, ...future, ...past].join('\t'))
      for (const numeric of ['always', 'auto']) {
        const rtf = new Intl.RelativeTimeFormat(tag, { numeric, style })
        for (const v of REL_VALUES) {
          const word = numeric === 'auto' && Math.abs(v) <= 2 ? words[v + 2] : ''
          const list = v > 0 || Object.is(v, 0) ? future : past
          const pick = (c) => list[CATS.indexOf(c)] || list[5]
          const want = word || pick(pr.select(Math.abs(v))).replace('{0}', nf.format(Math.abs(v)))
          const got = rtf.format(v, u)
          if (got !== want) throw new Error(`${l}: relative ${v} ${u} ${style} ${numeric}: ${JSON.stringify(got)}, predicted ${JSON.stringify(want)}`)
        }
      }
    }
  }
  if (weekYears) textDerived++
  return out
}
const dateStart = Date.now()
const dateFields = new Map(all.map((l) => [l, datesOf(l)]))
const dtxt = []
dtxt.push(`# generated by scripts/cldr.mjs from cldr-dates-full ${CLDR_VERSION} and Node ICU ${process.versions.icu} (patterns) — do not edit`)
dtxt.push('# read by crates/i18n-next/build.rs; a locale section holds the fields that differ from its parent (`key<tab>value`, lists tab-separated)')
let dateDiff = 0
for (const l of all) {
  const own = dateFields.get(l)
  const p = parentOf(l)
  const base = p === null ? null : dateFields.get(p)
  dtxt.push(`locale ${l} ${p ?? '-'}`)
  const before = dtxt.length
  for (const [k, v] of own) if (base?.get(k) !== v) dtxt.push(`${k}\t${v}`)
  if (base) for (const k of base.keys()) if (!own.has(k)) dtxt.push(`${k}\t-`)
  dateDiff += dtxt.length - before
}
dtxt.push('')
writeFileSync(here('../crates/i18n-next/src/format/data/dates.txt'), dtxt.join('\n'))
console.log(`dates.txt: ${all.length} locales, ${derivedCount} patterns derived (${textDerived} locales with week years), ${overrideCount} overrides, ${dateDiff} field lines, ${Date.now() - dateStart} ms`)
