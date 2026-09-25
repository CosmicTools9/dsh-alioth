/**
 * Assemble a **model source** directory — what `ALIOTH_MODEL_SOURCE` points at now that the
 * package's frozen snapshot (`builtin`) is retired.
 *
 * Two modes, one implementation (`scripts/lib/model-source-fixture.ts`):
 *
 * ```sh
 * # A deployment: channel content (version anchor + registry rows) + the vendored kit.
 * mise run alioth:model-source -- --source github:CosmicTools9/Alioth@v10.0.34 --out ~/.dsh-alioth/model-source
 * mise run alioth:model-source -- --source ~/WorkSpace/Alioth --out ~/.dsh-alioth/model-source
 *
 * # A self-check (keyless, network-free): kit + a tiny registry seed.
 * mise run alioth:model-source -- --out /tmp/alioth-check/model-source --fixture-seed
 * ```
 *
 * Exit code is non-zero when the assembled source is not usable: no adapters, or (outside fixture
 * mode) no registry rows — so a deployment never silently boots an empty registry.
 * @module scripts/assemble-model-source
 */

import { homedir } from 'node:os'
import path from 'node:path'
import { inspectModelArtifacts, parseModelSource, resolveModelSnapshot } from '@dsh-alioth/env-alioth'
import { assembleModelSource, FIXTURE_REGISTRY_SEED } from './lib/model-source-fixture.ts'

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(`--${name}`)
  return index === -1 ? undefined : process.argv[index + 1]
}

const out = arg('out')
const fixture = process.argv.includes('--fixture-seed')
const source = arg('source') ?? process.env.ALIOTH_MODEL_SOURCE

if (out === undefined) {
  console.error('assemble-model-source: --out <dir> is required')
  process.exit(1)
}
if (fixture && source !== undefined) {
  console.error('assemble-model-source: --fixture-seed and --source are mutually exclusive')
  process.exit(1)
}
if (!fixture && (source === undefined || source.trim().length === 0)) {
  console.error('assemble-model-source: --source <spec> (or ALIOTH_MODEL_SOURCE) is required; --fixture-seed builds a self-check fixture')
  process.exit(1)
}

const dataRoot = process.env.ALIOTH_DATA_ROOT
  ?? path.join(process.env.XDG_DATA_HOME ?? path.join(homedir(), '.local', 'share'), 'dsh-alioth')

const releaseDir = fixture
  ? undefined
  : (await resolveModelSnapshot(parseModelSource(source as string), dataRoot)).dir

const { adapters } = await assembleModelSource(out, {
  ...(releaseDir === undefined ? {} : { releaseDir }),
  ...(fixture ? { seedSql: FIXTURE_REGISTRY_SEED } : {}),
})

const artifacts = await inspectModelArtifacts(out)
const problems: string[] = []
if (adapters === 0) {
  problems.push('no skill-adapters were copied — the kit (packages/alioth/env-alioth/vendor/skill-adapters) is missing or empty')
}
if (!fixture && artifacts.registrySource !== 'snapshot') {
  problems.push('the source carries no *isahl_meta*.sql registry rows — pass a model source that has the release sidecar (derived data, delivered out of band)')
}
if (artifacts.publicationVersion === undefined) {
  problems.push('the source carries no latest.json version anchor')
}

console.log(`assemble-model-source: ${out}`)
console.log(`  release      ${releaseDir === undefined ? '(fixture)' : releaseDir}`)
console.log(`  version      ${artifacts.publicationVersion ?? '(none)'}`)
console.log(`  registry     ${artifacts.registrySource} (${artifacts.ddlFiles.length} DDL file(s))`)
console.log(`  adapters     ${adapters}`)
if (problems.length > 0) {
  for (const problem of problems) console.error(`✗ ${problem}`)
  process.exit(1)
}
console.log('assemble-model-source: OK')
