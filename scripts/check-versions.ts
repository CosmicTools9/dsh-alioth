/**
 * Gate: all workspace packages share one version — the root version.
 *
 * The plugin group releases as a unit (bundle mounts every package); mixed
 * versions (bundle 0.1.0, libs 0.0.0) are release artifacts waiting to
 * confuse. Rule: every packages/<group>/<pkg>/package.json `version` equals
 * the root package.json `version`. Examples stay 0.0.0 private and exempt.
 * Usage: node --import tsx scripts/check-versions.ts
 */
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(SCRIPT_DIR, '..')
const PACKAGES_DIR = path.join(ROOT, 'packages')

interface PkgJson {
  readonly name: string
  readonly version: string
  readonly license?: string
  readonly private?: boolean
}

async function readJson(file: string): Promise<PkgJson> {
  return JSON.parse(await readFile(file, 'utf8')) as PkgJson
}

async function main(): Promise<void> {
  const root = await readJson(path.join(ROOT, 'package.json'))
  const problems: string[] = []
  const seen: string[] = []

  for (const group of await readdir(PACKAGES_DIR, { withFileTypes: true })) {
    if (!group.isDirectory()) continue
    const groupDir = path.join(PACKAGES_DIR, group.name)
    for (const pkg of await readdir(groupDir, { withFileTypes: true })) {
      if (!pkg.isDirectory()) continue
      const file = path.join(groupDir, pkg.name, 'package.json')
      const manifest = await readJson(file).catch(() => null)
      if (manifest === null) continue
      seen.push(manifest.name)
      if (manifest.version !== root.version) {
        problems.push(`${manifest.name}: version ${manifest.version} != root ${root.version} (bump all packages together)`)
      }
      if (manifest.license !== root.license) {
        problems.push(`${manifest.name}: license "${manifest.license ?? 'MISSING'}" != root "${root.license}"`)
      }
    }
  }

  if (seen.length === 0) {
    throw new Error('no workspace packages found — wrong working directory?')
  }
  if (problems.length > 0) {
    for (const p of problems) console.error(`✗ ${p}`)
    console.error(`\nversion gate: ${problems.length} violation(s) over ${seen.length} packages`)
    process.exitCode = 1
    return
  }
  console.log(`version gate: OK (${seen.length} packages @ ${root.version}, license ${root.license})`)
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
