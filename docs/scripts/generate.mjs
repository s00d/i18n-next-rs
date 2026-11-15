// Builds the reference pages from the repository sources before `nuxt dev` / `nuxt generate`.
// Output (gitignored, rewritten on every run):
//   content/3.reference/  JS API + types (guest-js/*.ts), Tauri commands (src/commands.rs), permissions (permissions/)
//   content/9.changelog.md  CHANGELOG.md with frontmatter and site links
// A source that does not exist yet (or is still empty) gives a short empty-state page, never a failure.
// Edit the sources, never the generated pages.
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { parse as parseToml } from 'smol-toml'

const root = fileURLToPath(new URL('../../', import.meta.url))
const content = fileURLToPath(new URL('../content/', import.meta.url))
const refDir = `${content}3.reference/`
const plugin = 'crates/tauri-plugin-i18n-next/'
const repo = 'https://github.com/s00d/i18n-next-rs/blob/main/'

const exists = (rel) => existsSync(root + rel)
const read = (rel) => readFileSync(root + rel, 'utf8')
const page = (title, description, source, body) =>
  `---\ntitle: ${JSON.stringify(title)}\ndescription: ${JSON.stringify(description)}\ngenerated: true\n---\n\n` +
  `<!-- Generated from ${source} by docs/scripts/generate.mjs. Edit the source, not this file. -->\n\n${body.trim()}\n`
// Inline code inside a table cell: one line, `|` escaped for GFM.
const cell = (text) => (text ? '`' + text.replace(/\s+/g, ' ').replace(/\|/g, '\\|') + '`' : '')
const oneLine = (text) => (text || '').replace(/\s*\n\s*/g, ' ').replace(/\|/g, '\\|').trim()
const link = (rel) => `[\`${rel}\`](${repo}${rel})`
function check(cond, message) {
  if (!cond) throw new Error(`generate.mjs: ${message}`)
}

// ---------------------------------------------------------------- JS API (TypeScript compiler API)

function jsdoc(node) {
  const docs = ts.getJSDocCommentsAndTags(node).filter(ts.isJSDoc)
  const doc = docs.at(-1)
  if (!doc) return { text: '', tags: {} }
  const tags = {}
  for (const tag of doc.tags ?? []) tags[tag.tagName.text] = ts.getTextOfJSDocComment(tag.comment) ?? ''
  return { text: ts.getTextOfJSDocComment(doc.comment) ?? '', tags }
}

function docBlock({ text, tags }) {
  const out = []
  if ('deprecated' in tags) out.push(`**Deprecated.** ${tags.deprecated}`.trim())
  if (text) out.push(text)
  return out.join('\n\n')
}

// `name<T>(a: A, b?: B): R` from a function declaration, arrow function or method.
function fnSig(name, fn, sf) {
  const tp = fn.typeParameters ? `<${fn.typeParameters.map((t) => t.getText(sf)).join(', ')}>` : ''
  const params = fn.parameters.map((p) => p.getText(sf)).join(', ')
  return `${name}${tp}(${params})${fn.type ? `: ${fn.type.getText(sf)}` : ''}`.replace(/\s+/g, ' ').replace(/\(\s/g, '(').replace(/,?\s\)/g, ')')
}

const hasModifier = (node, kind) => node.modifiers?.some((m) => m.kind === kind) ?? false
const isFn = (node) => node && (ts.isArrowFunction(node) || ts.isFunctionExpression(node))

