// Case-mapping data of linked-message modifiers (DESIGN 1.1 "Linked", M3).
//
// `@.upper:` / `@.lower:` / `@.capitalize:` call `toLocaleUpperCase(tag)` / `toLocaleLowerCase(tag)`
// (`@i18n-micro/utils` `linked-messages`). V8 runs the default Unicode mapping for every tag except
// those whose language ICU special-cases: `tr`, `az` (dotted / dotless i), `lt` (dot above), `el`
// (Greek upper case drops accents). Rust's `str::to_uppercase` / `to_lowercase` are the default
// mapping; the special cases need what `std` does not expose, so this script writes it from
// Node's own ICU into `crates/i18n-next/src/linked/case_data.rs`:
//
// - `CASE_IGNORABLE`, `CASED`: the Unicode properties ICU's Greek upper case and final sigma read;
// - `ABOVE` (combining class 230), `OTHER_ACCENT` (any other non-zero class), `SOFT_DOTTED`: the
//   dot types of ICU `ucase_getDotType`, read by the `tr` / `lt` context rules;
// - `GREEK_0370`, `GREEK_1F00`: ICU `GreekUpper` letter data (upper-case base letter + flags),
//   derived from the canonical decomposition.
//
// Every table is checked against V8 itself before it is written (the properties by the behaviour
// they cause), so the file is what this Node does, not what a spec says. Run: pnpm -C scripts casing.
// Deterministic: a second run gives a byte-identical file.
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const here = (p) => fileURLToPath(new URL(p, import.meta.url))
const OUT = here('../crates/i18n-next/src/linked/case_data.rs')
const MAX = 0x10ffff
const isSurrogate = (c) => c >= 0xd800 && c <= 0xdfff
const ch = (c) => String.fromCodePoint(c)

/** Sorted code point ranges `[start, end]` where `pred` holds. */
function ranges(pred) {
  const out = []
  let start = -1
  for (let c = 0; c <= MAX + 1; c++) {
    const yes = c <= MAX && !isSurrogate(c) && pred(c)
    if (yes && start < 0) start = c
    if (!yes && start >= 0) {
      out.push([start, c - 1])
      start = -1
    }
  }
  return out
}

const caseIgnorable = (c) => /\p{Case_Ignorable}/u.test(ch(c))
const cased = (c) => /\p{Cased}/u.test(ch(c))
const softDotted = (c) => /\p{Soft_Dotted}/u.test(ch(c))
// `lt` lower case puts a dot above on `I` when "more above" follows: a class-230 mark after only
// marks of other non-zero classes. So `I` + c tells class 230; `I` + c + U+0301 tells "other".
const above = (c) => ('I' + ch(c)).toLocaleLowerCase('lt').startsWith('i̇')
const otherAccent = (c) => !above(c) && ('I' + ch(c) + '́').toLocaleLowerCase('lt').startsWith('i̇')

// ---- check the properties against what ICU does with them ----
// Greek `ή` keeps its tonos when it stands alone: nothing cased before it, and after it, past any
// case-ignorable characters, nothing cased. So `ή` + c + `1` keeps it unless c is cased (not ignorable);
// `ή` + c + `α` keeps it only when c is neither cased nor ignorable.
for (let c = 0; c <= MAX; c++) {
  if (isSurrogate(c)) continue
  const keepsDigit = ('ή' + ch(c) + '1').toLocaleUpperCase('el').startsWith('Ή')
  const keepsAlpha = ('ή' + ch(c) + 'α').toLocaleUpperCase('el').startsWith('Ή')
  const ign = caseIgnorable(c)
  const cas = cased(c) && !ign
  // a Greek diacritic right after the letter is part of it (`getDiacriticData`), not context
  if ([0x300, 0x301, 0x302, 0x303, 0x304, 0x306, 0x308, 0x311, 0x313, 0x314, 0x342, 0x343, 0x344, 0x345].includes(c)) continue
  if (keepsDigit !== !cas || keepsAlpha !== (!cas && !ign)) {
    throw new Error(`U+${c.toString(16)}: ICU case type differs from \\p{Case_Ignorable} / \\p{Cased}`)
  }
}
// Soft_Dotted as `lt` upper case reads it: a U+0307 after a soft-dotted letter is dropped.
for (let c = 0; c <= MAX; c++) {
  if (isSurrogate(c) || above(c) || otherAccent(c)) continue
  const drops = (ch(c) + '̇').toLocaleUpperCase('lt') === ch(c).toLocaleUpperCase('lt')
  if (drops !== softDotted(c)) throw new Error(`U+${c.toString(16)}: Soft_Dotted differs from lt upper case`)
}

