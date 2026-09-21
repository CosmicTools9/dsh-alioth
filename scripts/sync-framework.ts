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
 * freshness gate (same discipline as `check:dicts --require-fresh`). Both modes
 * then assert the synced tree is self-consistent: every gate script an adapter
 * spawns and every declared `reference_paths` / `inputs` asset must resolve
 * inside the vendor tree, so a sync set can never ship an adapter whose
 * programs or context assets are absent. CI does not run this: the truth source
 * is the AliothStudio working checkout.
 * @module scripts/sync-framework
 */

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, rmdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { unreachableGatePrograms } from '@dsh-alioth/skill-alioth'
import { unreachableAdapterReferences } from './lib/adapter-references.ts'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..')
const VENDOR = path.join(REPO_ROOT, 'packages', 'alioth', 'env-alioth', 'vendor')
const STUDIO_ROOT = path.resolve(process.env.ALIOTH_STUDIO_ROOT ?? path.join(REPO_ROOT, '..', 'AliothStudio'))
const CHECK_ONLY = process.argv.includes('--check')

/** Declared sync set: AliothStudio source → vendored destination (relative to VENDOR). */
const SYNC_SET: readonly { readonly source: string; readonly dest: string }[] = [
  // 2026-09-14 upstream relocated the adapter set from the repo root into the
  // AppAgent crate (openspec change relocate-skill-adapters-into-appagent;
  // `scripts/check/check-agent-skill-adapter-paths.ts` now *forbids* a root
  // `skill-adapters/`). The vendored dest stays `skill-adapters` — that is the
  // layout the runtime and `unreachableGatePrograms` expect.
  { source: 'Meta/backend/app-agent/skill-adapters', dest: 'skill-adapters' },
  { source: 'scripts/prototype-tool.js', dest: 'scripts/prototype-tool.js' },
  { source: 'scripts/build-ns.sh', dest: 'scripts/build-ns.sh' },
  { source: 'scripts/cargo-check.sh', dest: 'scripts/cargo-check.sh' },
  { source: 'scripts/check/check-nav-hrefs.ts', dest: 'scripts/check/check-nav-hrefs.ts' },
  // Gate programs the adapters invoke directly: alioth-block 1.1 (check-block-json)
  // and alioth-service 1.5 (audit-service-spec). Absent here they spawn ENOENT →
  // GateErrorKind path-missing (not LLM-fixable) and the track stalls for good.
  { source: 'scripts/check/check-block-json.ts', dest: 'scripts/check/check-block-json.ts' },
  { source: 'scripts/check/audit-service-spec.ts', dest: 'scripts/check/audit-service-spec.ts' },
  { source: 'scripts/check/audit-css-framework.mjs', dest: 'scripts/check/audit-css-framework.mjs' },
  // Gate data the check scripts read (check-block-json.ts loads
  // baselines/block-json-baseline.json). Synced as a directory so a gate that
  // starts depending on another baseline cannot ship without its data.
  { source: 'scripts/check/baselines', dest: 'scripts/check/baselines' },
  { source: 'scripts/check/check-config-json.mjs', dest: 'scripts/check/check-config-json.mjs' },
  { source: 'scripts/check/check-module-blocks.mjs', dest: 'scripts/check/check-module-blocks.mjs' },
  { source: 'scripts/check/check-module-contract.mjs', dest: 'scripts/check/check-module-contract.mjs' },
  { source: 'scripts/check/check-shared-kernel.ts', dest: 'scripts/check/check-shared-kernel.ts' },
  // Prototype-chain gates + capability catalog (2026-09-22 catch-up). Each is
  // spawned by adapters that shipped without it:
  //   capability-catalog.ts          alioth-block 1.2 --check (alioth-block.yaml:29)
  //                                  alioth-module 1.2 (alioth-module.yaml:33) / 1.4 (:71)
  //   check-prototype-types.ts       alioth-app 1.3 (alioth-app.yaml:37), alioth-block 1.3
  //                                  (alioth-block.yaml:50), alioth-module 1.5 (alioth-module.yaml:87)
  //   check-prototype-render.ts      alioth-app 1.3 (alioth-app.yaml:40), alioth-block 1.3
  //                                  (alioth-block.yaml:53), alioth-module 1.5 (alioth-module.yaml:90)
  //   check-namespace-frontend.sh    alioth-gui 1.6 (alioth-gui.yaml:56). The .sh is a
  //                                  launcher: it execs check-namespace-frontend.ts (:17),
  //                                  which is not a gate arg and so would never be caught
  //                                  by the spawn-surface check — synced alongside.
  { source: 'scripts/capability-catalog.ts', dest: 'scripts/capability-catalog.ts' },
  { source: 'scripts/check/check-prototype-types.ts', dest: 'scripts/check/check-prototype-types.ts' },
  { source: 'scripts/check/check-prototype-render.ts', dest: 'scripts/check/check-prototype-render.ts' },
  { source: 'scripts/check/check-namespace-frontend.sh', dest: 'scripts/check/check-namespace-frontend.sh' },
  { source: 'scripts/check/check-namespace-frontend.ts', dest: 'scripts/check/check-namespace-frontend.ts' },
  // Build-regression eval toolchain (upstream `scripts/pre/gates.ts:1004-1016`
  // registers it as the `appagent-build-eval` push gate; the runner drives
  // appagent-client.ts and reads the case set). The case set is resolved from
  // the content root as `Meta/backend/app-agent/eval/cases.yaml` — the same
  // repo-relative path upstream uses (appagent-build-eval.ts:76) — so it is
  // vendored at that exact path, not at the top level.
  { source: 'scripts/eval/appagent-build-eval.ts', dest: 'scripts/eval/appagent-build-eval.ts' },
  { source: 'scripts/eval/appagent-client.ts', dest: 'scripts/eval/appagent-client.ts' },
  { source: 'scripts/eval/appagent-build-eval.selftest.ts', dest: 'scripts/eval/appagent-build-eval.selftest.ts' },
  { source: 'Meta/backend/app-agent/eval/cases.yaml', dest: 'Meta/backend/app-agent/eval/cases.yaml' },
  // Declared adapter references (reference_paths / fixed inputs) that the
  // reachability check below resolves — absent here the steps hand the model
  // asset paths that do not exist:
  //   alioth-block reference_paths      → docs/specs/BLOCK_SCHEMA.md (alioth-block.yaml:10)
  //   alioth-module reference_paths     → docs/specs/MODULE_SPEC.md (alioth-module.yaml:11)
  //   alioth-service reference_paths    → .agents/skills/alioth-service/references/
  //                                       (alioth-service.yaml:6)
  //   alioth-module 1.3 inputs          → Pre-Proc/Alioth/Prototypes/Modules/
  //                                       system-settings/llm-tsx/module.tsx
  //                                       (alioth-module.yaml:51 — the assembly reference)
  { source: 'docs/specs/BLOCK_SCHEMA.md', dest: 'docs/specs/BLOCK_SCHEMA.md' },
  { source: 'docs/specs/MODULE_SPEC.md', dest: 'docs/specs/MODULE_SPEC.md' },
  { source: '.agents/skills/alioth-service/references', dest: '.agents/skills/alioth-service/references' },
  {
    source: 'Pre-Proc/Alioth/Prototypes/Modules/system-settings/llm-tsx/module.tsx',
    dest: 'Pre-Proc/Alioth/Prototypes/Modules/system-settings/llm-tsx/module.tsx',
  },
  { source: 'scripts/eval/evaluate-prototype-reference.ts', dest: 'scripts/eval/evaluate-prototype-reference.ts' },
  { source: 'scripts/lib', dest: 'scripts/lib' },
  { source: '.agents/skills/alioth-design/references', dest: '.agents/skills/alioth-design/references' },
  { source: 'Framework/frontend/components/utilities.json', dest: 'Framework/frontend/components/utilities.json' },
  { source: 'Framework/backend', dest: 'Framework/backend' },
  { source: 'Gateway/backend', dest: 'Gateway/backend' },
  { source: 'SSO/backend', dest: 'SSO/backend' },
  // The gateway's default features path-dep on the Alioth baseline services —
  // without them the vendored gateway manifest cannot even parse.
  { source: 'Pre-Proc/Alioth/Sources/Apps/Services', dest: 'Pre-Proc/Alioth/Sources/Apps/Services' },
  { source: 'Pre-Proc/Alioth/openapi', dest: 'Pre-Proc/Alioth/openapi' },
]

