/**
 * App status for the console's Alioth right-Sidebar tab: a read-only projection of
 * one app workspace (`Pre-Proc/{ns}/Apps/{app}`) — artifact contract health, the
 * AppAgent run position, pending deferred gates, and the last closure verdict.
 *
 * Read-only by construction: nothing here writes, creates a fresh run, or sweeps
 * deferred items (`sweep` unlocks due gates — that belongs to the tools, not to a
 * panel that someone merely opened). Corrupt state is reported as data, never
 * thrown at the caller: one broken file must not blank the whole panel.
 * @module @dsh-alioth/auth-web-alioth/app-status
 */

import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import type { SessionApp } from '@dsh-alioth/auth-alioth'
import { validateArtifact } from '@dsh-alioth/gen-alioth'
import { readRun } from '@dsh-alioth/skill-alioth'
import { createDeferredStore, readClosureVerdicts } from '@dsh-alioth/verify-alioth'

/**
 * Contract validation of `app.json` plus the counts the panel shows (reached
 * through `AppStatus`; not part of the module's exported surface).
 */
interface AppStatusAppJson {
  readonly present: boolean
  readonly valid: boolean
  readonly errors: readonly string[]
  readonly name: string | null
  readonly status: string | null
  readonly version: string | null
  readonly modules: number
  readonly blocks: number
}

/** One pending gate registered against this app (or unattributed). */
interface AppStatusDeferredItem {
  readonly id: string
  readonly app: string | null
  readonly reason: string
  readonly createdAt: string
}

/** The whole panel payload. Every field is present; absences are explicit. */
export interface AppStatus {
  readonly ok: true
  readonly app: { readonly namespace: string; readonly code: string; readonly dir: string }
  readonly artifacts: {
    readonly appJson: AppStatusAppJson
    readonly extensions: {
      readonly files: number
      /** Loader-contract verification of the declared extension forms. */
      readonly verification: 'passed' | 'degraded' | 'absent'
    }
    readonly sources: { readonly dirs: number }
    readonly prototype: { readonly html: boolean }
    readonly modulesOnDisk: number
  }
  readonly pipeline: {
    readonly run:
      | { readonly present: false }
      | { readonly present: true; readonly trackIndex: number; readonly stepIndex: number; readonly completed: number; readonly lastCompleted: string | null }
      | { readonly present: true; readonly error: string }
    readonly deferred: { readonly open: number; readonly items: readonly AppStatusDeferredItem[] }
    readonly closure:
      | { readonly present: false }
      | { readonly present: true; readonly verdict: string; readonly seq: number; readonly at: string }
  }
}

async function readJson(file: string): Promise<{ value: unknown } | { error: string } | null> {
  let raw: string
  try {
    raw = await readFile(file, 'utf8')
  } catch {
    return null
  }
  try {
    return { value: JSON.parse(raw) as unknown }
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

/** Count files with the given extension directly inside `dir` (0 when absent). */
async function countFiles(dir: string, extension: string): Promise<number> {
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) return 0
  return entries.filter(entry => entry.isFile() && entry.name.endsWith(extension)).length
}

/** Count direct subdirectories of `dir` (0 when absent). */
async function countDirs(dir: string): Promise<number> {
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) return 0
  return entries.filter(entry => entry.isDirectory()).length
}

async function exists(file: string): Promise<boolean> {
  return await readFile(file, 'utf8').then(() => true, () => false)
}

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? value as Record<string, unknown> : {}
}

function asStringArray(value: unknown): readonly string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
}

async function readAppJson(dir: string): Promise<AppStatusAppJson> {
  const parsed = await readJson(path.join(dir, 'app.json'))
  if (parsed === null) {
    return { present: false, valid: false, errors: [], name: null, status: null, version: null, modules: 0, blocks: 0 }
  }
  if ('error' in parsed) {
    return { present: true, valid: false, errors: [`app.json 无法解析：${parsed.error}`], name: null, status: null, version: null, modules: 0, blocks: 0 }
  }
  const validation = validateArtifact('app', parsed.value)
  const document = asRecord(parsed.value)
  const config = asRecord(document.config)
  return {
    present: true,
    valid: validation.valid,
    errors: validation.errors,
    name: typeof document.name === 'string' ? document.name : null,
    status: typeof document.status === 'string' ? document.status : null,
    version: typeof document.version === 'string' ? document.version : null,
    modules: asStringArray(config.modules).length,
    blocks: asStringArray(config.blocks).length,
  }
}

