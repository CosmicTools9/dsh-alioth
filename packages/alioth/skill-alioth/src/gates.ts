/**
 * Gate execution: the adapter's step gates are the acceptance checks that let
 * a step complete. Mirrors the upstream contract (`skills/mod.rs` GateResult
 * Pass/NotAttempted/Fail, `GateErrorKind`; `dialog_tools/run_skill.rs`
 * `check_output_glob`): output-glob verifies artifacts exist; program gates run
 * an external checker and compare the exit code against `expected_exit_code`,
 * optionally also verifying the `output_glob` artifact they must produce.
 *
 * A gate may additionally carry a **content predicate**
 * (`require_json_pointer` + `require_json_equals`): the newest-mtime file among
 * the glob hits is parsed and its RFC 6901 pointer compared against the expected
 * value — fail-closed (missing file / unparsable JSON / absent pointer /
 * unequal value all fail). Failures carry a structured `repair` contract
 * (`repair.ts`) so the caller reports a rule id, not raw terminal text.
 * Pure check functions — callers choose the runtime.
 * @module @dsh-alioth/skill-alioth/gates
 */

import { globSync, statSync } from 'node:fs'
import { access as accessPath, readFile } from 'node:fs/promises'
import path from 'node:path'
import type { StepGate } from './adapter.ts'
import { repairContractFor, type FailureKind, type RepairContract } from './repair.ts'

/** Upstream GateResult: Pass / NotAttempted (declared, not executed) / Fail. */
export type GateStatus = 'pass' | 'not-attempted' | 'fail'

/**
 * Local failure classification from a gate's evidence text — **internal** to this
 * module: it only feeds `programFailureKind` → the repair rule table (`repair.ts`).
 * Upstream deleted its same-named `GateErrorKind` (the wire contract is
 * `RepairClass` + `ruleId`), so this is deliberately NOT part of the public API and
 * MUST NOT be re-exported: one classification vocabulary, not two.
 * Precedence: runner refusals → environment paths → timeouts → default contract
 * (an exit-code mismatch is output quality by definition).
 */
type GateErrorKind = 'contract' | 'tool-whitelist' | 'path-missing' | 'other'

function classifyGateError(detail: string): GateErrorKind {
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
  /** Structured repair contract on failures (upstream `repair.rs`); absent on pass/not-attempted. */
  readonly repair?: RepairContract
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

/** One glob expansion: the resolution detail plus every matched artifact file. */
interface GlobResolution {
  readonly ok: boolean
  readonly detail: string
  readonly files: readonly string[]
}

function isFile(target: string): boolean {
  try {
    return statSync(target).isFile()
  } catch {
    return false
  }
}

async function expandGlob(glob: string, context: GateContext): Promise<GlobResolution> {
  const resolved = resolveTemplate(glob, context)
  // The adapter globs are `Pre-Proc/...`-style paths anchored at the repo; we
  // resolve them under preProcRoot when they start with the known prefix.
  const prefix = 'Pre-Proc/'
  const candidate = resolved.startsWith(prefix)
    ? path.join(context.preProcRoot, resolved.slice(prefix.length))
    : path.resolve(context.preProcRoot, resolved)
  if (!withinRoot(candidate, context.preProcRoot)) {
    return { ok: false, detail: `glob escapes preProcRoot: ${resolved}`, files: [] }
  }
  // Glob patterns (a-v*.html) match against the artifact tree; concrete
  // paths stay a plain existence check.
  if (/[*?[]/.test(candidate)) {
    const matches = globSync(candidate).filter(isFile)
    return matches.length > 0
      ? {
        ok: true,
        detail: `matches: ${resolved} (${matches.length} file${matches.length === 1 ? '' : 's'})`,
        files: matches,
      }
      : { ok: false, detail: `no match for glob: ${resolved}`, files: [] }
  }
  try {
    await accessPath(candidate)
    return { ok: true, detail: `exists: ${resolved}`, files: [candidate] }
  } catch {
    return { ok: false, detail: `missing: ${resolved}`, files: [] }
  }
}

/** Newest-mtime file among the glob hits (upstream: the just-produced report is the evidence). */
function newestByMtime(files: readonly string[]): string | undefined {
  let newest: string | undefined
  let newestMs = Number.NEGATIVE_INFINITY
  for (const file of files) {
    let mtimeMs: number
    try {
      mtimeMs = statSync(file).mtimeMs
    } catch {
      continue
    }
    if (mtimeMs >= newestMs) {
      newest = file
      newestMs = mtimeMs
    }
  }
  return newest
}

/** RFC 6901 pointer lookup (upstream `serde_json::Value::pointer`). */
function resolveJsonPointer(doc: unknown, pointer: string): { found: boolean; value?: unknown } {
  if (pointer === '') return { found: true, value: doc }
  if (!pointer.startsWith('/')) return { found: false }
  let current: unknown = doc
  for (const raw of pointer.slice(1).split('/')) {
    const segment = raw.replace(/~1/g, '/').replace(/~0/g, '~')
    if (Array.isArray(current)) {
      // RFC 6901: array indices are digits without leading zeros; `-` (past the
      // end) never resolves to a value.
      if (!/^(0|[1-9]\d*)$/.test(segment)) return { found: false }
      const index = Number(segment)
      if (index >= current.length) return { found: false }
      current = current[index]
      continue
    }
    if (typeof current === 'object' && current !== null && Object.hasOwn(current, segment)) {
      current = (current as Record<string, unknown>)[segment]
      continue
    }
    return { found: false }
  }
  return { found: true, value: current }
}

/** Deep equality of two parsed JSON values (upstream compares `serde_json::Value`s). */
function jsonEquals(left: unknown, right: unknown): boolean {
  if (left === right) return true
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false
    return left.every((entry, index) => jsonEquals(entry, right[index]))
  }
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) return false
  const leftRecord = left as Record<string, unknown>
  const rightRecord = right as Record<string, unknown>
  const leftKeys = Object.keys(leftRecord)
  if (leftKeys.length !== Object.keys(rightRecord).length) return false
  return leftKeys.every(
    key => Object.hasOwn(rightRecord, key) && jsonEquals(leftRecord[key], rightRecord[key]),
  )
}

