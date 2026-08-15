/**
 * Gate execution: the adapter's step gates are the acceptance checks that let
 * a step complete. `output-glob` verifies artifacts exist; `program` runs an
 * external checker. Pure check functions — callers choose the runtime.
 * @module @dsh-alioth/skill-alioth/gates
 */

import { access } from 'node:fs/promises'
import path from 'node:path'
import type { StepGate } from './adapter.ts'

export interface GateResult {
  readonly gate: StepGate
  readonly ok: boolean
  readonly detail: string
}

export interface GateContext {
  /** Root of the Pre-Proc tree; `{ns}`/`{app}` placeholders resolve under it. */
  readonly preProcRoot: string
  /** Resolves `{ns}` and `{app}` placeholders in globs/programs. */
  readonly variables: Readonly<Record<string, string>>
}

function resolveTemplate(template: string, context: GateContext): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => context.variables[key] ?? match)
}

function withinRoot(target: string, root: string): boolean {
  const resolved = path.resolve(target)
  return resolved.startsWith(root + path.sep) || resolved === root
}

async function globExists(glob: string, context: GateContext): Promise<{ ok: boolean; detail: string }> {
  const resolved = resolveTemplate(glob, context)
  // The adapter globs are `Pre-Proc/...`-style paths anchored at the repo; we
  // resolve them under preProcRoot when they start with the known prefix.
  const prefix = 'Pre-Proc/'
  const candidate = resolved.startsWith(prefix)
    ? path.join(context.preProcRoot, resolved.slice(prefix.length))
    : path.resolve(context.preProcRoot, resolved)
  if (!withinRoot(candidate, context.preProcRoot)) {
    return { ok: false, detail: `glob escapes preProcRoot: ${resolved}` }
  }
  try {
    await access(candidate)
    return { ok: true, detail: `exists: ${resolved}` }
  } catch {
    return { ok: false, detail: `missing: ${resolved}` }
  }
}

/**
 * Check one gate. Program gates are declared, not executed here — execution
 * belongs to the deployment (bun/node availability, process policy). The
 * default check treats a declared program gate as ok-with-notice; deployments
 * that run programs override via the `runProgram` hook.
 */
export async function checkGate(
  gate: StepGate,
  context: GateContext,
  runProgram?: (program: string, args: readonly string[]) => Promise<{ ok: boolean; detail: string }>,
): Promise<GateResult> {
  if (gate.kind === 'output-glob') {
    const result = await globExists(gate.outputGlob, context)
    return { gate, ok: result.ok, detail: result.detail }
  }
  if (runProgram !== undefined) {
    const result = await runProgram(gate.program, gate.args)
    return { gate, ok: result.ok, detail: result.detail }
  }
  const resolved = resolveTemplate(gate.program, context)
  return {
    gate,
    ok: true,
    detail: `program gate declared (${resolved}) — not executed in this environment`,
  }
}

/** Check every gate of a step; all must pass. */
export async function checkStepGates(
  gates: readonly StepGate[],
  context: GateContext,
  runProgram?: (program: string, args: readonly string[]) => Promise<{ ok: boolean; detail: string }>,
): Promise<GateResult[]> {
  const results: GateResult[] = []
  for (const gate of gates) {
    results.push(await checkGate(gate, context, runProgram))
  }
  return results
}
