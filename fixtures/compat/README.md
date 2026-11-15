# Compat goldens

What the JS library does, recorded so the Rust crate can be checked against it line by line (DESIGN.md 7.2).

- Oracle: the published `@i18n-micro/core` and `@i18n-micro/utils` at the versions pinned in `scripts/package.json`
  (installed from npm, not the local source), on Node 22.
- Generator: `scripts/compat.mjs`. Regenerate with:

  ```sh
  pnpm -C scripts install
  pnpm -C scripts compat
  ```

- Only JS writes these files. The Rust test (`compat.rs`) runs the same calls, prints the same lines and compares;
  it never rewrites them. CI regenerates them and fails on any diff (`git diff --exit-code fixtures/compat`).
- Output is deterministic: no timestamps, lines sorted, a second run gives byte-identical files.

## Layout

| Path | What |
| --- | --- |
| `locales/` | the catalog fed to `loadTranslationDir` (root files, `pages/`, plus broken and out-of-layout files on purpose) |
| `__golden__/load.md` | what the loader produced: each file's fate and every leaf of every `(locale, page)` chunk |
| `__golden__/lookup.md` | `t` (raw lookup) and `has` over keys × locale configs × pages |
| `__golden__/interpolate.md` | `interpolate(template, params)` and `t(key, params[, default])` through the instance |
| `__golden__/options.md` | `disablePageLocales` loading, `routesLocaleLinks` page aliases, per-locale fallback chains |

## File format

~~~
# compat: <name>                      ← title

> oracle: @i18n-micro/core 2.1.0, …   ← header lines, start with "> "
> node 22, icu 78.3, cldr 48.0, …

```text                               ← fence
<call> → <result>                     ← one line per case
```                                   ← fence
~~~

- Rust compares **call lines only**: every line that is not empty and does not start with `#`, `>` or a code fence
  (three backticks). Header lines are informational; a header change (new Node ICU, a version bump) still shows up as
  a diff in CI, on purpose.
- The `node` header is the major version only (`process.versions.node`), because CI uses `node-version: 22` and the
  patch floats. `icu` / `cldr` / `unicode` are full versions of that Node; nothing in M1 depends on them, later
  stages (plurals, formats) do.
- Separator is ` → ` (space, U+2192, space). No call contains ` → `, so split on the **first** occurrence (a result
  may contain it).
- Lines are unique by call and sorted by the **UTF-8 bytes** of the whole line (Rust: sort `Vec<String>`). Files
  end with a newline, LF only.

### Spelling of values

**Strings** (keys, locales, pages, templates, string results) are `JSON.stringify(s)` with one extra step: these
invisible code units are additionally written as `\uXXXX` with lowercase hex, so editors can neither hide nor alter
them:

```
U+007F–U+00A0  U+00AD  U+1680  U+180E  U+2000–U+200F  U+2028–U+202F
U+205F–U+206F  U+3000  U+FE00–U+FE0F  U+FEFF  U+FFF9–U+FFFB
```

`JSON.stringify` itself escapes `"` and `\` as `\"` `\\`, uses `\b \f \n \r \t`, writes other code units below
U+0020 as `\u00xx` (lowercase), and leaves everything else raw (`/` is not escaped; Cyrillic, emoji stay as is).

**Results** are one of:

| Result | Meaning |
| --- | --- |
| `"…"` | a string (spelled as above) |
| `1.5`, `1e+21`, `0` | a number, spelled as JS `String(n)` (for finite numbers equal to `JSON.stringify`; `-0` → `0`, `1e21` → `1e+21`, `1e-7` → `1e-7`, `123456789012345678901` → `123456789012345680000`) |
| `true` / `false` | a boolean (as a value, or the answer of `has`) |
| `<object>` | a non-array object: a subtree, or an empty `{}` |
| `<array>` | an array (a subtree, or an empty `[]`) |
| `<null>` | a JSON `null` leaf as stored (only in `load.md`) |
| `<missing>` | nothing: no value at the path, or the lookup returned `null` |