/**
 * Directories never synced (build output / dependency trees inside a source
 * dir). `vendor/` is deliberately absent: the only directory of that name under
 * the sync set is `.agents/skills/alioth-design/references/vendor/` — the
 * prototype runtime (React UMD bundles, fonts) that the built `*-v{N}.html`
 * artifacts load. Matching it by name froze those 16 files out of every sync
 * while `--check` stayed green. No cargo vendor tree exists under any synced
 * entry (verified 2026-09-22).
 */
const EXCLUDED_SOURCE_DIRS = new Set(['target', 'node_modules'])

/**
 * Source-relative subtrees never synced, by the same rule as
 * `isExcludedSourceFile`: upstream gitignores them, so they are runtime state
 * of the checkout, not framework artifacts. `Gateway/backend/data/` is the
 * gateway's local file-storage root (`.gitignore:186`; uploads land in
 * `data/local-files/{ns}/{kind}/{id}/`) — vendoring it pins user uploads in
 * PROVENANCE.json and drifts on every local run.
 */
const EXCLUDED_SOURCE_SUBTREES: readonly string[] = ['Gateway/backend/data']

/**
 * Files never synced: editor/runtime artifacts the repository gitignores
 * (.DS_Store, .env and friends, logs). Syncing one puts an entry in
 * PROVENANCE.json that no clone can satisfy, and the vendor provenance gate
 * reports "manifest entry without file" in CI.
 */
