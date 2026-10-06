/**
 * Generate the tsconfig path map that lets this checkout typecheck against the
 * sibling harness sources instead of its built `lib/` declarations.
 *
 * The harness checkout (`../deepseek-harness`, override with `HARNESS_ROOT`) is
 * the source of truth for every `@deepseek-ai/*` package this repo consumes, so
 * a path map derived from its manifests keeps typecheck in step with the fork
 * without a publish or an install. Only the typecheck program extends the
 * generated file: the emit build keeps resolving through node_modules to each
 * package's `exports`, because compiling another checkout's sources into this
 * one's output would move `rootDir` and duplicate module identity.
 *
 *   pnpm exec tsx scripts/gen-harness-paths.ts            # write
 *   pnpm exec tsx scripts/gen-harness-paths.ts --check    # drift gate
 * @module scripts/gen-harness-paths
 */

import { existsSync, globSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..')
const HARNESS = path.resolve(process.env.HARNESS_ROOT ?? path.join(REPO_ROOT, '..', 'deepseek-harness'))
const OUTPUT = path.join(REPO_ROOT, 'tsconfig.harness-paths.json')
const CHECK_ONLY = process.argv.includes('--check')

/** One subpath export of a harness package, mapped to the source that produced it. */
interface HarnessExport {
  /** Import specifier this repo can write, e.g. `@deepseek-ai/dsh-session/types`. */
  readonly specifier: string
  /** Source file relative to the harness checkout, e.g. `packages/core/session/src/index.ts`. */
  readonly source: string
}

/**
 * Map one export entry's declaration path back to its source file: the emitted
 * `lib/types/<x>.d.ts` comes from `src/<x>.ts`, and `lib/types/<x>/index.d.ts`
 * from `src/<x>/index.ts`.
 * @param typesPath - the `types` target of one export entry.
 * @returns the source path relative to the package directory, or undefined when the shape is unknown.
 */
function sourceOfTypesPath(typesPath: string): string | undefined {
  const emitted = /^\.\/lib\/types\/(.+)\.d\.ts$/.exec(typesPath)?.[1]
  if (emitted === undefined) return undefined
  if (emitted === 'index') return 'src/index.ts'
  if (emitted.endsWith('/index')) return `src/${emitted.slice(0, -'/index'.length)}/index.ts`
  return `src/${emitted}.ts`
}

/**
 * Packages whose sources are excluded from the map and consumed through their
 * built declarations instead. Same class of exclusion as the vendor tier:
 * - `dsh-llm-deepseek` compiles its SSE parser against the DOM type surface
 *   (`BufferSource`) that the harness's own typecheck provides but this repo's
 *   lib set deliberately does not — mapping it here would drag the whole
 *   settings/config-editor/app-boot source chain into this program, where the
 *   vendored loader's type identity then clashes with the registry copy
 *   app-picker installs (measured: 9 errors, all in harness boot sources).
 * A consumer importing these two packages typechecks against `lib/types/*.d.ts`
 * of the sibling checkout — the checkout is built first by contract.
 */
const SOURCE_EXCLUDED = new Set([
  '@deepseek-ai/dsh-llm-deepseek',
  '@deepseek-ai/dsh-llm-deepseek-api-key',
])

/**
 * Collect every specifier this repo may import from the harness checkout.
 *
 * The vendor tier (`@deepseek-ai/cordis`, its plugin family, `schemastery`,
 * `cosmokit`) is deliberately absent: its sources are not written under the
 * strictness set the harness packages use, so compiling them with this repo's
 * options fails (measured: 92 errors, all in vendor/cordis + vendor/loader),
 * and its built declarations are the boundary a consumer imports anyway.
 * @returns the mapped exports, sorted by specifier.
 */
function collectExports(): HarnessExport[] {
  const manifests = [
    ...globSync('packages/*/*/package.json', { cwd: HARNESS }),
    ...globSync('native/*/packages/*/package.json', { cwd: HARNESS }),
  ].sort()
  const collected: HarnessExport[] = []
  for (const manifest of manifests) {
    const directory = path.dirname(manifest).split(path.sep).join('/')
    const parsed = JSON.parse(readFileSync(path.join(HARNESS, manifest), 'utf8')) as {
      name?: string
      exports?: Record<string, { types?: string } | string>
    }
    if (parsed.name === undefined || parsed.exports === undefined) continue
    if (SOURCE_EXCLUDED.has(parsed.name)) continue
    for (const [key, target] of Object.entries(parsed.exports)) {
      // `./package.json` is not code, and the `./src/*` escape hatch names a
      // source path that a TS `paths` entry cannot express as a subpath pattern.
      if (key === './package.json' || key.startsWith('./src/')) continue
      const types = typeof target === 'string' ? undefined : target.types
      if (types === undefined) continue
      const source = sourceOfTypesPath(types)
      if (source === undefined || !existsSync(path.join(HARNESS, directory, source))) continue
      collected.push({ specifier: key === '.' ? parsed.name : `${parsed.name}/${key.slice(2)}`, source: `${directory}/${source}` })
    }
  }
  return collected.sort((left, right) => left.specifier.localeCompare(right.specifier))
}

/**
 * Render the generated tsconfig. Targets stay relative to this file so the map
 * travels with the checkout layout instead of one machine's absolute paths.
 * @param entries - the collected exports.
 * @returns the file contents.
 */
function render(entries: readonly HarnessExport[]): string {
  const lines = [
    '{',
    '  // Generated by scripts/gen-harness-paths.ts from the harness manifests.',
    '  // Keeps typecheck on the fork sources; the emit build still resolves',
    '  // through node_modules to each package exports.',
    '  "compilerOptions": {',
    '    "paths": {',
  ]
  for (const entry of entries) {
    lines.push(`      ${JSON.stringify(entry.specifier)}: [${JSON.stringify(path.posix.relative(REPO_ROOT, path.join(HARNESS, entry.source)))}],`)
  }
  lines.push('    }', '  }', '}', '')
  return lines.join('\n')
}

const entries = collectExports()
const generated = render(entries)
if (CHECK_ONLY) {
  const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, 'utf8') : ''
  if (current !== generated) {
    console.error(`gen-harness-paths: ${path.relative(REPO_ROOT, OUTPUT)} is stale — run \`pnpm exec tsx scripts/gen-harness-paths.ts\``)
    process.exit(1)
  }
  console.log(`gen-harness-paths: ${String(entries.length)} specifier(s) up to date`)
} else {
  writeFileSync(OUTPUT, generated, 'utf8')
  console.log(`gen-harness-paths: wrote ${String(entries.length)} specifier(s) from ${HARNESS}`)
}