**Arguments** (params, default values) are spelled like JSON (`{"name":"Ann"}`, `["Ann","Bob"]`, no spaces, keys in
insertion order), plus the JS values JSON cannot spell: `undefined`, `NaN`, `Infinity`, `-Infinity`, `-0`.

**Context** `"de-at"|"de", "about"` is `<locale>|<fallback locale>, <page>`. The root chunk is page `"index"`
(page `""` is the same chunk). `core` has one fallback locale, not a chain: `de-at` falls back to `de` only, never
to `en`. The Rust test builds the instance with exactly the chain `[locale, fallback]` (no fallback when they are
equal).

## The calls

### `load.md`

- `file("<relative path>") → "<status>"`: every `.json` file `listTranslationFiles` found, relative to `locales/`
  with `/`. Status: `"root <locale>"`, `"page <page> <locale>"`, `"ignore"` (outside the layout, silently dropped),
  `"skip: invalid JSON"` (parse error, the loader logs and drops the file), `"skip: not an object"` (valid JSON
  but not an object: array, `null`, …). Non-`.json` files are never listed.
- `load("<locale>", "<page>", "<path>") → <result>`: for every chunk the loader produced, every raw leaf of the
  parsed tree (a primitive, `null`, an empty `{}` / `[]`), its dotted path (keys joined by `.`, array indices as
  `0`, `1`, …, `__proto__` keys skipped), and the value **`getByPath(chunk, path)` reads at that path**. So:
  - the own flat key wins: `"a.b": "flat"` beside `a: {b: "nested"}` gives one line `"a.b" → "flat"`;
  - a flat key with `null` falls back to the walk: `"x.y": null` beside `x: {y: "walked"}` gives `"walked"`;
  - a nested dotted key cannot be reached: `p: {"q.r": …}` gives `"p.q.r" → <missing>`;
  - the root key `""` cannot be reached (`t("")` reads nothing): `"" → <missing>`; `e: {"": …}` is reached by `"e."`.

  For Rust this is the content of the flat map after loading: every `load` line with a value must be readable at
  that key, `<missing>` lines must not be.

### `lookup.md`

- `t(ctx, "<key>") → <result>`: the raw lookup (`BaseI18n.resolveLookup`), layers in order: page of the locale, root
  of the locale, page of the fallback, root of the fallback; first non-null wins. JS `t()` turns `<missing>` into the
  key (or the default) — that is recorded in `interpolate.md`, not here.
- `has(ctx, "<key>") → true|false`: `BaseI18n.has`. **It checks the active locale only** (its page and root), never
  the fallback: `has("de"|"en", "index", "only_en") → false` while `t` finds it.

### `interpolate.md`

- `interpolate("<template>", <params>) → "<result>"`: the token is `\{([^{}\s]+)\}` with JS `\s` (it includes
  U+00A0, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF, U+1680, but **not** U+200B). Params are
  read as **own** properties only (`{toString}` with `{}` stays text), a missing or `undefined` param leaves
  `{name}` as is, values are `String(value)`, a single pass (a value containing `{b}` is not expanded again).
  Array params are an object too: `{0}`, `{1}` and also `{length}`.
- `t(ctx, "<key>", <params>[, <default>]) → <result>`: `BaseI18n.t`. A string value is interpolated; a number,
  boolean or subtree is returned as is, uninterpolated; on a miss the default is interpolated when there are params,
  else returned raw; with no default the **key** is returned, never interpolated.
- The `\s` set is pinned code point by code point: for each of U+0009–U+000D, U+0020, U+00A0, U+1680,
  U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF (whitespace) and U+180E, U+0085, U+200B (not
  whitespace) there is one `interpolate("{a<cp>b}", …)` and one `interpolate("{<cp>}", …)` line.
- `interpolate("{01} {1}", ["a","b"])`: `{01}` is no array index (`params["01"]` is `undefined` on an array), so it
  stays; with an object `{"1":…,"01":…}` both are own keys and both are replaced.

### `options.md`

Options of `@i18n-micro/utils` and the Nuxt module that core alone does not show.

- `file_nopages("<relative path>") → "<status>"`: `classifyTranslationRelativePath(path, true)`: with
  `disablePageLocales` a page file is `"root <locale>"`.