/**
 * Evaluate a gate's content predicate (upstream `add-step-gate-json-predicate`):
 * the **newest-mtime** file among the glob hits is parsed and its RFC 6901
 * pointer compared against the expected value. Missing file, unparsable JSON,
 * absent pointer and unequal value all FAIL (fail-closed) — an artifact
 * existing is *not* evidence that its content is acceptable.
 */
async function checkJsonPredicate(gate: StepGate, files: readonly string[]): Promise<GlobResolution> {
  const pointer = gate.requireJsonPointer
  const expected = gate.requireJsonEquals
  if (pointer === undefined || expected === undefined) {
    return { ok: true, detail: 'no content predicate', files: [] }
  }
  const newest = newestByMtime(files)
  if (newest === undefined) {
    return { ok: false, detail: `gate require_json 无文件可判定: ${gate.outputGlob ?? ''}`, files: [] }
  }
  let text: string
  try {
    text = await readFile(newest, 'utf8')
  } catch (error) {
    return { ok: false, detail: `gate require_json 读取失败 ${newest}: ${describe(error)}`, files: [] }
  }
  let doc: unknown
  try {
    doc = JSON.parse(text)
  } catch (error) {
    return { ok: false, detail: `gate require_json 解析失败 ${newest}: ${describe(error)}`, files: [] }
  }
  const lookup = resolveJsonPointer(doc, pointer)
  if (!lookup.found) {
    return { ok: false, detail: `gate require_json pointer '${pointer}' 缺失于 ${newest}`, files: [] }
  }
  if (!jsonEquals(lookup.value, expected)) {
    return {
      ok: false,
      detail: `gate require_json 不满足 ${newest}: pointer '${pointer}'（期望 ${JSON.stringify(expected)} 实际 ${JSON.stringify(lookup.value) ?? 'undefined'}）`,
      files: [],
    }
  }
  return { ok: true, detail: `require_json ${pointer} == ${JSON.stringify(expected)} @ ${newest}`, files: [] }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** Classify a program-gate failure into the structured repair vocabulary. */
function programFailureKind(detail: string): FailureKind {
  const kind = classifyGateError(detail)
  if (kind === 'tool-whitelist' || kind === 'path-missing') return 'gate-missing-program'
  if (/timeout|timed out/.test(detail.toLowerCase())) return 'gate-timeout'
  return 'gate-exit'
}

/** Build a failed gate result: detail + structured repair contract. */
function failed(gate: StepGate, failure: FailureKind, source: string, detail: string): GateResult {
  return {
    gate,
    status: 'fail',
    detail,
    repair: repairContractFor(failure, source, detail),
  }
}

/**
 * Check one gate, in upstream order: program (when declared) → glob existence →
 * content predicate. Program gates without a runner are `not-attempted` (never
 * silently passed) — execution belongs to the deployment (bun/node
 * availability, process policy).
 */
export async function checkGate(
  gate: StepGate,
  context: GateContext,
  runProgram?: ProgramRunner,
): Promise<GateResult> {
  if (gate.kind === 'output-glob') {
    const resolved = await expandGlob(gate.outputGlob, context)
    if (!resolved.ok) return failed(gate, 'gate-output-missing', '', resolved.detail)
    const predicate = await checkJsonPredicate(gate, resolved.files)
    if (!predicate.ok) return failed(gate, 'gate-json-predicate', '', predicate.detail)
    return {
      gate,
      status: 'pass',
      detail: predicate.detail === 'no content predicate' ? resolved.detail : `${resolved.detail}; ${predicate.detail}`,
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
    return failed(gate, programFailureKind(result.detail), gate.program, result.detail)
  }
  if (result.exitCode !== gate.expectedExitCode) {
    const detail = `${result.detail} (expected exit ${gate.expectedExitCode})`
    return failed(gate, programFailureKind(detail), gate.program, detail)
  }
  if (gate.outputGlob !== undefined) {
    // Program passed: the artifact it must produce is verified too
    // (upstream: `output_glob` on a program gate is checked after execution).
    const glob = await expandGlob(gate.outputGlob, context)
    if (!glob.ok) return failed(gate, 'gate-output-missing', '', glob.detail)
    const predicate = await checkJsonPredicate(gate, glob.files)
    if (!predicate.ok) return failed(gate, 'gate-json-predicate', '', predicate.detail)
    const detail = predicate.detail === 'no content predicate' ? `${result.detail}; ${glob.detail}` : `${result.detail}; ${glob.detail}; ${predicate.detail}`
    return { gate, status: 'pass', detail }
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
