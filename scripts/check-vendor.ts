/**
 * Gate: vendored model artifacts stay attributable and tamper-evident.
 *
 * packages/alioth/env-alioth/vendor/ redistributes Apache-2.0 artifacts of the
 * Alioth model (registry DDL/seed, skill-adapters, prototype build chain).
 * This gate enforces the compliance rules from AGENTS.md:
 *   1. LICENSE and NOTICE are present in the vendor tree
 *      ("snapshot caches keep upstream license files")
 *   2. every file on disk is recorded in PROVENANCE.json with a sha256
 *   3. no PROVENANCE.json entry is missing from disk (no silent removals)
 *
 * PROVENANCE.json is generated: `node --import tsx scripts/check-vendor.ts
 * --update` (run after any vendor refresh). Exit 1 on violation.
 */
import { createHash } from 'node:crypto'
import { readdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const VENDOR_DIR = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'env-alioth', 'vendor')
const PROVENANCE = path.join(VENDOR_DIR, 'PROVENANCE.json')
const UPDATE = process.argv.includes('--update')

interface Manifest {
  readonly description: string
  readonly license: 'Apache-2.0'
  readonly files: Readonly<Record<string, string>>
}

async function walk(dir: string): Promise<string[]> {
  const out: string[] = []
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...await walk(full))
    else if (entry.isFile()) out.push(full)
  }
  return out
}

async function sha256(file: string): Promise<string> {
  return createHash('sha256').update(await readFile(file)).digest('hex')
}

async function main(): Promise<void> {
  const diskFiles = (await walk(VENDOR_DIR))
    .map(f => path.relative(VENDOR_DIR, f))
    .filter(f => f !== 'PROVENANCE.json')
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

  if (UPDATE) {
    const files: Record<string, string> = {}
    for (const rel of diskFiles) files[rel] = await sha256(path.join(VENDOR_DIR, rel))
    const manifest: Manifest = {
      description: 'Vendored Alioth model artifacts (Apache-2.0, The Alioth Authors). Regenerate with scripts/check-vendor.ts --update after every vendor refresh.',
      license: 'Apache-2.0',
      files,
    }
    await writeFile(PROVENANCE, JSON.stringify(manifest, null, 2) + '\n')
    console.log(`vendor gate: PROVENANCE.json updated (${diskFiles.length} files)`)
    return
  }

  // 2./3. manifest round-trip
  const manifest = JSON.parse(await readFile(PROVENANCE, 'utf8')) as Manifest
  const recorded = new Set(Object.keys(manifest.files))
  for (const rel of diskFiles) {
    if (!recorded.has(rel)) {
      problems.push(`unrecorded file: vendor/${rel} (run check-vendor.ts --update or drop it)`)
      continue
    }
    const actual = await sha256(path.join(VENDOR_DIR, rel))
    if (manifest.files[rel] !== actual) {
      problems.push(`hash mismatch: vendor/${rel} (hand-edited? refresh via the model repo, never edit in place)`)
    }
  }
  for (const rel of recorded) {
    if (!diskFiles.includes(rel)) problems.push(`manifest entry without file: vendor/${rel}`)
  }

  if (problems.length > 0) {
    for (const p of problems) console.error(`✗ ${p}`)
    console.error(`\nvendor gate: ${problems.length} violation(s) over ${diskFiles.length} files`)
    process.exitCode = 1
    return
  }
  console.log(`vendor gate: OK (${diskFiles.length} files, LICENSE + NOTICE present, hashes match PROVENANCE.json)`)
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