// Public surface = every `export` of each guest-js entry point.
const entryPoints = [
  { file: `${plugin}guest-js/index.ts`, specifier: 'tauri-plugin-i18n-next-api' },
  { file: `${plugin}guest-js/vue.ts`, specifier: 'tauri-plugin-i18n-next-api/vue' },
]
const apis = entryPoints.filter((e) => exists(e.file)).map(({ file, specifier }) => {
  const sf = ts.createSourceFile(file, read(file), ts.ScriptTarget.Latest, true)
  const api = { file, specifier, sf, functions: [], events: [], objects: [], interfaces: [], aliases: [] }
  for (const st of sf.statements) {
    if (!hasModifier(st, ts.SyntaxKind.ExportKeyword)) continue
    const doc = jsdoc(st)
    if ('internal' in doc.tags) continue
    if (ts.isFunctionDeclaration(st)) {
      const e = { name: st.name.text, sig: fnSig(st.name.text, st, sf), doc: docBlock(doc) }
      ;(e.name.startsWith('on') ? api.events : api.functions).push(e)
    } else if (ts.isVariableStatement(st)) {
      for (const d of st.declarationList.declarations) {
        const name = d.name.getText(sf)
        if (isFn(d.initializer)) {
          const e = { name, sig: fnSig(name, d.initializer, sf), doc: docBlock(jsdoc(d)) }
          ;(name.startsWith('on') ? api.events : api.functions).push(e)
        } else if (d.initializer && ts.isObjectLiteralExpression(d.initializer)) {
          const members = d.initializer.properties
            .filter((m) => ts.isPropertyAssignment(m) && isFn(m.initializer))
            .map((m) => {
              const full = `${name}.${m.name.getText(sf)}`
              return { name: full, sig: fnSig(full, m.initializer, sf), doc: docBlock(jsdoc(m)) }
            })
          api.objects.push({ name, members })
        }
      }
    } else if (ts.isInterfaceDeclaration(st)) api.interfaces.push(st)
    else if (ts.isTypeAliasDeclaration(st)) api.aliases.push(st)
  }
  return api
})
const callables = (api) => [...api.functions, ...api.events, ...api.objects.flatMap((o) => o.members)]
const undocumented = apis.flatMap(callables).filter((e) => !e.doc).map((e) => e.name)
if (undocumented.length) console.warn(`generate.mjs: no JSDoc: ${undocumented.join(', ')}`)

const anchor = (name) => name.toLowerCase().replace(/\./g, '')
const entries = (list) =>
  list.map((e) => `### \`${e.name}()\`\n\n\`\`\`ts\n${e.sig}\n\`\`\`\n${e.doc ? `\n${e.doc}\n` : ''}`).join('\n')
const summary = (list) =>
  '| Function | Description |\n|----------|-------------|\n' +
  list.map((e) => `| [${cell(e.name + '()')}](#${anchor(e.name)}) | ${oneLine(e.doc.split('\n\n')[0])} |`).join('\n')

function jsApiPage() {
  const sections = apis.filter((api) => callables(api).length).flatMap((api) => [
    `## \`${api.specifier}\``,
    '',
    `From ${link(api.file)}.`,
    '',
    summary(callables(api)),
    '',
    entries(api.functions),
    ...(api.events.length ? ['**Event listeners**', '', entries(api.events)] : []),
    ...api.objects.flatMap((o) => [`**\`${o.name}\`**`, '', entries(o.members)]),
  ])
  const body = sections.length
    ? ['Everything `tauri-plugin-i18n-next-api` exports, generated from the guest-js sources.','Option and result types are on the [types page](/reference/types).', '', ...sections].join('\n')
    : `Nothing exported yet: the guest-js sources (${link(`${plugin}guest-js`)}) are not there. The API lands in M8.`
  return page('JavaScript API', 'Every function tauri-plugin-i18n-next-api exports, generated from guest-js.', 'guest-js/*.ts', body)
}

