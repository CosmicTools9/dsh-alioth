/**
 * Framework sync: AliothStudio is the SOURCE OF TRUTH for the framework code
 * dsh-alioth vendors (design references, prototype toolchain, build/check
 * scripts, skill adapters, Framework backend crates). This script copies the
 * declared file set from a AliothStudio checkout into env-alioth/vendor/,
 * re-applies the one recorded local patch (PROTOTYPE_TOOL_ROOT), and refreshes
 * PROVENANCE.json.
 *
 *   ALIOTH_STUDIO_ROOT=../AliothStudio pnpm run sync:framework           # sync
 *   ALIOTH_STUDIO_ROOT=../AliothStudio pnpm run sync:framework --check   # drift report only
 *
 * `--check` compares sha256 per manifest file and exits 1 on drift — the local
 * freshness gate (same discipline as `check:dicts --require-fresh`). CI does
 * not run this: the truth source is the AliothStudio working checkout.
 * @module scripts/sync-framework
 */

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..')
const VENDOR = path.join(REPO_ROOT, 'packages', 'alioth', 'env-alioth', 'vendor')
const STUDIO_ROOT = path.resolve(process.env.ALIOTH_STUDIO_ROOT ?? path.join(REPO_ROOT, '..', 'AliothStudio'))
const CHECK_ONLY = process.argv.includes('--check')

/** Declared sync set: AliothStudio source → vendored destination (relative to VENDOR). */
const SYNC_SET: readonly { readonly source: string; readonly dest: string }[] = [
  { source: 'skill-adapters', dest: 'skill-adapters' },
  { source: 'scripts/prototype-tool.js', dest: 'scripts/prototype-tool.js' },
  { source: 'scripts/build-ns.sh', dest: 'scripts/build-ns.sh' },
  { source: 'scripts/cargo-check.sh', dest: 'scripts/cargo-check.sh' },
  { source: 'scripts/check/check-nav-hrefs.ts', dest: 'scripts/check/check-nav-hrefs.ts' },
  { source: 'scripts/check/audit-css-framework.mjs', dest: 'scripts/check/audit-css-framework.mjs' },
  { source: 'scripts/check/check-config-json.mjs', dest: 'scripts/check/check-config-json.mjs' },
  { source: 'scripts/check/check-module-blocks.mjs', dest: 'scripts/check/check-module-blocks.mjs' },
  { source: 'scripts/check/check-module-contract.mjs', dest: 'scripts/check/check-module-contract.mjs' },
  { source: 'scripts/check/check-shared-kernel.ts', dest: 'scripts/check/check-shared-kernel.ts' },
  { source: 'scripts/eval/evaluate-prototype-reference.ts', dest: 'scripts/eval/evaluate-prototype-reference.ts' },
  { source: 'scripts/lib/parsers.ts', dest: 'scripts/lib/parsers.ts' },
  { source: '.agents/skills/alioth-design/references', dest: '.agents/skills/alioth-design/references' },
  { source: 'Framework/frontend/components/utilities.json', dest: 'Framework/frontend/components/utilities.json' },
  { source: 'Framework/backend', dest: 'Framework/backend' },
]

/** Directories never synced (build output / dependency trees inside a source dir). */
const EXCLUDED_SOURCE_DIRS = new Set(['target', 'node_modules', 'vendor'])

function* walkFiles(root: string): Generator<string> {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name)
    if (entry.isDirectory()) {
      if (EXCLUDED_SOURCE_DIRS.has(entry.name)) continue
      yield* walkFiles(full)
    } else if (entry.isFile()) {
      yield full
    }
  }
}

function sha256(file: string): string {
  return createHash('sha256').update(readFileSync(file)).digest('hex')
}

/**
 * Local patches applied to synced content (re-applied after every sync; when
 * upstream adopts them the transform is a no-op). `relative` is the manifest
 * destination path. --check compares the PATCHED source content, so the
 * recorded local patch is never reported as drift.
 */
