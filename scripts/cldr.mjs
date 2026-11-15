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
import { readFileSync, writeFileSync } from 'node:fs'
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
