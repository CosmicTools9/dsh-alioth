/**
 * One-shot Alioth environment doctor: resolve the model snapshot (pulling from
 * github when needed), connect to the environment's PostgreSQL 18, bootstrap the
 * `isahl_meta` registry, and print a health report. Exit code 0 = green.
 *
 * Failures are reported as ONE line on stderr (`alioth-doctor: <reason>`, exit 1) — the reason is
 * already the whole diagnosis (missing model source, unreachable database, model database refused),
 * and a stack trace would only bury it.
 *
 * Env overrides:
 *   ALIOTH_MODEL_SOURCE   github:owner/repo[@ref] | local path (required — `builtin` was retired)
 *   ALIOTH_DATABASE_URL   the environment's PostgreSQL (required — fails loud when unset)
 *   ALIOTH_DATA_ROOT      state root for model snapshots
 * Flag: --reset           drop isahl_meta + stamp, then re-bootstrap from the snapshot (destructive)
 *       --allow-unbuilt-semantic-index
 *                        exit 0 when the ONLY failing check is the semantic index not being built
 *                        yet — the expected state of a fresh data root (a container's first boot).
 *                        Any other red still fails, and the status line says so instead of a bare red.
 */
import { Context } from '@deepseek-ai/cordis'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { maskUrl } from '@dsh-alioth/env-alioth'

async function main(): Promise<void> {
  const databaseUrl = process.env.ALIOTH_DATABASE_URL
  const dataRoot = process.env.ALIOTH_DATA_ROOT
  const reset = process.argv.includes('--reset')
  const allowUnbuiltSemanticIndex = process.argv.includes('--allow-unbuilt-semantic-index')
  const config: envAlioth.Config = {
    modelSource: envAlioth.requireModelSource(),
    ...(databaseUrl === undefined ? {} : { databaseUrl }),
    ...(dataRoot === undefined ? {} : { dataRoot }),
  }

  const ctx = new Context()
  const fiber = await ctx.plugin(envAlioth, config)
  try {
    if (reset) {
      // Destructive by request: drop the registry and re-bootstrap from the snapshot.
      await ctx.aliothEnv.resetRegistry()
    }
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
    // The waiver covers exactly one red. Report the verdict the exit code expresses — a bare
    // `status red` next to exit 0 is what a container log reader would rightly call a contradiction.
    const blocking = report.checks.filter(check => !check.ok
      && !(allowUnbuiltSemanticIndex && check.name === 'semantic-index'))
    const waived = report.status !== 'green' && blocking.length === 0
    console.log(waived
      ? 'status   green (waived: semantic-index not built yet — rebuilds on the first semantic_search)'
      : `status   ${report.status}`)
    process.exitCode = report.status === 'green' || waived ? 0 : 1
  } finally {
    await fiber.dispose()
  }
}

main().catch((error: unknown) => {
  console.error(`alioth-doctor: ${error instanceof Error ? error.message : String(error)}`)
  process.exitCode = 1
})
