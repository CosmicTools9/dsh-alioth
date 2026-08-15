/**
 * Entity-definition validation for the write path. Mirrors the vendored
 * validator semantics (parent-exists, no-circular-inheritance, depth) against
 * the bootstrapped registry, and — with no degradation — checks coordinates
 * and FK local keys against real dictionary snapshots (`coordinates.json`,
 * `fk-index.json`) exported from the AliothStudio dev DB, the same pattern as
 * AliothStudio's own `fk_index.rs`. Pure: registry data is injected, so tests
 * and the future write tool share one model.
 * @module @dsh-alioth/skill-alioth/entity-validate
 */

import coordinatesDict from './data/coordinates.json' with { type: 'json' }
import fkIndex from './data/fk-index.json' with { type: 'json' }
import physicalTables from './data/physical-tables.json' with { type: 'json' }

export interface ReferenceSpec {
  readonly targetTable: string
  /** Physical FK local column (reference_config.local_key), when the reference is physical. */
  readonly localKey?: string
  /** Junction table (reference_config.junction_table), when the reference is junction-based. */
  readonly junctionTable?: string
}

export interface FieldSpec {
  readonly name: string
  readonly category: 'scalar' | 'reference' | 'computed' | 'auto'
  readonly dataType: string
  /** Business label (meta_fields.title). */
  readonly title?: string
  /** NOT NULL flag (meta_fields.is_required). */
  readonly required?: boolean
  readonly reference?: ReferenceSpec
}

export interface CoordinatesSpec {
  readonly scene: string
  readonly factor: string
  readonly function: string
}

export interface EntitySpec {
  /** New collection table_name (e.g. `zc_id_inventory`). */
  readonly table: string
  readonly name: string
  readonly inherits: readonly string[]
  readonly coordinates?: CoordinatesSpec
  readonly fields: readonly FieldSpec[]
}

export interface ValidationIssue {
  readonly code: string
  readonly message: string
}

/** Existing-registry view needed by the validators. */
export interface RegistryView {
  readonly collections: ReadonlyMap<string, { readonly name: string; readonly inherits: readonly string[] }>
}

const TABLE_NAME_RE = /^[a-z][a-z0-9_-]*$/
const FIELD_NAME_RE = /^[a-z][a-z0-9_]*$/
/** Seed snapshot max inheritance depth (dev data: depth ≤ 4). */
const MAX_INHERITANCE_DEPTH = 5

/** Field name → declared physical local keys, from the FK index snapshot. */
const LOCAL_KEYS_BY_TABLE: ReadonlyMap<string, ReadonlySet<string>> = (() => {
  const map = new Map<string, Set<string>>()
  const refs = fkIndex.refs as unknown as readonly (readonly [string, string, string, string])[]
  for (const [table, , , localKey] of refs) {
    const set = map.get(table)
    if (set === undefined) {
      map.set(table, new Set([localKey]))
    } else {
      set.add(localKey)
    }
  }
  return map
})()

const SCENE_CODES = new Set(coordinatesDict.scene as readonly string[])
const FACTOR_CODES = new Set(coordinatesDict.factor as readonly string[])
const FUNCTION_CODES = new Set(coordinatesDict.function as readonly string[])

/** Physical isahl tables ([table, parent]); new entities must map onto one. */
const PHYSICAL_TABLES = new Set((physicalTables.tables as unknown as readonly (readonly [string, string])[]).map(([table]) => table))
/** Root-family common columns every lifecycle table inherits. */
const ROOT_COLUMNS = new Set(physicalTables.root_columns as readonly string[])

function issue(code: string, message: string): ValidationIssue {
  return { code, message }
}

function validateName(table: string, name: string, fields: readonly FieldSpec[]): ValidationIssue[] {
  const issues: ValidationIssue[] = []
  if (!TABLE_NAME_RE.test(table)) {
    issues.push(issue('entity-name', `table name ${JSON.stringify(table)} must match ^[a-z][a-z0-9-]*$`))
  }
  if (name.length === 0) {
    issues.push(issue('entity-name', `entity name must not be empty`))
  }
  for (const field of fields) {
    if (!FIELD_NAME_RE.test(field.name)) {
      issues.push(issue('field-name', `field name ${JSON.stringify(field.name)} must match ^[a-z][a-z0-9_]*$`))
    }
  }
  return issues
}

