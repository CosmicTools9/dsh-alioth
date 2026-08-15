/**
 * One-shot Alioth environment doctor: resolve the model snapshot (pulling from
 * github when needed), provision PostgreSQL when none is configured, bootstrap
 * the `isahl_meta` registry, and print a health report. Exit code 0 = green.
 *
 * Env overrides:
 *   ALIOTH_MODEL_SOURCE   github:owner/repo[@ref] | local path (default github:CosmicTools9/AppCreator@main)
 *   ALIOTH_DATABASE_URL   reuse an existing PostgreSQL instead of provisioning
 *   ALIOTH_DATA_ROOT      state root for snapshots + embedded cluster
 */
import { Context } from '@deepseek-ai/cordis'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { maskUrl } from '@dsh-alioth/env-alioth'

const databaseUrl = process.env.ALIOTH_DATABASE_URL
const dataRoot = process.env.ALIOTH_DATA_ROOT
const config: envAlioth.Config = {
  modelSource: process.env.ALIOTH_MODEL_SOURCE ?? 'github:CosmicTools9/AppCreator@main',
  ...(databaseUrl === undefined ? {} : { databaseUrl }),
  ...(dataRoot === undefined ? {} : { dataRoot }),
}

const ctx = new Context()
const fiber = await ctx.plugin(envAlioth, config)
const info = await ctx.aliothEnv.ready()
console.log(`model    ${info.modelVersion} @ ${info.sourceRef.slice(0, 12)}`)
console.log(`  dir    ${info.modelDir}`)
console.log(`db       ${maskUrl(info.databaseUrl)}`)
console.log(`boot     created=${info.bootstrap.created} stamped=${info.bootstrap.stamped}`
  + (info.bootstrap.drift === undefined ? '' : ` DRIFT stamped=${info.bootstrap.drift.stamped.modelVersion}/${info.bootstrap.drift.stamped.sourceRef.slice(0, 12)}`))

const report = await ctx.aliothEnv.doctor()
for (const check of report.checks) {
  console.log(`${check.ok ? '✓' : '✗'} ${check.name.padEnd(15)} ${check.detail}`)
}
console.log(`status   ${report.status}`)

await fiber.dispose()
process.exitCode = report.status === 'green' ? 0 : 1
