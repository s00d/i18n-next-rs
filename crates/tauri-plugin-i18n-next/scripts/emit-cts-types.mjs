// dist-js/*.d.ts -> *.d.cts for the `require` condition. A plain copy is wrong once an entry imports another:
// `from './index.js'` inside a .d.cts would resolve to the ESM types, so relative `.js` specifiers become `.cjs`.
// Only real module specifiers change (import / export declarations, `import x = require()`, `import()` types),
// found with the TypeScript parser, so comments and JSDoc examples stay as written.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import ts from 'typescript'

function specifiers(file) {
  const out = []
  const visit = (node) => {
    let spec
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) spec = node.moduleSpecifier
    else if (ts.isExternalModuleReference(node)) spec = node.expression
    else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) spec = node.argument.literal
    if (spec && ts.isStringLiteral(spec)) out.push(spec)
    ts.forEachChild(node, visit)
  }
  visit(file)
  return out
}

const dir = new URL('../dist-js/', import.meta.url)
for (const name of readdirSync(dir).filter((n) => n.endsWith('.d.ts'))) {
  let text = readFileSync(new URL(name, dir), 'utf8')
  const file = ts.createSourceFile(name, text, ts.ScriptTarget.Latest, true)
  // From the end, so earlier positions stay valid.
  for (const spec of specifiers(file).reverse()) {
    if (!/^\.{1,2}\//.test(spec.text)) continue
    if (!spec.text.endsWith('.js')) throw new Error(`${name}: relative import without .js: ${spec.text} (write './x.js' in guest-js)`)
    const start = spec.getStart(file) + 1 // inside the quote
    text = text.slice(0, start) + spec.text.slice(0, -3) + '.cjs' + text.slice(spec.end - 1)
  }
  writeFileSync(new URL(name.replace(/\.d\.ts$/, '.d.cts'), dir), text)
}