// ---- Greek letter data (ICU GreekUpper getLetterData) ----
const VOWEL = 0x400
const YPOGEGRAMMENI = 0x800
const ACCENT = 0x1000
const DIALYTIKA = 0x2000
const OTHER_DIACRITIC = 0x4000
const VOWELS = new Set([0x391, 0x395, 0x397, 0x399, 0x39f, 0x3a5, 0x3a9])
function greek(c) {
  const d = [...ch(c).normalize('NFD')].map((x) => x.codePointAt(0))
  const base = d[0]
  if (!/\p{Script=Greek}/u.test(ch(base)) || !/\p{L}/u.test(ch(base)) || !cased(base)) return 0
  const up = [...ch(base).toUpperCase()]
  if (up.length !== 1) return 0
  const upper = up[0].codePointAt(0)
  if (upper > 0x3ff) return 0
  let data = upper
  if (VOWELS.has(upper)) data |= VOWEL
  for (const m of d.slice(1)) {
    if (m === 0x300 || m === 0x301 || m === 0x342) data |= ACCENT
    else if (m === 0x308) data |= DIALYTIKA
    else if (m === 0x345) data |= YPOGEGRAMMENI
    else if ([0x304, 0x306, 0x313, 0x314, 0x343].includes(m)) data |= OTHER_DIACRITIC
    else throw new Error(`U+${c.toString(16)}: unexpected mark U+${m.toString(16)}`)
  }
  return data
}
const table = (from, len) => Array.from({ length: len }, (_, i) => greek(from + i))
if (greek(0x2126) !== (0x3a9 | VOWEL)) throw new Error('ohm sign')

// ---- write ----
/** A range packed into one u32: start << 11 | (length - 1), lengths split at 2048. */
function pack(list) {
  const out = []
  for (const [a, b] of list) {
    for (let s = a; s <= b; s += 2048) out.push((s * 2048 + Math.min(b - s, 2047)) >>> 0)
  }
  return out
}
const hex = (n, w) => `0x${n.toString(16).padStart(w, '0')}`
function rows(values, w, perLine) {
  const lines = []
  for (let i = 0; i < values.length; i += perLine) lines.push(`    ${values.slice(i, i + perLine).map((v) => `${hex(v, w)},`).join(' ')}`)
  return lines.join('\n')
}
function u32s(name, doc, list) {
  const packed = pack(list)
  return `/// ${doc}\npub(super) const ${name}: [u32; ${packed.length}] = [\n${rows(packed, 8, 6)}\n];\n`
}
const g0370 = table(0x370, 0x90)
const g1f00 = table(0x1f00, 0x100)
const text = `//! Generated by scripts/casing.mjs (pnpm -C scripts casing) from Node ${process.versions.node.split('.')[0]}, ICU ${process.versions.icu}, Unicode ${process.versions.unicode}: do not edit.
//! Ranges are packed \`start << 11 | (length - 1)\`, sorted by start.

${u32s('CASE_IGNORABLE', 'Unicode `Case_Ignorable`.', ranges(caseIgnorable))}
${u32s('CASED', 'Unicode `Cased`.', ranges(cased))}
${u32s('SOFT_DOTTED', 'Unicode `Soft_Dotted`.', ranges(softDotted))}
${u32s('ABOVE', 'Canonical combining class 230.', ranges(above))}
${u32s('OTHER_ACCENT', 'Any other non-zero canonical combining class.', ranges(otherAccent))}
/// ICU \`GreekUpper\` letter data of U+0370..U+03FF: upper-case base letter | flags (\`linked/case.rs\`).
pub(super) const GREEK_0370: [u16; 0x90] = [
${rows(g0370, 4, 8)}
];

/// The same for U+1F00..U+1FFF.
pub(super) const GREEK_1F00: [u16; 0x100] = [
${rows(g1f00, 4, 8)}
];
`
writeFileSync(OUT, text)
console.log(`${OUT}: ${text.length} bytes`)
