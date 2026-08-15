/**
 * Minimal JSON Schema (draft-07 subset) validator. Deliberately dependency-free:
 * the artifact contracts in `contracts/` use only this subset (type, required,
 * properties, additionalProperties, items, pattern, enum, min/maxLength,
 * integer), so no general engine (ajv) is warranted. Errors carry JSON pointers.
 * @module @dsh-alioth/gen-alioth/validate-engine
 */

export type Schema =
  | {
    type?: 'object'
    required?: readonly string[]
    properties?: Readonly<Record<string, Schema>>
    additionalProperties?: boolean
    enum?: readonly unknown[]
  }
  | {
    type?: 'array'
    items?: Schema
    enum?: readonly unknown[]
  }
  | {
    type?: 'string'
    pattern?: string
    minLength?: number
    maxLength?: number
    enum?: readonly unknown[]
  }
  | {
    type?: 'number' | 'integer' | 'boolean' | 'null'
    enum?: readonly unknown[]
  }

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function checkValue(value: unknown, schema: Schema, path: string, errors: string[]): void {
  if (schema.enum !== undefined && !schema.enum.some(entry => entry === value)) {
    errors.push(`${path}: value not in enum`)
    return
  }
  switch (schema.type) {
    case 'object': {
      if (!isObject(value)) {
        errors.push(`${path}: expected object`)
        return
      }
      for (const key of schema.required ?? []) {
        if (!(key in value)) {
          errors.push(`${path}: missing required property ${JSON.stringify(key)}`)
        }
      }
      for (const [key, propertySchema] of Object.entries(schema.properties ?? {})) {
        if (key in value) {
          checkValue(value[key], propertySchema, `${path}/${key}`, errors)
        }
      }
      if (schema.additionalProperties === false) {
        for (const key of Object.keys(value)) {
          if (!(schema.properties !== undefined && key in schema.properties)) {
            errors.push(`${path}: unexpected property ${JSON.stringify(key)}`)
          }
        }
      }
      break
    }
    case 'array': {
      if (!Array.isArray(value)) {
        errors.push(`${path}: expected array`)
        return
      }
      if (schema.items !== undefined) {
        const items = schema.items
        value.forEach((entry, index) => checkValue(entry, items, `${path}/${index}`, errors))
      }
      break
    }
    case 'string': {
      if (typeof value !== 'string') {
        errors.push(`${path}: expected string`)
        return
      }
      if (schema.pattern !== undefined && !new RegExp(schema.pattern).test(value)) {
        errors.push(`${path}: does not match pattern ${schema.pattern}`)
      }
      if (schema.minLength !== undefined && value.length < schema.minLength) {
        errors.push(`${path}: shorter than minLength ${schema.minLength}`)
      }
      if (schema.maxLength !== undefined && value.length > schema.maxLength) {
        errors.push(`${path}: longer than maxLength ${schema.maxLength}`)
      }
      break
    }
    case 'integer': {
      if (!Number.isInteger(value)) {
        errors.push(`${path}: expected integer`)
      }
      break
    }
    case 'number': {
      if (typeof value !== 'number') {
        errors.push(`${path}: expected number`)
      }
      break
    }
    case 'boolean': {
      if (typeof value !== 'boolean') {
        errors.push(`${path}: expected boolean`)
      }
      break
    }
    case 'null': {
      if (value !== null) {
        errors.push(`${path}: expected null`)
      }
      break
    }
    default: {
      // No `type` keyword: any value passes (only enum applies).
    }
  }
}

/** Validate `value` against a draft-07 subset schema; returns JSON-pointer errors. */
export function validateAgainstSchema(value: unknown, schema: Schema): readonly string[] {
  const errors: string[] = []
  checkValue(value, schema, '/', errors)
  return errors
}
