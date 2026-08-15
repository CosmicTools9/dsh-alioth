/**
 * Self-bootstrapping Alioth environment for the DeepSeek Harness. Pulls the
 * latest Alioth model snapshot (github `CosmicTools9/AppCreator` or a local
 * checkout), provisions PostgreSQL when none is configured, bootstraps the
 * `isahl_meta` entity registry per the model DDL baseline, and exposes a
 * read-only `doctor()` health report. Consumers (`tool-alioth`, orchestration
 * skills) call `ctx.aliothEnv.ready()` before touching the registry.
 * @module @dsh-alioth/env-alioth
 */

import { mkdir } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { Context, Service } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { bootstrapDatabase, type BootstrapResult } from './bootstrap.ts'
import { runDoctor, type DoctorReport } from './doctor.ts'
import { parseModelSource, resolveModelSnapshot, type ModelSnapshot } from './model-source.ts'
import { acquirePostgres, type PgHandle, type PgOptions } from './pg.ts'
export const name = 'env-alioth'
export const inject: readonly string[] = []

/** Deployment choices for the Alioth environment. */
export interface Config {
  /** Existing PostgreSQL URL (`postgres://...`). Omit to auto-provision an embedded instance under `dataRoot`. */
  readonly databaseUrl?: string
  /** `github:owner/repo[@ref]` or a filesystem path to an AppCreator checkout. */
  readonly modelSource: string
  /** State root for model snapshots and the embedded cluster. Default: XDG data home + `/dsh-alioth`. */
  readonly dataRoot?: string
}

export const Config: z<Config> = z.object({
  databaseUrl: z.string(),
  modelSource: z.string().default('github:CosmicTools9/AppCreator@main'),
  dataRoot: z.string(),
})

/** What `ready()` resolved: provenance plus how the database got there. */
export interface AliothEnvInfo {
  readonly databaseUrl: string
  readonly modelDir: string
  readonly sourceRef: string
  readonly modelVersion: string
  readonly bootstrap: BootstrapResult
}

/**
 * The `ctx.aliothEnv` service. Lazily converges the environment on first
 * `ready()`; a failure un-memoizes so the next call retries from scratch.
 * Disposal closes the database client and stops an owned embedded server.
 */
export class AliothEnv extends Service {
  constructor(ctx: Context, private readonly config: Config) {
    super(ctx, 'aliothEnv')
    ctx.effect(() => () => {
      const state = this.state
      this.state = undefined
      this.ensure = undefined
      return state?.handle.close()
    })
  }
  private ensure: Promise<AliothEnvInfo> | undefined
  private state: { handle: PgHandle; snapshot: ModelSnapshot } | undefined

  /** Resolve the environment (snapshot → database → bootstrap), memoized. */
  ready(): Promise<AliothEnvInfo> {
    if (this.ensure === undefined) {
      this.ensure = this.ensureNow().catch((error: unknown) => {
        void this.state?.handle.close().catch(() => {})
        this.state = undefined
        this.ensure = undefined
        throw error
      })
    }
    return this.ensure
  }

  /** Read-only health report over the resolved environment. */
  async doctor(): Promise<DoctorReport> {
    await this.ready()
    const state = this.state
    if (state === undefined) {
      throw new Error('env-alioth: doctor ran without a resolved environment')
    }
    return runDoctor(state.handle.client, state.snapshot)
  }

  private async ensureNow(): Promise<AliothEnvInfo> {
    const dataRoot = this.config.dataRoot
      ?? path.join(process.env.XDG_DATA_HOME ?? path.join(homedir(), '.local', 'share'), 'dsh-alioth')
    await mkdir(dataRoot, { recursive: true })
    const snapshot = await resolveModelSnapshot(parseModelSource(this.config.modelSource), dataRoot)
    const pgOptions: PgOptions = this.config.databaseUrl === undefined
      ? { dataRoot }
      : { url: this.config.databaseUrl, dataRoot }
    const handle = await acquirePostgres(pgOptions)
    this.state = { handle, snapshot }
    const bootstrap = await bootstrapDatabase(handle.client, snapshot.artifacts.ddlFiles, {
      modelVersion: snapshot.modelVersion,
      sourceRef: snapshot.sourceRef,
    })
    return {
      databaseUrl: handle.url,
      modelDir: snapshot.dir,
      sourceRef: snapshot.sourceRef,
      modelVersion: snapshot.modelVersion,
      bootstrap,
    }
  }
}
export { maskUrl } from './doctor.ts'

export function apply(ctx: Context, config: Config): void {
  ctx.plugin(AliothEnv, config)
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothEnv: AliothEnv
  }
}
