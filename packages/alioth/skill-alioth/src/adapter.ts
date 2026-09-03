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
 */
export type StepGate =
  | { readonly kind: 'output-glob'; readonly outputGlob: string }
  | {
    readonly kind: 'program'
    readonly program: string
    readonly args: readonly string[]
    readonly expectedExitCode: number
    readonly timeoutSec: number
    readonly outputGlob?: string
  }

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

function parseGate(value: unknown, context: string): StepGate {
  if (typeof value !== 'object' || value === null) {
    throw new Error(`skill-alioth: ${context} gate must be an object`)
  }
  const record = value as Record<string, unknown>
  const outputGlob = record.output_glob
  // Upstream StepGate: `program` empty/absent + output_glob = pure file
  // check; a non-empty program makes it a program gate (which may also
  // declare the artifact glob it must produce).
  const program = record.program
  if (typeof program === 'string' && program.length > 0) {
    return {
      kind: 'program',
      program,
      args: [...asStringArray(record.args, `${context} gate args`)],
      expectedExitCode: asInt(record.expected_exit_code, `${context} gate expected_exit_code`, 0),
      timeoutSec: asInt(record.timeout_sec, `${context} gate timeout_sec`, DEFAULT_GATE_TIMEOUT_SEC),
      ...(typeof outputGlob === 'string' ? { outputGlob } : {}),
    }
  }
  if (typeof outputGlob === 'string') {
    return { kind: 'output-glob', outputGlob }
  }
  throw new Error(`skill-alioth: ${context} gate must declare output_glob or program`)
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
  return {
    name,
    description: typeof record.description === 'string' ? record.description : '',
    version: typeof record.version === 'string' ? record.version : '',
    tracks: rawTracks.map((track, index) => parseTrack(track, index)),
    defaultTools: asStringArray(record.default_tools, `${sourceName} default_tools`),
    referencePaths: asStringArray(record.reference_paths, `${sourceName} reference_paths`),
  }
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
