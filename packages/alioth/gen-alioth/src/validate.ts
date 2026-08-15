/**
 * Artifact contract validation. The four JSON Schemas in `contracts/` are
 * hand-written from licensed-distribution evidence (golden app mirror,
 * AppCreator `module.json`, vendored `service.json` fixture, model spec rules)
 * and validated by the dependency-free engine in `validate-engine.ts`. When
 * the model distribution later publishes `_schema/*.schema.json`, swap the
 * schema sources here without touching callers.
 * @module @dsh-alioth/gen-alioth/validate
 */

import appSchema from './contracts/app.schema.json' with { type: 'json' }
import moduleSchema from './contracts/module.schema.json' with { type: 'json' }
import blockSchema from './contracts/block.schema.json' with { type: 'json' }
import serviceSchema from './contracts/service.schema.json' with { type: 'json' }
import { validateAgainstSchema, type Schema } from './validate-engine.ts'

const schemas = {
  // JSON imports widen `type` to `string`; the engine's subset typing needs the literal.
  app: appSchema as unknown as Schema,
  module: moduleSchema as unknown as Schema,
  block: blockSchema as unknown as Schema,
  service: serviceSchema as unknown as Schema,
} as const

export type ArtifactKind = keyof typeof schemas

export interface ValidationResult {
  readonly valid: boolean
  readonly errors: readonly string[]
}

/** Validate `value` against the named artifact contract. */
export function validateArtifact(kind: ArtifactKind, value: unknown): ValidationResult {
  const errors = validateAgainstSchema(value, schemas[kind])
  return { valid: errors.length === 0, errors }
}
