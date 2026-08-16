/**
 * Artifact contract validation. The four JSON Schemas in `contracts/` are
 * hand-written from licensed-distribution evidence (golden app mirror,
 * AppCreator `module.json`, vendored `service.json` fixture, model spec rules)
 * and validated by the dependency-free engine in `validate-engine.ts`.
 *
 * Schema sources are injectable: when the model distribution later publishes
 * `_schema/*.schema.json`, callers can pass their own schema set to
 * `validateArtifactWith` without touching the built-in defaults.
 * @module @dsh-alioth/gen-alioth/validate
 */

import appSchema from './contracts/app.schema.json' with { type: 'json' }
import moduleSchema from './contracts/module.schema.json' with { type: 'json' }
import blockSchema from './contracts/block.schema.json' with { type: 'json' }
import serviceSchema from './contracts/service.schema.json' with { type: 'json' }
import { validateAgainstSchema, type Schema } from './validate-engine.ts'

/** The four artifact kinds. */
export type ArtifactKind = 'app' | 'module' | 'block' | 'service'

/** One schema set for all artifact kinds (swap source without touching callers). */
export type ArtifactSchemas = Readonly<Record<ArtifactKind, Schema>>

const builtinSchemas: ArtifactSchemas = {
  // JSON imports widen `type` to `string`; the engine's subset typing needs the literal.
  app: appSchema as unknown as Schema,
  module: moduleSchema as unknown as Schema,
  block: blockSchema as unknown as Schema,
  service: serviceSchema as unknown as Schema,
}

export interface ValidationResult {
  readonly valid: boolean
  readonly errors: readonly string[]
}

/** Validate `value` against a named artifact contract of the given schema set. */
export function validateArtifactWith(schemas: ArtifactSchemas, kind: ArtifactKind, value: unknown): ValidationResult {
  const errors = validateAgainstSchema(value, schemas[kind])
  return { valid: errors.length === 0, errors }
}

/** Validate `value` against the built-in artifact contracts. */
export function validateArtifact(kind: ArtifactKind, value: unknown): ValidationResult {
  return validateArtifactWith(builtinSchemas, kind, value)
}