/** Extension verification status: canonical report first, its degraded sibling second. */
async function readExtensionVerification(dir: string): Promise<'passed' | 'degraded' | 'absent'> {
  const canonical = await readJson(path.join(dir, 'extension-verify.json'))
  if (canonical !== null && !('error' in canonical)) {
    const status = asRecord(canonical.value).status
    return status === 'passed' || status === 'degraded' ? status : 'absent'
  }
  // A degraded run DELETES the canonical report, so its presence is the only
  // durable evidence of a degradation — never read it as "never verified".
  return await exists(path.join(dir, 'extension-verify.degraded.json')) ? 'degraded' : 'absent'
}

/** Whether a module's `module.json` is on disk (`Sources/{module}/` and `Modules/` layouts). */
async function countModulesOnDisk(dir: string): Promise<number> {
  let count = 0
  for (const parent of ['Sources', 'Modules']) {
    const entries = await readdir(path.join(dir, parent), { withFileTypes: true }).catch(() => null)
    if (entries === null) continue
    for (const entry of entries) {
      if (entry.isDirectory() && await exists(path.join(dir, parent, entry.name, 'module.json'))) count += 1
    }
  }
  return count
}

/**
 * Project one app workspace's status. `dataRoot` is the deployment data root that
 * holds `workflows/` (run state) and `deferred/` (pending gates) — the same root
 * the verify and workflow tools write to.
 * @param app - the session's app workspace, already authorized by the caller.
 * @param dataRoot - deployment data root (`ctx.aliothEnv.dataRoot()`).
 * @returns the panel payload; never throws for corrupt on-disk state.
 */
export async function buildAppStatus(app: SessionApp, dataRoot: string): Promise<AppStatus> {
  const run = await readRun(path.join(dataRoot, 'workflows'), { namespace: app.namespace, app: app.code })
    .then(record => record, error => ({ error: error instanceof Error ? error.message : String(error) }))

  const deferredItems = await createDeferredStore(dataRoot).all().catch(() => [] as const)
  const openItems: AppStatusDeferredItem[] = []
  for (const item of deferredItems) {
    // Unattributed items match any app on purpose (fail-closed default), so a
    // panel must show them rather than let them look absent for this app.
    if (item.app !== undefined && item.app !== app.code) continue
    openItems.push({ id: item.id, app: item.app ?? null, reason: item.reason, createdAt: item.createdTs })
  }

  const verdicts = await readClosureVerdicts(app.dir).catch(() => [] as const)
  const last = verdicts.length > 0 ? verdicts[verdicts.length - 1] : undefined

  return {
    ok: true,
    app: { namespace: app.namespace, code: app.code, dir: app.dir },
    artifacts: {
      appJson: await readAppJson(app.dir),
      extensions: {
        files: await countFiles(path.join(app.dir, 'extensions'), '.yaml'),
        verification: await readExtensionVerification(app.dir),
      },
      sources: { dirs: await countDirs(path.join(app.dir, 'Sources')) },
      prototype: { html: await exists(path.join(app.dir, 'prototype.html')) },
      modulesOnDisk: await countModulesOnDisk(app.dir),
    },
    pipeline: {
      run: run === null
        ? { present: false }
        : 'error' in run
          ? { present: true, error: run.error }
          : {
              present: true,
              trackIndex: run.position.trackIndex,
              stepIndex: run.position.stepIndex,
              completed: run.completed.length,
              lastCompleted: run.completed.length > 0 ? run.completed[run.completed.length - 1] ?? null : null,
            },
      deferred: { open: openItems.length, items: openItems },
      closure: last === undefined
        ? { present: false }
        : { present: true, verdict: last.verdict, seq: last.seq, at: last.ts },
    },
  }
}
