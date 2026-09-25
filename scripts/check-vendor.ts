/**
 * Gate: vendored model artifacts stay attributable.
 *
 * packages/alioth/env-alioth/vendor/ redistributes Apache-2.0 artifacts of the
 * Alioth model (the isahl_meta structure baseline, skill-adapters, prototype
 * build chain, framework crates). This gate enforces the compliance rule from
 * AGENTS.md: LICENSE and NOTICE are present in the vendor tree and state the
 * license ("snapshot caches keep upstream license files").
 *
 * A per-file sha256 manifest (PROVENANCE.json) used to live here. It was retired
 * on 2026-09-24: the registry rows and other derived data are never committed, so
 * the manifest kept recording files a fresh clone cannot have, and every vendor
 * refresh had to hand-edit a derived file. Freshness against the truth source
 * (the AliothMeta checkout) is `pnpm run sync:framework --check`; the model registry's
 * freshness own gate is `pnpm run check:dicts`. Exit 1 on violation.
 */
import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const VENDOR_DIR = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'env-alioth', 'vendor')

async function walk(dir: string): Promise<string[]> {
  const out: string[] = []
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...await walk(full))
    else if (entry.isFile()) out.push(full)
  }
  return out
}

async function main(): Promise<void> {
  if (process.argv.includes('--update')) {
    console.error('vendor gate: --update was retired with PROVENANCE.json (2026-09-24); nothing to record')
    process.exitCode = 1
    return
  }

  const diskFiles = (await walk(VENDOR_DIR))
    .map(f => path.relative(VENDOR_DIR, f))
    .sort()

  const problems: string[] = []

  // 1. compliance files present
  for (const required of ['LICENSE', 'NOTICE']) {
    if (!diskFiles.includes(required)) {
      problems.push(`missing compliance file: vendor/${required}`)
    }
  }
  const notice = await readFile(path.join(VENDOR_DIR, 'NOTICE'), 'utf8').catch(() => '')
  if (!notice.includes('Apache License, Version 2.0')) {
    problems.push('vendor/NOTICE does not state the Apache-2.0 license')
  }
  const license = await readFile(path.join(VENDOR_DIR, 'LICENSE'), 'utf8').catch(() => '')
  if (!license.includes('Apache License')) {
    problems.push('vendor/LICENSE does not look like the Apache-2.0 text')
  }

  if (problems.length > 0) {
    for (const p of problems) console.error(`✗ ${p}`)
    console.error(`\nvendor gate: ${problems.length} violation(s)`)
    process.exitCode = 1
    return
  }
  console.log(`vendor gate: OK (${diskFiles.length} files, LICENSE + NOTICE present)`)
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