function isExcludedSourceFile(name: string): boolean {
  return name === '.DS_Store'
    || name === '.env'
    || name.startsWith('.env.')
    || name.endsWith('.log')
}

function* walkFiles(root: string, sourcePrefix: string): Generator<string> {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name)
    if (entry.isDirectory()) {
      if (EXCLUDED_SOURCE_DIRS.has(entry.name)) continue
      const relative = `${sourcePrefix}/${entry.name}`
      if (EXCLUDED_SOURCE_SUBTREES.includes(relative)) continue
      yield* walkFiles(full, relative)
    } else if (entry.isFile()) {
      if (isExcludedSourceFile(entry.name)) continue
      yield full
    }
  }
}

/** Files under the vendor root, as vendor-relative paths (no source-side filters). */
function* walkDestFiles(root: string): Generator<string> {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (EXCLUDED_SOURCE_DIRS.has(entry.name)) continue
      for (const nested of walkDestFiles(path.join(root, entry.name))) {
        yield path.join(entry.name, nested)
      }
    } else if (entry.isFile()) {
      yield entry.name
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
  if (relative === 'scripts/build-ns.sh' && !content.includes('Generic namespace fallback (dsh-alioth)')) {
    // Arbitrary namespaces build exactly like the known ones (feature
    // {ns_lower} + sso); namespace-specific seed steps stay upstream-only.
    const search = [
      '  *)',
      '    echo "❌ Unknown namespace: $NS"',
      '    echo "   Supported: Alioth, WZ, AVIC-CAASEC, Cosmic-Tools, Meta"',
      '    exit 1',
      '    ;;',
    ].join('\n')
    const replacement = [
      '  *)',
      '    # Generic namespace fallback (dsh-alioth): any namespace whose',
      '    # {ns_lower} feature exists in the gateway manifest builds here.',
      '    TARGET_DIR="$PROJECT_ROOT/Deploy/$NS/bin"',
      '    BINARY_NAME="${NS_LOWER}-server"',
      '    echo "→ Building Gateway (features=$NS_LOWER, target=Deploy/$NS/target/)..."',
      '    cd "$PROJECT_ROOT/Gateway/backend"',
      '    BINARY_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/alioth-gateway"',
      '    cargo build $CARGO_FLAGS -p alioth-gateway --no-default-features --features "$NS_LOWER,sso" --target-dir "$PROJECT_ROOT/Deploy/$NS/target"',
      '    NEEDS_RESIGN=true',
      '    ;;',
    ].join('\n')
    if (content.includes(search)) {
      return content.replace(search, replacement)
    }
  }
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
  const produced = new Set<string>()
  let copied = 0
  let checked = 0
  for (const entry of SYNC_SET) {
    const source = path.join(STUDIO_ROOT, entry.source)
    if (!existsSync(source)) {
      console.error(`sync-framework: source missing: ${entry.source}`)
      return 1
    }
    const sourceIsFile = statSync(source).isFile()
    const sourceFiles = sourceIsFile ? [source] : [...walkFiles(source, entry.source)]
    for (const file of sourceFiles) {
      checked += 1
      // A file entry's dest IS the target; a dir entry's dest is the copy root.
      const destRelative = sourceIsFile ? entry.dest : path.join(entry.dest, path.relative(source, file))
      produced.add(destRelative)
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

  // A directory sync owns its subtree: a file left there by an earlier sync
  // whose source no longer produces it is pruned. Upstream moves modules
  // (`Framework/backend/identity-org/src/repository.rs` → `repository/`), and a
  // stale sibling of the new directory is an E0761 hard error for cargo, not
  // just dead weight. Only synced directory roots are scanned (`backend/ddl/**`
  // and the LICENSE/NOTICE pair belong to other scripts), and the by-name
  // exclusions above are kept — they are deployment templates, not sync output.
  const dirDests = SYNC_SET
    .filter(entry => statSync(path.join(STUDIO_ROOT, entry.source)).isDirectory())
    .map(entry => entry.dest)
  const stale = [...walkDestFiles(VENDOR)]
    .filter(relative => !produced.has(relative))
    .filter(relative => dirDests.some(dest => relative.startsWith(`${dest}${path.sep}`)))
    .filter(relative => !isExcludedSourceFile(path.basename(relative)))
    .sort()
  for (const relative of stale) {
    if (CHECK_ONLY) continue
    rmSync(path.join(VENDOR, relative), { force: true })
    // A pruned file can leave its directory empty; git tracks no empty
    // directory, so it would only surface as deploy noise.
    let parent = path.dirname(path.join(VENDOR, relative))
    while (parent !== VENDOR
      && !dirDests.some(dest => parent === path.join(VENDOR, dest))
      && readdirSync(parent).length === 0) {
      rmdirSync(parent)
      parent = path.dirname(parent)
    }
  }

  // A gate script the vendor tree cannot spawn stalls its track for good
  // (ENOENT classifies as path-missing, which is not LLM-fixable), so the sync
  // set must carry every script and data file the adapters invoke.
  const adaptersDir = path.join(VENDOR, 'skill-adapters')
  const unreachable = unreachableGatePrograms(adaptersDir, VENDOR)
  if (unreachable.length > 0) {
    console.error(`framework-sync: ${unreachable.length} adapter gate program(s) missing from the vendor tree:`)
    for (const item of unreachable) {
      console.error(`  - ${item.script} (${item.adapter} ${item.step})`)
    }
    console.error('add the source (and any data it reads) to SYNC_SET')
    return 1
  }

  // The reference surface: `reference_paths` / `inputs` assets the adapters
  // declare. Same class of defect as a missing gate program — the step hands
  // the model a path the vendor tree never carried, and the context it was
  // supposed to inject is silently absent. Checked in both modes.
  const unreachableRefs = unreachableAdapterReferences(adaptersDir, VENDOR)
  if (unreachableRefs.length > 0) {
    console.error(`framework-sync: ${unreachableRefs.length} adapter reference(s) missing from the vendor tree:`)
    for (const item of unreachableRefs) {
      console.error(`  - ${item.reference} (${item.adapter} ${item.step})`)
    }
    console.error('add the referenced source to SYNC_SET')
    return 1
  }

  if (CHECK_ONLY) {
    if (drifted.length > 0) {
      console.error(`framework-sync: ${drifted.length} file(s) drifted from AliothStudio (${STUDIO_ROOT}):`)
      for (const item of drifted.slice(0, 20)) console.error(`  - ${item}`)
      if (drifted.length > 20) console.error(`  … +${drifted.length - 20} more`)
      console.error('run `pnpm run sync:framework` to refresh, then `pnpm run check:vendor --update`')
      return 1
    }
    if (stale.length > 0) {
      console.error(`framework-sync: ${stale.length} vendored file(s) SYNC_SET no longer produces:`)
      for (const item of stale.slice(0, 20)) console.error(`  - ${item}`)
      if (stale.length > 20) console.error(`  … +${stale.length - 20} more`)
      console.error('run `pnpm run sync:framework` to prune, then `pnpm run check:vendor --update`')
      return 1
    }
    console.log(`framework-sync: OK (${checked} files match AliothStudio)`)
    return 0
  }

  // Gateway manifest normalization: upstream optional path deps use varying
  // ../ depth (some resolve OUTSIDE the checkout). Any leading ../ run longer
  // than two is clamped to ../../ — the content-root depth of Gateway/backend.
  const gatewayManifest = path.join(VENDOR, 'Gateway', 'backend', 'Cargo.toml')
  if (existsSync(gatewayManifest)) {
    const normalized = readFileSync(gatewayManifest, 'utf8').replace(
      /path = "(\.\.\/){3,}/g,
      'path = "../../',
    )
    if (normalized !== readFileSync(gatewayManifest, 'utf8')) {
      writeFileSync(gatewayManifest, normalized)
      console.log('framework-sync: normalized gateway manifest dep paths to content-root depth')
    }
  }

  const patches = reapplyLocalPatches()
  console.log(`framework-sync: synced ${copied} drifted file(s) of ${checked} (source: ${STUDIO_ROOT})`)
  if (stale.length > 0) {
    console.log(`framework-sync: pruned ${stale.length} stale file(s) SYNC_SET no longer produces`)
    for (const item of stale) console.log(`  - ${item}`)
  }
  for (const patch of patches) console.log(`framework-sync: patch re-applied — ${patch}`)
  console.log('next: pnpm run check:vendor --update')
  return 0
}

process.exit(main())
