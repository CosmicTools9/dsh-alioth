/**
 * Skill-adapter parsing: the model distribution's `skill-adapters/*.yaml`
 * define the AppAgent tracks/steps/gates as data. This module parses them
 * with a real YAML parser (no regex) into typed models, validating structure
 * loudly — a malformed adapter is a model-distribution defect worth surfacing.
 * @module @dsh-alioth/skill-alioth/adapter
 */

import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { parse as parseYaml } from 'yaml'

/**
 * A gate on one step (upstream `StepGate`, skills/mod.rs). Two forms:
 * - pure file check: `output_glob` only (`program` empty/absent upstream);
 * - program gate: `program` + `args` (+ optional `output_glob` the program
 *   must produce), with `expected_exit_code` (default 0) and `timeout_sec`
 *   (default 120).
 *
 * Both forms may carry a **content predicate** (upstream
 * `add-step-gate-json-predicate`): `require_json_pointer` (RFC 6901) +
 * `require_json_equals` (any JSON value) MUST be set together and MUST be
 * attached to an `output_glob` — the gate then evaluates the pointer against the
 * **newest-mtime** file among the glob hits, and a missing file / unparsable
 * JSON / absent pointer / unequal value all FAIL (fail-closed).
 */
export type StepGate =
  | {
    readonly kind: 'output-glob'
    readonly outputGlob: string
    readonly requireJsonPointer?: string
    readonly requireJsonEquals?: unknown
  }
  | {
    readonly kind: 'program'
    readonly program: string
    readonly args: readonly string[]
    readonly expectedExitCode: number
    readonly timeoutSec: number
    readonly outputGlob?: string
    readonly requireJsonPointer?: string
    readonly requireJsonEquals?: unknown
  }

/**
 * Step phase (upstream `add-step-plan-apply-phase`): `plan` produces *proposal*
 * artifacts only — its write surface narrows to the artifacts this step's
 * `output_glob` declares; `apply` lands them under the namespace sandbox alone.
 * Default `apply` keeps every pre-existing adapter's semantics unchanged.
 */
export type StepPhase = 'plan' | 'apply'

export interface StepSchema {
  readonly type: string
  readonly required: readonly string[]
}

export interface Step {
  readonly id: string
  readonly instruction: string
  readonly tools: readonly string[]
  readonly schema: StepSchema | undefined
  readonly gates: readonly StepGate[]
  /** Read-only reference asset paths — injected as path hints (upstream G3). */
  readonly referencePaths: readonly string[]
  /** Input files the engine reads and injects (templates `{ns}`/`{module}`). */
  readonly inputs: readonly string[]
  /** Step phase; `plan` steps only write their declared proposal artifacts. */
  readonly phase: StepPhase
}

export interface Track {
  readonly name: string
  readonly steps: readonly Step[]
}

export interface Adapter {
  readonly name: string
  readonly description: string
  readonly version: string
  readonly tracks: readonly Track[]
  readonly defaultTools: readonly string[]
  readonly referencePaths: readonly string[]
}

function asString(value: unknown, context: string): string {
  if (typeof value !== 'string') {
    throw new Error(`skill-alioth: ${context} must be a string`)
  }
  return value
}

function asStringArray(value: unknown, context: string): readonly string[] {
  if (value === undefined) {
    return []
  }
  if (!Array.isArray(value) || value.some(entry => typeof entry !== 'string')) {
    throw new Error(`skill-alioth: ${context} must be an array of strings`)
  }
  return value
}

const DEFAULT_GATE_TIMEOUT_SEC = 120

function asInt(value: unknown, context: string, fallback: number): number {
  if (value === undefined) {
    return fallback
  }
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    throw new Error(`skill-alioth: ${context} must be an integer`)
  }
  return value
}

/**
 * Keys a gate may carry (upstream `StepGate` fields). Anything else is a
 * gate form this build cannot evaluate: fail-closed at load time rather than
 * silently degrading to a bare existence check.
 */
const GATE_KEYS: Readonly<Record<string, true>> = {
  program: true,
  args: true,
  expected_exit_code: true,
  output_glob: true,
  require_json_pointer: true,
  require_json_equals: true,
  timeout_sec: true,
}

/** Step phase values; anything else is a hand-written adapter typo. */
const STEP_PHASES: Readonly<Record<string, true>> = { plan: true, apply: true }

/** Content predicate pair (both set or both absent — enforced at load time). */
interface GatePredicate {
  readonly requireJsonPointer?: string
  readonly requireJsonEquals?: unknown
}