function withLocalPatches(relative: string, content: string): string {
  if (relative === 'scripts/cargo-check.sh' && !content.includes('CARGO_WORKSPACE_DIR')) {
    // The service crates live in the namespace workspace
    // (Pre-Proc/{ns}/Cargo.toml), not at the content root; deployments name
    // it via CARGO_WORKSPACE_DIR so `-p <crate>` resolves.
    return content.replace(
      'cd "$PROJECT_ROOT"',
      'cd "${CARGO_WORKSPACE_DIR:-$PROJECT_ROOT}"',
    )
  }
  if (relative === 'scripts/prototype-tool.js' && !content.includes('PROTOTYPE_TOOL_ROOT')) {
    return content.replace(
      "const ROOT = resolve(import.meta.dirname, '..');",
      "// PROTOTYPE_TOOL_ROOT: deployment override for the content root (the dir that\n"
      + "// contains Pre-Proc/, .agents/skills/alioth-design/references and Framework/).\n"
      + "// Defaults to the vendored tree's parent for upstream parity.\n"
      + "var ROOT = resolve(process.env.PROTOTYPE_TOOL_ROOT || resolve(import.meta.dirname, '..'));",
    )
  }
  return content
}

function reapplyLocalPatches(): string[] {
  const applied: string[] = []
  const tool = path.join(VENDOR, 'scripts', 'prototype-tool.js')
  if (existsSync(tool)) {
    const content = readFileSync(tool, 'utf8')
    const patched = withLocalPatches('scripts/prototype-tool.js', content)
    if (patched !== content) {
      writeFileSync(tool, patched)
      applied.push('prototype-tool.js: PROTOTYPE_TOOL_ROOT override')
    }
  }
  return applied
}

function main(): number {
  if (!existsSync(STUDIO_ROOT)) {
    console.error(`sync-framework: AliothStudio checkout not found at ${STUDIO_ROOT} — set ALIOTH_STUDIO_ROOT`)
    return 1
  }

  const drifted: string[] = []
  let copied = 0
  let checked = 0
  for (const entry of SYNC_SET) {
    const source = path.join(STUDIO_ROOT, entry.source)
    if (!existsSync(source)) {
      console.error(`sync-framework: source missing: ${entry.source}`)
      return 1
    }
    const sourceIsFile = statSync(source).isFile()
    const sourceFiles = sourceIsFile ? [source] : [...walkFiles(source)]
    for (const file of sourceFiles) {
      checked += 1
      // A file entry's dest IS the target; a dir entry's dest is the copy root.
      const destRelative = sourceIsFile ? entry.dest : path.join(entry.dest, path.relative(source, file))
      const target = path.join(VENDOR, destRelative)
      const patched = withLocalPatches(destRelative, readFileSync(file, 'utf8'))
      const sourceHash = createHash('sha256').update(patched).digest('hex')
      const destHash = existsSync(target) ? sha256(target) : ''
      if (sourceHash !== destHash) {
        drifted.push(destRelative)
        if (!CHECK_ONLY) {
          mkdirSync(path.dirname(target), { recursive: true })
          writeFileSync(target, patched)
          copied += 1
        }
      }
    }
  }

  if (CHECK_ONLY) {
    if (drifted.length > 0) {
      console.error(`framework-sync: ${drifted.length} file(s) drifted from AliothStudio (${STUDIO_ROOT}):`)
      for (const item of drifted.slice(0, 20)) console.error(`  - ${item}`)
      if (drifted.length > 20) console.error(`  … +${drifted.length - 20} more`)
      console.error('run `pnpm run sync:framework` to refresh, then `pnpm run check:vendor --update`')
      return 1
    }
    console.log(`framework-sync: OK (${checked} files match AliothStudio)`)
    return 0
  }

  const patches = reapplyLocalPatches()
  console.log(`framework-sync: synced ${copied} drifted file(s) of ${checked} (source: ${STUDIO_ROOT})`)
  for (const patch of patches) console.log(`framework-sync: patch re-applied — ${patch}`)
  console.log('next: pnpm run check:vendor --update')
  return 0
}

process.exit(main())