function typesPage() {
  const blocks = apis.flatMap(({ sf, interfaces, aliases }) => {
    const head = (name, st) => {
      const doc = docBlock(jsdoc(st))
      return `### \`${name}\`\n\n${doc ? `${doc}\n\n` : ''}`
    }
    const ifaces = interfaces.map((st) => {
      const ext = st.heritageClauses?.map((h) => h.getText(sf)).join(' ')
      const rows = st.members.map((m) => {
        const d = docBlock(jsdoc(m))
        // `readTimeout?: number // milliseconds`: a trailing line comment is the description too.
        const trailing = sf.text.slice(m.end, sf.text.indexOf('\n', m.end)).match(/\/\/\s*(.*)$/)?.[1] ?? ''
        const desc = oneLine([d, trailing].filter(Boolean).join(' '))
        if (ts.isIndexSignatureDeclaration(m)) return `| ${cell('[' + m.parameters[0].getText(sf) + ']')} | ${cell(m.type.getText(sf))} | ${desc} |`
        const opt = m.questionToken ? '?' : ''
        return `| ${cell(m.name.getText(sf) + opt)} | ${cell(m.type?.getText(sf) ?? 'unknown')} | ${desc} |`
      })
      const name = st.name.text + (st.typeParameters ? `<${st.typeParameters.map((t) => t.getText(sf)).join(', ')}>` : '')
      return `${head(name, st)}${ext ? `Extends ${cell(ext.replace(/^extends\s+/, ''))}.\n\n` : ''}| Property | Type | Description |\n|----------|------|-------------|\n${rows.join('\n')}\n`
    })
    const als = aliases.map((st) => `${head(st.name.text, st)}\`\`\`ts\n${st.getText(sf).replace(/^export\s+/, '')}\n\`\`\`\n`)
    return [...ifaces, ...als]
  })
  const body = blocks.length
    ? ['Every interface and type alias the package exports, generated from the guest-js sources.', '', blocks.join('\n')].join('\n')
    : 'No exported types yet. Option, payload and event types arrive with the full API in M8.'
  return page('Types', 'Types of tauri-plugin-i18n-next-api, generated from guest-js.', 'guest-js/*.ts', body)
}

// ---------------------------------------------------------------- Permissions (permissions/**)

function tomlFiles(dir) {
  if (!exists(dir)) return []
  return readdirSync(root + dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? tomlFiles(`${dir}/${e.name}`) : e.name.endsWith('.toml') ? [`${dir}/${e.name}`] : [],
  )
}

const permissions = [] // { identifier, description }
for (const file of tomlFiles(`${plugin}permissions/autogenerated`).sort()) {
  for (const p of parseToml(read(file)).permission ?? []) permissions.push({ identifier: p.identifier, description: p.description ?? '' })
}
const defaultToml = `${plugin}permissions/default.toml`
const defaults = exists(defaultToml) ? (parseToml(read(defaultToml)).default ?? null) : null
const permissionIds = new Set(permissions.map((p) => p.identifier))

function permissionsPage() {
  if (!permissions.length && !defaults) {
    return page('Permissions', 'Every permission identifier of the plugin, generated from permissions/.', 'permissions/',
      `No permissions yet: ${link(`${plugin}permissions`)} has no \`default.toml\` and Tauri has not generated \`autogenerated/\`. The default set (\`load\`, \`load_page\`, \`get_locale\`, \`available_locales\`, \`pages\`, \`set_locale\`) lands in M7.`)
  }
  const inDefault = new Set(defaults?.permissions ?? [])
  const body = [
    `Generated from ${link(defaultToml)} and \`permissions/autogenerated/\`, which Tauri writes from the command list on build.`,
    '',
    ...(defaults
      ? ['## Default permission set', '', 'Identifier: `i18n-next:default`', '', (defaults.description ?? '').trim(), '',
        (defaults.permissions ?? []).map((p) => `\`${p}\``).join(', ') || 'Empty.', '']
      : []),
    '## All permissions',
    '',
    ...(permissions.length
      ? ['| Identifier | Description | In default |', '|------------|-------------|------------|',
        ...permissions.map((p) => `| ${cell(`i18n-next:${p.identifier}`)} | ${oneLine(p.description)} | ${inDefault.has(p.identifier) ? '✓' : ''} |`)]
      : ['Not generated yet: build the plugin once (`cargo build -p tauri-plugin-i18n-next`) to create `permissions/autogenerated/`.']),
  ].join('\n')
  return page('Permissions', 'Every permission identifier of the plugin, generated from permissions/.', 'permissions/', body)
}

