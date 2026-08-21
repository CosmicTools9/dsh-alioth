/**
 * Gate: the checked-in semantic-mapping library (skill-alioth/src/data) stays
 * anchored and fresh.
 *
 * 1. Tamper-evidence (always): every dictionary file matches its sha256 in
 *    anchor.json. Hand-edits and partial regenerations fail here.
 * 2. Freshness (when the Alioth model repo is present — ALIOTH_REPO env or
 *    ../Alioth): regenerate into a temp dir and byte-diff against the
 *    checked-in files. Drift means the model moved and the plugin library is
 *    stale — entity-validate decisions silently diverge from the registry.
 *    Pass --require-fresh to fail when the repo is absent (CI mode).
 *
 * Refresh procedure: node --import tsx scripts/generate-semantic-dicts.ts
 * Usage: node --import tsx scripts/check-semantic-dicts.ts [--require-fresh]
 */
import { createHash } from 'node:crypto'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { generateDicts } from './generate-semantic-dicts.ts'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const DATA_DIR = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'skill-alioth', 'src', 'data')
const ALIOTH_REPO = process.env.ALIOTH_REPO ?? path.join(process.env.HOME ?? '', 'WorkSpace', 'Alioth')
const REQUIRE_FRESH = process.argv.includes('--require-fresh')

const DICTS = ['coordinates.json', 'physical-tables.json', 'fk-index.json'] as const

interface Anchor {
  readonly description: string
  readonly source: string
  readonly files: Readonly<Record<string, string>>
}

async function sha256(file: string): Promise<string> {
  return createHash('sha256').update(await readFile(file)).digest('hex')
}

async function pathExists(target: string): Promise<boolean> {
  return await readFile(target).then(() => true, () => false)
}

async function main(): Promise<void> {
  const problems: string[] = []

  // 1. anchor round-trip
  const anchor = JSON.parse(await readFile(path.join(DATA_DIR, 'anchor.json'), 'utf8')) as Anchor
  const anchored = new Set(Object.keys(anchor.files))
  for (const name of DICTS) {
    if (!anchored.has(name)) {
      problems.push(`anchor.json does not record ${name} (regenerate via generate-semantic-dicts.ts)`)
      continue
    }
    const actual = await sha256(path.join(DATA_DIR, name))
    if (anchor.files[name] !== actual) {
      problems.push(`${name}: hash != anchor (hand-edited or stale anchor — regenerate the library, never edit in place)`)
    }
  }
  for (const name of anchored) {
    if (!(DICTS as readonly string[]).includes(name)) {
      problems.push(`anchor.json records unknown file ${name}`)
    }
  }

  // 2. freshness vs the model repo
  let fresh = false
  if (await pathExists(path.join(ALIOTH_REPO, 'latest.json'))) {
    const tmp = await mkdtemp(path.join(tmpdir(), 'dicts-fresh-'))
    try {
      await generateDicts(tmp, ALIOTH_REPO)
      for (const name of DICTS) {
        const generated = await readFile(path.join(tmp, name), 'utf8')
        const checkedIn = await readFile(path.join(DATA_DIR, name), 'utf8')
        if (generated !== checkedIn) {
          problems.push(`${name}: stale — regeneration from ${ALIOTH_REPO} differs (run generate-semantic-dicts.ts and ship the new library)`)
        }
      }
      fresh = true
    } finally {
      await rm(tmp, { recursive: true, force: true })
    }
  } else if (REQUIRE_FRESH) {
    problems.push(`freshness required but Alioth repo not found at ${ALIOTH_REPO} (set ALIOTH_REPO)`)
  }

  if (problems.length > 0) {
    for (const p of problems) console.error(`✗ ${p}`)
    console.error(`\nsemantic-dict gate: ${problems.length} violation(s)`)
    process.exitCode = 1
    return
  }
  console.log(`semantic-dict gate: OK (anchored @ ${anchor.source.slice(0, 60)}${fresh ? ', fresh vs model repo' : ', freshness not checked (no ALIOTH_REPO)'})`)
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
