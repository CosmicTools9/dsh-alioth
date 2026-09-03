/**
 * Self-bootstrapping Alioth environment for the DeepSeek Harness. Uses the
 * model's open distribution (github:CosmicTools9/Alioth, or the builtin
 * frozen vendor set — default), provisions
 * PostgreSQL when none is configured, bootstraps the `isahl_meta` entity
 * registry per the model DDL baseline, and exposes a read-only `doctor()`
 * health report. dsh-alioth is a sibling consumer of the Alioth model — not
 * a consumer of AppCreator's application products. Consumers (`tool-alioth`,
 * orchestration skills) call `ctx.aliothEnv.ready()` before touching the registry.
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
import type { QueryResult, QueryResultRow } from 'pg'
import { acquirePostgres, type PgHandle, type PgOptions } from './pg.ts'
export const name = 'env-alioth'
export const inject: readonly string[] = []

/** Deployment choices for the Alioth environment. */
export interface Config {
  /** Existing PostgreSQL URL (`postgres://...`). Omit to auto-provision an embedded instance under `dataRoot`. */
  readonly databaseUrl?: string
  /** `github:owner/repo[@ref]` or a filesystem path to a model-distribution checkout. */
  readonly modelSource: string
  /** State root for model snapshots and the embedded cluster. Default: XDG data home + `/dsh-alioth`. */
  readonly dataRoot?: string
}

export const Config: z<Config> = z.object({
  databaseUrl: z.string(),
  modelSource: z.string().default('builtin'),
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
 * The database handle lives for the service's lifetime (a reset drops the
 * registry schemas but keeps the cluster), while the snapshot memo clears on
 * reset so the next `ready()` re-bootstraps from the current model.
 * Disposal closes the database client and stops an owned embedded server.
 */
export class AliothEnv extends Service {
  private readonly config: Config

  constructor(ctx: Context, config: Config) {
    super(ctx, 'aliothEnv')
    this.config = config
    ctx.effect(() => () => {
      const handle = this.handle
      this.handle = undefined
      this.snapshot = undefined
      this.ensure = undefined
      return handle?.close()
    })
  }
  private ensure: Promise<AliothEnvInfo> | undefined
  private handle: PgHandle | undefined
  private snapshot: ModelSnapshot | undefined

  /** Resolve the environment (snapshot → database → bootstrap), memoized. */
  ready(): Promise<AliothEnvInfo> {
    if (this.ensure === undefined) {
      this.ensure = this.ensureNow().catch((error: unknown) => {
        void this.handle?.close().catch(() => {})
        this.handle = undefined
        this.snapshot = undefined
        this.ensure = undefined
        throw error
      })
    }
    return this.ensure
  }

  /** State root (model snapshots + embedded cluster + derived artifacts). */
  dataRoot(): string {
    return this.config.dataRoot
      ?? path.join(process.env.XDG_DATA_HOME ?? path.join(homedir(), '.local', 'share'), 'dsh-alioth')
  }

  /** Run a parameterized query against the bootstrapped registry database. */
  async sql<T extends QueryResultRow>(text: string, values?: readonly unknown[]): Promise<QueryResult<T>> {
    await this.ready()
    const handle = this.handle
    if (handle === undefined) {
      throw new Error('env-alioth: sql ran without a resolved environment')
    }
    return handle.client.query<T>(text, values === undefined ? undefined : [...values])
  }

  /** Read-only health report over the resolved environment. */
  async doctor(): Promise<DoctorReport> {
    await this.ready()
    const handle = this.handle
    const snapshot = this.snapshot
    if (handle === undefined || snapshot === undefined) {
      throw new Error('env-alioth: doctor ran without a resolved environment')
    }
    return runDoctor(handle.client, snapshot, this.dataRoot())
  }

  /**
   * Destructive registry reset: drops `isahl_meta` and the `dsh_alioth` stamp,
   * then invalidates the memoized snapshot so the next `ready()` re-runs the
   * model baseline from the current model. The database cluster stays up.
   * This is the explicit model upgrade path — never call it implicitly.
   */
  async resetRegistry(): Promise<void> {
    await this.ready()
    const handle = this.handle
    if (handle === undefined) {
      throw new Error('env-alioth: reset ran without a resolved environment')
    }
    await handle.client.query('DROP SCHEMA IF EXISTS isahl_meta CASCADE')
    await handle.client.query('DROP SCHEMA IF EXISTS dsh_alioth CASCADE')
    this.snapshot = undefined
    this.ensure = undefined
  }

  private async ensureNow(): Promise<AliothEnvInfo> {
    const dataRoot = this.config.dataRoot
      ?? path.join(process.env.XDG_DATA_HOME ?? path.join(homedir(), '.local', 'share'), 'dsh-alioth')
    await mkdir(dataRoot, { recursive: true })
    const snapshot = await resolveModelSnapshot(parseModelSource(this.config.modelSource), dataRoot)
    if (this.handle === undefined) {
      const pgOptions: PgOptions = this.config.databaseUrl === undefined
        ? { dataRoot }
        : { url: this.config.databaseUrl, dataRoot }
      this.handle = await acquirePostgres(pgOptions)
    }
    this.snapshot = snapshot
    const bootstrap = await bootstrapDatabase(this.handle.client, snapshot.artifacts.ddlFiles, {
      modelVersion: snapshot.modelVersion,
      sourceRef: snapshot.sourceRef,
    })
    return {
      databaseUrl: this.handle.url,
      modelDir: snapshot.dir,
      sourceRef: snapshot.sourceRef,
      modelVersion: snapshot.modelVersion,
      bootstrap,
    }
  }
}
export { maskUrl } from './doctor.ts'
export {
  provisionPrototypeRoot,
  removeProvisionedAssets,
  VENDOR_ROOT,
  type PrototypeRootInfo,
} from './prototype-root.ts'

export function apply(ctx: Context, config: Config): void {
  ctx.plugin(AliothEnv, config)
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothEnv: AliothEnv
  }
}
