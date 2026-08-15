import { describe, expect, it } from 'vitest'
import { validateEntitySpec, type EntitySpec, type RegistryView } from '../src/entity-validate.ts'
import coordinatesDict from '../src/data/coordinates.json' with { type: 'json' }
import fkIndex from '../src/data/fk-index.json' with { type: 'json' }

/** Registry view mirroring the dev-seed snapshot (subset). */
const REGISTRY: RegistryView = {
  collections: new Map([
    ['zc_id_object', { name: '对象', inherits: [] }],
    ['zc_id_task', { name: '任务', inherits: ['zc_id_object'] }],
    ['zc_id_inventory', { name: '库存', inherits: ['zc_id_object'] }],
    ['zc_id_unit-currency', { name: '货币单位', inherits: ['zc_id_object'] }],
  ]),
}

const VALID_ENTITY: EntitySpec = {
  table: 'zc_id_deta-bill-check',
  name: '账单核查',
  inherits: ['zc_id_object'],
  coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
  fields: [
    { name: 'notice', category: 'scalar', dataType: 'text' },
    { name: 'biller', category: 'reference', dataType: 'bigint', reference: { targetTable: 'zc_id_subjects', localKey: 'fk_biller' } },
    { name: 'amount', category: 'reference', dataType: 'bigint', reference: { targetTable: 'zc_id_scal-amount', localKey: 'qk_amount' } },
  ],
}

describe('entity-validate snapshots', () => {
  it('coordinate dictionary is a real export with expected codes', () => {
    const scene = coordinatesDict.scene as readonly string[]
    const factor = coordinatesDict.factor as readonly string[]
    const func = coordinatesDict.function as readonly string[]
    expect(scene.length).toBeGreaterThan(50)
    expect(factor.length).toBeGreaterThan(50)
    expect(func.length).toBeGreaterThan(100)
    expect(scene).toContain('FE')
    expect(factor).toContain('GBA')
    expect(func).toContain('↑_AA')
  })

  it('fk index snapshot is a real export with physical local keys', () => {
    const refs = fkIndex.refs as unknown as readonly (readonly [string, string, string, string])[]
    expect(refs.length).toBeGreaterThan(1000)
    const billCheck = refs.find(([table, field]) => table === 'zc_id_deta-bill-check' && field === 'biller')
    expect(billCheck).toBeDefined()
  })
})

describe('entity-validate naming', () => {
  it('rejects table names outside the lowercase-hyphen charset', () => {
    const issues = validateEntitySpec({ ...VALID_ENTITY, table: 'Zc_id_purchase' }, REGISTRY)
    expect(issues.some(issue => issue.code === 'entity-name')).toBe(true)
  })

  it('rejects empty business names', () => {
    const issues = validateEntitySpec({ ...VALID_ENTITY, name: '' }, REGISTRY)
    expect(issues.some(issue => issue.code === 'entity-name')).toBe(true)
  })

  it('accepts a valid definition with no issues', () => {
    expect(validateEntitySpec(VALID_ENTITY, REGISTRY)).toEqual([])
  })
})

describe('entity-validate conflicts', () => {
  it('rejects an existing table and a duplicated business name', () => {
    const duplicate = validateEntitySpec({ ...VALID_ENTITY, table: 'zc_id_task' }, REGISTRY)
    expect(duplicate.some(issue => issue.code === 'collection-conflict')).toBe(true)

    const nameClash = validateEntitySpec({ ...VALID_ENTITY, name: '库存' }, REGISTRY)
    expect(nameClash.some(issue => issue.code === 'collection-name-conflict')).toBe(true)
  })
})

describe('entity-validate inheritance', () => {
  it('rejects unknown parents with the validator message shape', () => {
    const issues = validateEntitySpec({ ...VALID_ENTITY, inherits: ['zc_id_no-such'] }, REGISTRY)
    expect(issues).toContainEqual({ code: 'inherits-exists', message: 'parent class "zc_id_no-such" not found' })
  })

  it('rejects circular inheritance through the new entity', () => {
    const cyclic: EntitySpec = {
      ...VALID_ENTITY,
      inherits: ['zc_id_task'],
    }
    const registry: RegistryView = {
      collections: new Map([
        ['zc_id_object', { name: '对象', inherits: ['zc_id_deta-bill-check'] }],
        ['zc_id_task', { name: '任务', inherits: ['zc_id_object'] }],
      ]),
    }
    const issues = validateEntitySpec(cyclic, registry)
    expect(issues.some(issue => issue.code === 'inherits-circular')).toBe(true)
  })

  it('rejects inheritance deeper than the snapshot limit', () => {
    const registry: RegistryView = {
      collections: new Map([
        ['l0', { name: 'L0', inherits: [] }],
        ['l1', { name: 'L1', inherits: ['l0'] }],
        ['l2', { name: 'L2', inherits: ['l1'] }],
        ['l3', { name: 'L3', inherits: ['l2'] }],
        ['l4', { name: 'L4', inherits: ['l3'] }],
      ]),
    }
    const issues = validateEntitySpec({ ...VALID_ENTITY, table: 'zc_id_oper', inherits: ['l4'] }, registry)
    expect(issues).toContainEqual({ code: 'inherits-depth', message: 'inheritance depth 6 exceeds limit 5' })
  })
})

describe('entity-validate references', () => {
  it('rejects dangling target and junction tables', () => {
    const issues = validateEntitySpec({
      ...VALID_ENTITY,
      fields: [{
        name: 'fk_x',
        category: 'reference',
        dataType: 'bigint',
        reference: { targetTable: 'no_such', junctionTable: 'also_no_such' },
      }],
    }, REGISTRY)
    expect(issues.some(issue => issue.code === 'ref-target-exists')).toBe(true)
    expect(issues.some(issue => issue.code === 'ref-junction-exists')).toBe(true)
  })

  it('rejects local keys absent from the FK index snapshot', () => {
    const issues = validateEntitySpec({
      ...VALID_ENTITY,
      fields: [{
        name: 'fk_ghost',
        category: 'reference',
        dataType: 'bigint',
        reference: { targetTable: 'zc_id_inventory', localKey: 'fk_ghost' },
      }],
    }, REGISTRY)
    expect(issues.some(issue => issue.code === 'ref-local-key')).toBe(true)
  })

  it('accepts local keys present in the snapshot for the table', () => {
    const issues = validateEntitySpec({
      ...VALID_ENTITY,
      fields: [{
        name: 'biller',
        category: 'reference',
        dataType: 'bigint',
        reference: { targetTable: 'zc_id_subjects', localKey: 'fk_biller' },
      }],
    }, REGISTRY)
    expect(issues).toEqual([])
  })
})

describe('entity-validate coordinates', () => {
  it('rejects codes outside the real dictionary — no degradation', () => {
    const issues = validateEntitySpec({
      ...VALID_ENTITY,
      coordinates: { scene: 'XX', factor: 'ZZZ', function: '↓_QQ' },
    }, REGISTRY)
    expect(issues.some(issue => issue.code === 'coordinate-scene')).toBe(true)
    expect(issues.some(issue => issue.code === 'coordinate-factor')).toBe(true)
    expect(issues.some(issue => issue.code === 'coordinate-function')).toBe(true)
  })

  it('accepts codes from the real dictionary', () => {
    expect(validateEntitySpec(VALID_ENTITY, REGISTRY)).toEqual([])
  })
})
