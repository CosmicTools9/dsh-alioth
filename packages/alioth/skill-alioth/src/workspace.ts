/**
 * File-backed workflow state: one JSON file per app creation run under a
 * workspace root. The run carries the adapter, position, and completed step
 * ids; persistence makes multi-turn creation resumable across sessions while
 * keeping the durable truth on disk (no DB dependency).
 * @module @dsh-alioth/skill-alioth/workspace
 */

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { initialRunState, type RunPosition, type RunState } from './state.ts'
import type { Adapter } from './adapter.ts'

export interface RunMeta {
  readonly namespace: string
  readonly app: string
}

/** What a run file carries: positional indices plus the completed step ids. */
export interface RunRecord {
  readonly position: RunPosition
  readonly completed: readonly string[]
}

/**
 * Read a run's persisted state WITHOUT side effects — `null` when this app has no run
 * file yet. `loadRun` builds on it: a read-only surface (the console's app status panel)
 * must never materialize a fresh run just by looking at it, which `loadRun` does on
 * purpose to start one.
 */
export async function readRun(workspaceRoot: string, meta: RunMeta): Promise<RunRecord | null> {
  const file = runFile(workspaceRoot, meta)
  let raw: string
  try {
    raw = await readFile(file, 'utf8')
  } catch {
    return null
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    throw new Error(`skill-alioth: corrupt run state at ${file}`)
  }
  const position = (parsed as { position?: unknown }).position
  const completed = (parsed as { completed?: unknown }).completed
  if (
    typeof position !== 'object' || position === null
    || !Number.isInteger((position as { trackIndex?: unknown }).trackIndex)
    || !Number.isInteger((position as { stepIndex?: unknown }).stepIndex)
    || !Array.isArray(completed) || completed.some(entry => typeof entry !== 'string')
  ) {
    throw new Error(`skill-alioth: invalid run state at ${file}`)
  }
  return {
    position: {
      trackIndex: (position as { trackIndex: number }).trackIndex,
      stepIndex: (position as { stepIndex: number }).stepIndex,
    },
    completed: completed as string[],
  }
}

/** Load a run's state; starts a fresh run at the adapter's first step when absent. */
export async function loadRun(workspaceRoot: string, meta: RunMeta, adapter: Adapter): Promise<RunState> {
  const record = await readRun(workspaceRoot, meta)
  if (record !== null) {
    return { adapter, position: record.position, completed: record.completed }
  }
  const fresh = initialRunState(adapter)
  await saveRun(workspaceRoot, meta, fresh)
  return fresh
}

/** Persist a run's state atomically-ish (write temp then rename). */
export async function saveRun(workspaceRoot: string, meta: RunMeta, state: RunState): Promise<void> {
  const file = runFile(workspaceRoot, meta)
  await mkdir(path.dirname(file), { recursive: true })
  const payload = { position: state.position, completed: state.completed }
  await writeFile(file, `${JSON.stringify(payload, null, 2)}\n`)
}

function runFile(workspaceRoot: string, meta: RunMeta): string {
  return path.join(workspaceRoot, meta.namespace, meta.app, 'run-state.json')
}
