/**
 * Gate execution: the adapter's step gates are the acceptance checks that let
 * a step complete. Mirrors the upstream dialog-loop-era contract
 * (`pipeline/stage.rs` GateResult Pass/NotAttempted/Fail, `skills/mod.rs`
 * GateErrorKind): output-glob verifies artifacts exist; program gates run an
 * external checker and compare the exit code against `expected_exit_code`,
 * optionally also verifying the `output_glob` artifact they must produce.
 * Pure check functions — callers choose the runtime.
 * @module @dsh-alioth/skill-alioth/gates
 */

import { globSync } from 'node:fs'
import { access as accessPath } from 'node:fs/promises'
import path from 'node:path'
import type { StepGate } from './adapter.ts'

/** Upstream GateResult: Pass / NotAttempted (declared, not executed) / Fail. */
export type GateStatus = 'pass' | 'not-attempted' | 'fail'

/**
 * Upstream GateErrorKind — decides the retry policy: contract failures are
 * LLM-output quality (retryable in dialogue); tool-whitelist and path-missing
 * are environment refusals (fast-fail); other is uncategorized/transient.
 */
export type GateErrorKind = 'contract' | 'tool-whitelist' | 'path-missing' | 'other'

export function isLlmFixable(kind: GateErrorKind): boolean {
  return kind === 'contract' || kind === 'other'
}

/**
 * Classify a gate failure from its evidence (upstream `classify_error`
 * marker style, narrowed to the GateErrorKind set). Precedence: runner
 * refusals → environment paths → timeouts → default contract (an exit-code
 * mismatch is output quality by definition).
 */
export function classifyGateError(detail: string): GateErrorKind {
  const lowered = detail.toLowerCase()
  if (/whitelist|allowlist|not allowed|rejected|denied|permission denied|forbidden/.test(lowered)) {
    return 'tool-whitelist'
  }
  if (/\benoent\b|no such file|not found|spawn .* failed|escapes preprocroot/.test(lowered)) {
    return 'path-missing'
  }
  if (/timeout|timed out/.test(lowered)) {
    return 'other'
  }
  return 'contract'
}

export interface GateResult {
  readonly gate: StepGate
  readonly status: GateStatus
  readonly detail: string
  readonly errorKind?: GateErrorKind
}

export interface GateContext {
  /** Root of the Pre-Proc tree; `{ns}`/`{app}` placeholders resolve under it. */
  readonly preProcRoot: string
  /** Resolves `{ns}` and `{app}` placeholders in globs/programs. */
  readonly variables: Readonly<Record<string, string>>
}

export interface ProgramResult {
  /** Spawned and exited; `exitCode` is null when the program never ran. */
  readonly ok: boolean
  readonly exitCode: number | null
  readonly detail: string
}

export type ProgramRunner = (
  program: string,
  args: readonly string[],
  gate: StepGate,
) => Promise<ProgramResult>

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
  // Glob patterns (a-v*.html) match against the artifact tree; concrete
  // paths stay a plain existence check.
  if (/[*?[]/.test(candidate)) {
    const matches = globSync(candidate)
    return matches.length > 0
      ? { ok: true, detail: `matches: ${resolved} (${matches.length} file${matches.length === 1 ? '' : 's'})` }
      : { ok: false, detail: `no match for glob: ${resolved}` }
  }
  try {
    await accessPath(candidate)
    return { ok: true, detail: `exists: ${resolved}` }
  } catch {
    return { ok: false, detail: `missing: ${resolved}` }
  }
}

/**
 * Check one gate. Program gates are declared, not executed here — execution
 * belongs to the deployment (bun/node availability, process policy). Without
 * a runner the gate is `not-attempted` (never silently passed).
 */
export async function checkGate(
  gate: StepGate,
  context: GateContext,
  runProgram?: ProgramRunner,
): Promise<GateResult> {
  if (gate.kind === 'output-glob') {
    const result = await globExists(gate.outputGlob, context)
    return {
      gate,
      status: result.ok ? 'pass' : 'fail',
      detail: result.detail,
      ...(result.ok ? {} : { errorKind: classifyGateError(result.detail) }),
    }
  }
  if (runProgram === undefined) {
    const resolved = resolveTemplate(gate.program, context)
    return {
      gate,
      status: 'not-attempted',
      detail: `program gate declared (${resolved}) — not executed in this environment`,
    }
  }
  // Template resolution applies to program gates too: adapter args carry
  // `{ns}`/`{app}` placeholders that must resolve before invocation.
  const program = resolveTemplate(gate.program, context)
  const args = gate.args.map(arg => resolveTemplate(arg, context))
  const result = await runProgram(program, args, gate)
  if (result.exitCode === null) {
    return {
      gate,
      status: 'fail',
      detail: result.detail,
      errorKind: classifyGateError(result.detail),
    }
  }
  if (result.exitCode !== gate.expectedExitCode) {
    const detail = `${result.detail} (expected exit ${gate.expectedExitCode})`
    return { gate, status: 'fail', detail, errorKind: classifyGateError(detail) }
  }
  if (gate.outputGlob !== undefined) {
    // Program passed: the artifact it must produce is verified too
    // (upstream: `output_glob` on a program gate is checked after execution).
    const glob = await globExists(gate.outputGlob, context)
    if (!glob.ok) {
      return { gate, status: 'fail', detail: glob.detail, errorKind: classifyGateError(glob.detail) }
    }
    return { gate, status: 'pass', detail: `${result.detail}; ${glob.detail}` }
  }
  return { gate, status: 'pass', detail: result.detail }
}

/** Check every gate of a step; all must pass. */
export async function checkStepGates(
  gates: readonly StepGate[],
  context: GateContext,
  runProgram?: ProgramRunner,
): Promise<GateResult[]> {
  const results: GateResult[] = []
  for (const gate of gates) {
    results.push(await checkGate(gate, context, runProgram))
  }
  return results
}