function validateInherits(spec: EntitySpec, registry: RegistryView): ValidationIssue[] {
  const issues: ValidationIssue[] = []
  const all = new Map(registry.collections)
  all.set(spec.table, { name: spec.name, inherits: spec.inherits })
  for (const parent of spec.inherits) {
    // Parents must exist either as registered entities or as physical tables.
    if (!all.has(parent) && !PHYSICAL_TABLES.has(parent)) {
      issues.push(issue('inherits-exists', `parent class ${JSON.stringify(parent)} not found`))
    }
  }
  // DFS cycle check over the combined graph (new entity included).
  const visiting = new Set<string>()
  const visited = new Set<string>()
  const walk = (node: string, path: string[]): void => {
    if (visiting.has(node)) {
      issues.push(issue('inherits-circular', `circular inheritance: ${[...path, node].join(' -> ')}`))
      return
    }
    if (visited.has(node)) {
      return
    }
    visiting.add(node)
    for (const parent of all.get(node)?.inherits ?? []) {
      walk(parent, [...path, node])
    }
    visiting.delete(node)
    visited.add(node)
  }
  walk(spec.table, [])
  // Depth limit: longest chain from the new entity. Cycle-safe: the circular
  // check above already reported cycles, so cut recursion on revisits.
  const depthMemo = new Map<string, number>()
  const depthOf = (node: string): number => {
    const memo = depthMemo.get(node)
    if (memo !== undefined) {
      return memo
    }
    depthMemo.set(node, 0) // mark visiting; a revisit returns 0 (cut)
    const parents = all.get(node)?.inherits ?? []
    const depth = parents.length === 0 ? 1 : 1 + Math.max(...parents.map(depthOf))
    depthMemo.set(node, depth)
    return depth
  }
  const depth = depthOf(spec.table)
  if (depth > MAX_INHERITANCE_DEPTH) {
    issues.push(issue('inherits-depth', `inheritance depth ${depth} exceeds limit ${MAX_INHERITANCE_DEPTH}`))
  }
  return issues
}

function validateReferences(spec: EntitySpec, registry: RegistryView): ValidationIssue[] {
  const issues: ValidationIssue[] = []
  // Targets may be registered entities or any physical table (FK index targets
  // reference isahl tables; the registry snapshot may lag the physical tree).
  const tables = new Set([...registry.collections.keys(), ...PHYSICAL_TABLES])
  for (const field of spec.fields) {
    const reference = field.reference
    if (reference === undefined) {
      continue
    }
    if (!tables.has(reference.targetTable)) {
      issues.push(issue('ref-target-exists', `field ${field.name}: target table ${JSON.stringify(reference.targetTable)} not found`))
    }
    if (reference.junctionTable !== undefined && !tables.has(reference.junctionTable)) {
      issues.push(issue('ref-junction-exists', `field ${field.name}: junction table ${JSON.stringify(reference.junctionTable)} not found`))
    }
    if (reference.localKey !== undefined && reference.localKey.length > 0) {
      const known = LOCAL_KEYS_BY_TABLE.get(spec.table)
      const ok = ROOT_COLUMNS.has(reference.localKey) || known?.has(reference.localKey) === true
      if (!ok) {
        issues.push(issue(
          'ref-local-key',
          `field ${field.name}: local_key ${JSON.stringify(reference.localKey)} is neither a common lifecycle column `
          + `nor a declared physical reference column of ${spec.table} (per the FK index snapshot; `
          + `regenerate with scripts/generate-coord-dict.sh after upstream sync)`,
        ))
      }
    }
  }
  return issues
}

function validateCoordinates(spec: EntitySpec): ValidationIssue[] {
  const coordinates = spec.coordinates
  if (coordinates === undefined) {
    return []
  }
  const issues: ValidationIssue[] = []
  if (!SCENE_CODES.has(coordinates.scene)) {
    issues.push(issue('coordinate-scene', `scene code ${JSON.stringify(coordinates.scene)} not in the coordinate dictionary`))
  }
  if (!FACTOR_CODES.has(coordinates.factor)) {
    issues.push(issue('coordinate-factor', `factor code ${JSON.stringify(coordinates.factor)} not in the coordinate dictionary`))
  }
  if (!FUNCTION_CODES.has(coordinates.function)) {
    issues.push(issue('coordinate-function', `function code ${JSON.stringify(coordinates.function)} not in the coordinate dictionary`))
  }
  return issues
}

function validateConflicts(spec: EntitySpec, registry: RegistryView): ValidationIssue[] {
  const issues: ValidationIssue[] = []
  const existing = registry.collections.get(spec.table)
  if (existing !== undefined) {
    issues.push(issue('collection-conflict', `collection ${JSON.stringify(spec.table)} already exists`))
    return issues
  }
  // isahl forbids CREATE TABLE: the entity must map onto an existing physical table.
  if (!PHYSICAL_TABLES.has(spec.table)) {
    issues.push(issue('physical-table', `table ${JSON.stringify(spec.table)} is not an isahl physical table — isahl forbids CREATE TABLE; pick an existing unregistered table (list via schema_info)`))
    return issues
  }
  for (const [table, collection] of registry.collections) {
    if (collection.name === spec.name) {
      issues.push(issue('collection-name-conflict', `entity name ${JSON.stringify(spec.name)} already used by ${table}`))
    }
  }
  return issues
}

/**
 * Validate a new-entity definition. All issues are hard failures — the write
 * must not proceed with any. Order: naming → conflicts → inheritance →
 * references → coordinates.
 */
export function validateEntitySpec(spec: EntitySpec, registry: RegistryView): readonly ValidationIssue[] {
  return [
    ...validateName(spec.table, spec.name, spec.fields),
    ...validateConflicts(spec, registry),
    ...validateInherits(spec, registry),
    ...validateReferences(spec, registry),
    ...validateCoordinates(spec),
  ]
}