/** Read the `require_json_*` pair, throwing on the "only one set" configuration error. */
function parsePredicate(record: Record<string, unknown>, context: string): GatePredicate {
  const pointer = record.require_json_pointer
  const equals = record.require_json_equals
  if (pointer !== undefined && typeof pointer !== 'string') {
    throw new Error(`skill-alioth: ${context} require_json_pointer must be a string`)
  }
  if ((pointer === undefined) !== (equals === undefined)) {
    throw new Error(
      `skill-alioth: ${context} require_json_pointer 与 require_json_equals 须同时设置（仅设其一）`,
    )
  }
  if (pointer === undefined) return {}
  return { requireJsonPointer: pointer, requireJsonEquals: equals }
}

function parseGate(value: unknown, context: string): StepGate {
  if (typeof value !== 'object' || value === null) {
    throw new Error(`skill-alioth: ${context} gate must be an object`)
  }
  const record = value as Record<string, unknown>
  for (const key of Object.keys(record)) {
    if (GATE_KEYS[key] === undefined) {
      throw new Error(
        `skill-alioth: ${context} unknown gate key '${key}' — fail-closed（本 build 不认识的 gate 形态不得静默降级为存在性检查）`,
      )
    }
  }
  const outputGlob = record.output_glob
  const predicate = parsePredicate(record, `${context} gate`)
  // Upstream StepGate: `program` empty/absent + output_glob = pure file
  // check; a non-empty program makes it a program gate (which may also
  // declare the artifact glob it must produce).
  const program = record.program
  if (typeof program === 'string' && program.length > 0) {
    if (predicate.requireJsonPointer !== undefined && typeof outputGlob !== 'string') {
      throw new Error(`skill-alioth: ${context} gate require_json_* 谓词须依附 output_glob（无产物可判定）`)
    }
    return {
      kind: 'program',
      program,
      args: [...asStringArray(record.args, `${context} gate args`)],
      expectedExitCode: asInt(record.expected_exit_code, `${context} gate expected_exit_code`, 0),
      timeoutSec: asInt(record.timeout_sec, `${context} gate timeout_sec`, DEFAULT_GATE_TIMEOUT_SEC),
      ...(typeof outputGlob === 'string' ? { outputGlob } : {}),
      ...predicate,
    }
  }
  if (typeof outputGlob === 'string') {
    return { kind: 'output-glob', outputGlob, ...predicate }
  }
  throw new Error(`skill-alioth: ${context} gate must declare output_glob or program`)
}

/** Parse a step's `phase`; absent means `apply` (existing adapters unchanged). */
function parsePhase(value: unknown, context: string): StepPhase {
  if (value === undefined) return 'apply'
  if (typeof value === 'string' && STEP_PHASES[value] === true) {
    return value === 'plan' ? 'plan' : 'apply'
  }
  throw new Error(`skill-alioth: ${context} must be 'plan' or 'apply'`)
}

/** Upstream `Skill::migrate_outputs_to_gates`: deprecated `outputs` become
 * output-glob gates when the step declares no gates of its own. */
function migrateOutputsToGates(record: Record<string, unknown>, gates: readonly StepGate[]): readonly StepGate[] {
  if (gates.length > 0 || !Array.isArray(record.outputs)) {
    return gates
  }
  return record.outputs
    .filter((entry): entry is string => typeof entry === 'string')
    .map(outputGlob => ({ kind: 'output-glob' as const, outputGlob }))
}

function parseStep(value: unknown, index: number): Step {
  if (typeof value !== 'object' || value === null) {
    throw new Error(`skill-alioth: step #${index} must be an object`)
  }
  const record = value as Record<string, unknown>
  const id = asString(record.id, `step #${index} id`)
  const instruction = asString(record.instruction, `step ${id} instruction`)
  const rawGates = Array.isArray(record.gates) ? record.gates : []
  const rawSchema = record.schema
  let schema: StepSchema | undefined
  if (rawSchema !== undefined) {
    const schemaRecord = rawSchema as Record<string, unknown>
    if (typeof rawSchema !== 'object' || rawSchema === null || schemaRecord.type !== 'object') {
      throw new Error(`skill-alioth: step ${id} schema must be an object schema`)
    }
    const required = asStringArray(schemaRecord.required, `step ${id} schema.required`)
    schema = { type: 'object', required }
  }
  return {
    id,
    instruction,
    tools: asStringArray(record.tools, `step ${id} tools`),
    schema,
    gates: migrateOutputsToGates(
      record,
      rawGates.map((gate, gateIndex) => parseGate(gate, `step ${id} gate #${gateIndex}`)),
    ),
    referencePaths: asStringArray(record.reference_paths, `step ${id} reference_paths`),
    inputs: asStringArray(record.inputs, `step ${id} inputs`),
    phase: parsePhase(record.phase, `step ${id} phase`),
  }
}