// ---------------------------------------------------------------- Tauri commands (src/commands.rs)

// Splits `a: T<X, Y>, b: U` on top-level commas.
function splitTopLevel(text) {
  const parts = []
  let depth = 0
  let cur = ''
  for (const ch of text) {
    if ('(<['.includes(ch)) depth++
    if (')>]'.includes(ch)) depth--
    if (ch === ',' && depth === 0) {
      parts.push(cur)
      cur = ''
    } else cur += ch
  }
  if (cur.trim()) parts.push(cur)
  return parts.map((p) => p.trim()).filter(Boolean)
}

const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase())

// Rust doc -> page text: `# Parameters` / `# Returns` only repeat the table below, so they go;
// other `# Section` headings become bold lines (a markdown `#` would be a page title).
function rustDoc(lines) {
  const out = []
  let skip = false
  for (const line of lines) {
    const h = line.match(/^#+\s+(.*)$/)
    if (h) {
      skip = /^(Parameters|Returns)$/.test(h[1].trim())
      if (!skip) out.push(`**${h[1].trim()}**`, '')
      continue
    }
    if (!skip) out.push(line)
  }
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

function parseCommands(src) {
  const lines = src.split('\n')
  const commands = []
  let doc = []
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim()
    if (line.startsWith('///')) {
      doc.push(line.replace(/^\/\/\/ ?/, ''))
      continue
    }
    if (!/^#\[(tauri::)?command(\(.*\))?\]$/.test(line)) {
      if (!line.startsWith('#[')) doc = []
      continue
    }
    // Skip further attributes, then read the signature up to the body's `{`.
    let j = i + 1
    while (lines[j].trim().startsWith('#[')) j++
    let sig = ''
    while (!sig.includes('{') && j < lines.length) sig += lines[j++] + '\n'
    sig = sig.slice(0, sig.indexOf('{'))
    const m = sig.match(/pub(?:\([^)]*\))?\s+(async\s+)?fn\s+(\w+)\s*(<[^(]*>)?\s*\(/)
    check(m, `cannot parse command signature near line ${i + 1} of src/commands.rs`)
    const open = sig.indexOf('(', m.index + m[0].length - 1)
    let depth = 0
    let close = open
    for (; close < sig.length; close++) {
      if (sig[close] === '(') depth++
      if (sig[close] === ')' && --depth === 0) break
    }
    const params = splitTopLevel(sig.slice(open + 1, close))
      .map((p) => {
        const [name, ...type] = p.split(':')
        return { name: name.trim(), type: type.join(':').trim() }
      })
      // Injected by Tauri, not passed from JS.
      .filter((p) => !/^(AppHandle|State|Window|WebviewWindow|Webview)\b/.test(p.type))
    const ret = sig.slice(close + 1).replace(/^\s*->\s*/, '').trim()
    // `Result<T>`, `crate::Result<T>`, `Result<T, Error>` -> `T`.
    const ok = ret.match(/^(?:[\w:]+::)?Result<([\s\S]*?)(,\s*[\w:]*Error)?>$/)?.[1].trim() ?? (ret || '()')
    commands.push({ name: m[2], async: Boolean(m[1]), params, returns: ok.replace(/\s+/g, ' '), doc: rustDoc(doc) })
    doc = []
    i = j - 1
  }
  return commands
}

function commandsPage() {
  const file = `${plugin}src/commands.rs`
  const commands = exists(file) ? parseCommands(read(file)) : []
  const title = 'Tauri commands'
  const description = 'Every Tauri command of the plugin with its arguments, generated from src/commands.rs.'
  if (!commands.length) {
    return page(title, description, 'src/commands.rs', `No \`#[tauri::command]\` functions in ${exists(file) ? link(file) : `\`${file}\``} yet. The commands (\`load\`, \`load_page\`, \`set_locale\`, \`get_locale\`, \`available_locales\`, \`pages\`, \`meta\`) land in M7.`)
  }

  const noPermission = commands.filter((c) => !permissionIds.has(`allow-${c.name.replace(/_/g, '-')}`)).map((c) => c.name)
  if (permissionIds.size && noPermission.length) console.warn(`generate.mjs: commands without an allow-* permission: ${noPermission.join(', ')}`)

  const rows = commands.map((c) => `| [${cell(c.name)}](#${c.name}) | ${oneLine(c.doc.split('\n')[0])} |`).join('\n')
  const details = commands
    .map((c) => {
      const permission = `allow-${c.name.replace(/_/g, '-')}`
      const params = c.params.length
        ? '| Argument (JS) | Rust parameter | Type |\n|---------------|----------------|------|\n' +
          c.params.map((p) => `| ${cell(camel(p.name))} | ${cell(p.name)} | ${cell(p.type)} |`).join('\n')
        : 'No arguments.'
      return [
        `### \`${c.name}\``,
        '',
        c.doc,
        '',
        '```ts',
        `invoke('plugin:i18n-next|${c.name}'${c.params.length ? `, { ${c.params.map((p) => camel(p.name)).join(', ')} }` : ''})`,
        '```',
        '',
        params,
        '',
        `Returns ${cell(c.returns)}. ${c.async ? 'Rust: `async fn`.' : 'Rust: plain (sync) `fn`.'}` +
          (permissionIds.has(permission) ? ` Permission: ${cell(`i18n-next:${permission}`)}.` : ''),
        '',
      ].join('\n')
    })
    .join('\n')

  const body = [
    `The \`#[tauri::command]\` functions in ${link(file)}, called with \`invoke('plugin:i18n-next|<name>', args)\`.`,
    'You normally use the [JavaScript API](/reference/javascript-api) instead. Arguments are sent in camelCase;',
    'parameters injected by Tauri (`AppHandle`, `State`, `Window`) are not listed.',
    '',
    '| Command | Description |',
    '|---------|-------------|',
    rows,
    '',
    '## Commands',
    '',
    details,
  ].join('\n')
  return page(title, description, 'src/commands.rs', body)
}

// ---------------------------------------------------------------- Changelog (CHANGELOG.md)

function changelogPage() {
  const description = 'All notable changes to i18n-next, tauri-plugin-i18n-next and tauri-plugin-i18n-next-api.'
  if (!exists('CHANGELOG.md')) return page('Changelog', description, 'CHANGELOG.md', 'No releases yet.')
  let body = read('CHANGELOG.md').replace(/^# .*\n/, '')
  // Relative links would 404 on the site.
  body = body.replace(/\]\((?!https?:|#|\/|mailto:)([^)]+)\)/g, (_, target) => `](${repo}${target})`)
  return page('Changelog', description, 'CHANGELOG.md', body.trim() || 'No releases yet.')
}

// ---------------------------------------------------------------- write

rmSync(refDir, { recursive: true, force: true })
mkdirSync(refDir, { recursive: true })
const out = {
  [`${refDir}.navigation.yml`]: 'title: API reference\nicon: i-lucide-file-code\n',
  [`${refDir}1.javascript-api.md`]: jsApiPage(),
  [`${refDir}2.types.md`]: typesPage(),
  [`${refDir}3.commands.md`]: commandsPage(),
  [`${refDir}4.permissions.md`]: permissionsPage(),
  [`${content}9.changelog.md`]: changelogPage(),
}
for (const [file, text] of Object.entries(out)) {
  writeFileSync(file, text)
  console.log(`generated ${file.slice(root.length)}`)
}
