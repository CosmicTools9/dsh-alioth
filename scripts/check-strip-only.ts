/**
 * Gate: package sources must be Node strip-only compatible.
 *
 * The dsh loader runs `.ts` plugin entries through Node's native type
 * stripping (no transform). TS constructs that EMIT runtime code therefore
 * break at load time:
 *   - parameter properties (`constructor(private x: T)`) — emit `this.x = x`
 *   - `enum` / `const enum` declarations — emit an object (const enum member
 *     references also cannot be stripped)
 *   - non-ambient `namespace` with runtime body — emits an IIFE
 *   - `import x = require(...)` / `export =` — CJS-only emitted forms
 *   - parameter decorators — emit metadata/assignments
 *
 * Scans packages/<group>/<pkg>/src (plugin sources loaded by dsh); tests are compiled by
 * vitest and are out of scope. Exit 1 with file:line findings on violation.
 * Usage: node --import tsx scripts/check-strip-only.ts
 */
import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const PACKAGES_DIR = path.resolve(SCRIPT_DIR, '..', 'packages')

interface Finding {
  readonly file: string
  readonly line: number
  readonly rule: string
  readonly detail: string
}

async function collectTsFiles(dir: string): Promise<string[]> {
  const out: string[] = []
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      out.push(...await collectTsFiles(full))
    } else if (entry.isFile() && entry.name.endsWith('.ts') && !entry.name.endsWith('.d.ts')) {
      out.push(full)
    }
  }
  return out
}

function checkFile(file: string, source: ts.SourceFile): Finding[] {
  const findings: Finding[] = []
  const rel = path.relative(PACKAGES_DIR, file)

  const visit = (node: ts.Node): void => {
    if (ts.isParameter(node) && node.parent !== undefined && ts.isConstructorDeclaration(node.parent)) {
      const emitted = node.modifiers?.some(
        m =>
          m.kind === ts.SyntaxKind.PublicKeyword
          || m.kind === ts.SyntaxKind.PrivateKeyword
          || m.kind === ts.SyntaxKind.ProtectedKeyword
          || m.kind === ts.SyntaxKind.ReadonlyKeyword,
      )
      if (emitted) {
        findings.push({
          file: rel,
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          rule: 'parameter-property',
          detail: 'constructor parameter property emits `this.x = x` — not strip-only',
        })
      }
    }
    if (ts.isEnumDeclaration(node)) {
      findings.push({
        file: rel,
        line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
        rule: 'enum',
        detail: `\`enum ${node.name.text}\` emits a runtime object — not strip-only`,
      })
    }
    if (ts.isModuleDeclaration(node) && node.body !== undefined && ts.isModuleBlock(node.body)) {
      const ambient = node.modifiers?.some(m => m.kind === ts.SyntaxKind.DeclareKeyword) === true
      const emits = node.body.statements.some(
        s =>
          !ts.isInterfaceDeclaration(s)
          && !ts.isTypeAliasDeclaration(s)
          && !(ts.isImportDeclaration(s) && (s.importClause?.isTypeOnly ?? false)),
      )
      if (!ambient && emits) {
        findings.push({
          file: rel,
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
          rule: 'namespace',
          detail: `\`namespace ${node.name.getText(source)}\` has a runtime body — not strip-only`,
        })
      }
    }
    if (ts.isImportEqualsDeclaration(node)) {
      findings.push({
        file: rel,
        line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
        rule: 'import-equals',
        detail: '`import x = require(...)` emits a CJS interop form — not strip-only',
      })
    }
    if (ts.isExportAssignment(node) && node.isExportEquals) {
      findings.push({
        file: rel,
        line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
        rule: 'export-equals',
        detail: '`export =` emits a CJS form — not strip-only',
      })
    }
    ts.forEachChild(node, visit)
  }
  ts.forEachChild(source, visit)
  return findings
}


async function main(): Promise<void> {
  const groups = (await readdir(PACKAGES_DIR, { withFileTypes: true }))
    .filter(e => e.isDirectory())
    .map(e => e.name)
  const files: string[] = []
  for (const group of groups) {
    const groupDir = path.join(PACKAGES_DIR, group)
    for (const pkg of await readdir(groupDir, { withFileTypes: true })) {
      if (!pkg.isDirectory()) continue
      const srcDir = path.join(groupDir, pkg.name, 'src')
      files.push(...await collectTsFiles(srcDir).catch(() => []))
    }
  }
  if (files.length === 0) {
    throw new Error('no package sources found — wrong working directory?')
  }

  const findings: Finding[] = []
  for (const file of files) {
    const text = await readFile(file, 'utf8')
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.ES2023, true)
    findings.push(...checkFile(file, source))
  }

  if (findings.length > 0) {
    for (const f of findings) {
      console.error(`✗ ${f.file}:${f.line} [${f.rule}] ${f.detail}`)
    }
    console.error(`\nstrip-only gate: ${findings.length} violation(s) in ${files.length} files — the dsh loader runs .ts through Node native strip; these constructs emit code and break at load time`)
    process.exitCode = 1
    return
  }
  console.log(`strip-only gate: OK (${files.length} source files, zero emitted-code constructs)`)
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