- `load_nopages("<locale>", "<path>") → <result>`: the root chunks of `loadTranslationDir(dir, true)`, leaf by leaf
  as in `load.md`. Page files are folded into the root bucket by `storeLoadedTranslationFile`: files arrive in
  relative-path order (JS `<`, UTF-16 code units), a folded page file is merged **over** what is there, the root
  file is merged **under** what is there. So a page file wins over the root file in either order, and for a locale
  that sorts after `pages/` (`ru`) the root file's own keys survive under the page files' objects (`fold.y`). Two
  page folders of one locale fold in UTF-16 order: `pages/😀/` (U+D83D…) comes before `pages/｡/` (U+FF61), the
  opposite of UTF-8 byte order, so `u16` is the `｡` file's value. Rust: `.pages(false)`.
- `alias_t("<page>", "<key>") → <result>`: `routesLocaleLinks` `{"about-us": "about", "profile": "user-profile",
  "cfg": "settings"}`. The page a route reads is `resolveTranslationPayloadPage(route, links)`
  (`@i18n-micro/utils/payload-url`, one hop, own keys only), then the usual lookup in context `"de"|"en"`. Rust:
  `.page_alias(from, to)` for each link, `i18n.page("<page>")`.
- `chain_t("<locale>", "<page>", "<key>") → <result>`: a per-locale fallback chain. Core has one fallback locale;
  the chain is the Nuxt module's (`buildFallbackLocaleChain` of `@i18n-micro/utils/merge-source`: the locale, then
  its `fallbackLocale` while that names a configured locale not yet in the chain, then the global fallback as the
  weakest step). Locales: `en`, `de → ru`, `de-at → de`, `ru → de-at`, `fr`; global fallback `en`. That is a cycle:
  `ru` reads `ru, de-at, de, en`, `de` reads `de, ru, de-at, en`; `xx` (not configured) reads `en` only.
  **Method:** the Nuxt module premerges a page from the chain's files; to keep core's layer semantics (`null` and a
  scalar never hide a weaker layer's subtree), the oracle instead walks the chain on core layers: each step is
  `BaseI18n.resolveLookup` with that locale as both locale and fallback (its page, then its root), and the first
  non-null step wins. Rust: `config_json` with the same `locales[]` and `fallbackLocale`, `with_locale(locale)
  .page(page)`. The global fallback has no `fallbackLocale` of its own here: Rust follows links from the global
  fallback too, `buildFallbackLocaleChain` does not, and this golden does not decide that case.

## Contract for Rust

Rust `t()` returns `String`, so the markers map as follows. This mapping is the contract the Rust `compat` test
implements.

| Golden | `try_t` / `get` | `t` | `has` (active locale) |
| --- | --- | --- | --- |
| `"text"` | `Some("text")` | `"text"` | `true` |
| number `n` | `Some(String(n))` — JS spelling, see the table above | same | `true` |
| `true` / `false` | `Some("true")` / `Some("false")` | same | `true` |
| `<object>` / `<array>` | `None` (a subtree is not a message) | the key | **`true`** |
| `<null>` (load) / `<missing>` | `None` | the key (or the default) | `false` |

- `null` is "no value": it never shadows a weaker layer (`ru` `nil_override: null` → `en-value` from the fallback).
- Number spelling is ECMAScript `Number::toString` (shortest round-trip digits; exponent form for `>= 1e21` and
  `< 1e-6`, written `1e+21`, `1e-7`).
- A page chunk holds only its own keys; a subtree in a stronger layer hides nothing else: `nav` in the `about` page
  is `<object>`, yet `nav.about` still comes from root.
- The loader merges in relative-path order (JS `<`, UTF-16 code units): `pages/user-profile/en.json` and
  `pages/user/profile/en.json` both are page `user-profile`; the second is deep-merged over the first (objects
  merge, everything else including arrays and `null` replaces).
- Duplicate keys in one file: the last wins (`JSON.parse`), objects are not merged (`dobj` keeps only `y`).
- A UTF-8 BOM at the start of a file is skipped (`de-at.json`).