function parseTrack(value: unknown, index: number): Track {
  if (typeof value !== 'object' || value === null) {
    throw new Error(`skill-alioth: track #${index} must be an object`)
  }
  const record = value as Record<string, unknown>
  const name = asString(record.name, `track #${index} name`)
  const rawSteps = record.steps
  if (!Array.isArray(rawSteps)) {
    throw new Error(`skill-alioth: track ${name} steps must be an array`)
  }
  return { name, steps: rawSteps.map((step, stepIndex) => parseStep(step, stepIndex)) }
}

/**
 * Structural invariants of an adapter document (load-time fail-fast; upstream
 * `Skill::validate_gates` + `Skill::validate_phases`). Returns one message per
 * violation, `[]` when the document is valid:
 *
 * 1. a gate's content predicate MUST set `requireJsonPointer` and
 *    `requireJsonEquals` together, and MUST hang off an `output_glob`;
 * 2. a `plan` step MUST declare at least one gate `output_glob` — a proposal
 *    with no artifact slot cannot be reviewed;
 * 3. a `plan` step MUST have a later `apply` step in the same track — otherwise
 *    the proposal has no consumer (dead surface).
 *
 * Both phase invariants are *structural* errors (hand-written adapter typos):
 * failing at load time beats burning LLM retry rounds at execution time.
 */
export function validateAdapterStructure(adapter: Adapter): readonly string[] {
  const problems: string[] = []
  for (const track of adapter.tracks) {
    track.steps.forEach((step, index) => {
      for (const gate of step.gates) {
        const hasPointer = gate.requireJsonPointer !== undefined
        const hasEquals = gate.requireJsonEquals !== undefined
        if (hasPointer !== hasEquals) {
          problems.push(
            `track '${track.name}' step ${step.id}: require_json_pointer 与 require_json_equals 须同时设置（仅设其一）`,
          )
        }
        if ((hasPointer || hasEquals) && gate.outputGlob === undefined) {
          problems.push(`track '${track.name}' step ${step.id}: require_json_* 谓词须依附 output_glob（无产物可判定）`)
        }
      }
      if (step.phase !== 'plan') return
      if (!step.gates.some(gate => gate.outputGlob !== undefined)) {
        problems.push(
          `track '${track.name}' step ${step.id} 为 plan 步但未声明任何 gate.output_glob——方案必须有产物位`,
        )
      }
      if (!track.steps.slice(index + 1).some(later => later.phase !== 'plan')) {
        problems.push(`track '${track.name}' step ${step.id} 为 plan 步，但其后同 Track 内无 apply 步——方案无消费者`)
      }
    })
  }
  return problems
}

/** Parse one adapter document into a typed model; throws on malformed structure. */
export function parseAdapterDocument(source: string, sourceName: string): Adapter {
  let parsed: unknown
  try {
    parsed = parseYaml(source)
  } catch (error) {
    throw new Error(`skill-alioth: invalid YAML in ${sourceName}: ${error instanceof Error ? error.message : String(error)}`)
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new Error(`skill-alioth: ${sourceName} must be a mapping`)
  }
  const record = parsed as Record<string, unknown>
  const name = asString(record.name, `${sourceName} name`)
  const rawTracks = record.tracks
  if (!Array.isArray(rawTracks)) {
    throw new Error(`skill-alioth: ${sourceName} tracks must be an array`)
  }
  const adapter: Adapter = {
    name,
    description: typeof record.description === 'string' ? record.description : '',
    version: typeof record.version === 'string' ? record.version : '',
    tracks: rawTracks.map((track, index) => parseTrack(track, index)),
    defaultTools: asStringArray(record.default_tools, `${sourceName} default_tools`),
    referencePaths: asStringArray(record.reference_paths, `${sourceName} reference_paths`),
  }
  const problems = validateAdapterStructure(adapter)
  if (problems.length > 0) {
    throw new Error(`skill-alioth: ${sourceName} 结构非法:\n- ${problems.join('\n- ')}`)
  }
  return adapter
}

/** Read and parse one adapter file from a model snapshot. */
export async function loadAdapter(dir: string, fileName: string): Promise<Adapter> {
  const source = await readFile(path.join(dir, 'skill-adapters', fileName), 'utf8')
  return parseAdapterDocument(source, fileName)
}

/**
 * Parse the runtime program allowlist from the snapshot's `_runtime.yaml`
 * (RunCommandTool's `allowed_programs`). Missing/unreadable → empty list.
 */
export function parseRuntimeAllowedPrograms(content: string): string[] {
  try {
    const parsed = parseYaml(content) as { allowed_programs?: unknown } | null
    if (parsed === null || typeof parsed !== 'object' || !Array.isArray(parsed.allowed_programs)) {
      return []
    }
    return parsed.allowed_programs.filter((entry): entry is string => typeof entry === 'string')
  } catch {
    return []
  }
}
