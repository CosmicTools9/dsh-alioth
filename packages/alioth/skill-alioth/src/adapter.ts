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

/** A gate on one step. Either an output artifact glob or an external program. */
export type StepGate =
  | { readonly kind: 'output-glob'; readonly outputGlob: string }
  | { readonly kind: 'program'; readonly program: string; readonly args: readonly string[]; readonly outputGlob?: string }

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

function parseGate(value: unknown, context: string): StepGate {
  if (typeof value !== 'object' || value === null) {
    throw new Error(`skill-alioth: ${context} gate must be an object`)
  }
  const record = value as Record<string, unknown>
  // A gate declaring `program` is a program gate even when it also carries an
  // output_glob (the artifact the program produces).
  const program = record.program
  if (typeof program === 'string') {
    const args = [...asStringArray(record.args, `${context} gate args`)]
    const outputGlob = record.output_glob
    return {
      kind: 'program',
      program,
      args,
      ...(typeof outputGlob === 'string' ? { outputGlob } : {}),
    }
  }
  if (typeof record.output_glob === 'string') {
    return { kind: 'output-glob', outputGlob: record.output_glob }
  }
  throw new Error(`skill-alioth: ${context} gate must declare output_glob or program`)
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
    gates: rawGates.map((gate, gateIndex) => parseGate(gate, `step ${id} gate #${gateIndex}`)),
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
