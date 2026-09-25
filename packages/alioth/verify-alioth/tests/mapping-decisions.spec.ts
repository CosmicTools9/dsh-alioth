import { describe, expect, it } from 'vitest'
import { ACCEPT_SCORE, decideMappingVerdict } from '../src/mapping-decisions.ts'
import type { MappingVerdict } from '../src/mapping-verdicts.ts'

/** 上游 `decide_verdict` 的同形：域精确匹配 + 置信门槛 + 表存在性（目录不可用豁免）。 */
const verdict = (over: Partial<MappingVerdict> = {}): MappingVerdict => ({
  namespace: 'U-x',
  domain: 'material_plan',
  table: 'zc_id_unit',
  verdictText: '',
  source: 'user',
  confidence: 1,
  recordedAt: '2026-09-25T00:00:00.000Z',
  ...over,
})

const CATALOG = { collections: ['zc_id_unit'], lifecycleEntities: ['zc_id_bill'] }

describe('decideMappingVerdict', () => {
  it('decides the domain from an exact-match verdict', () => {
    const { decision, ignored } = decideMappingVerdict('material_plan', [verdict()], CATALOG)
    expect(decision?.table).toBe('zc_id_unit')
    expect(decision?.confidence).toBe(1)
    expect(ignored).toEqual([])
  })

  it('does not decide when no verdict matches the domain exactly', () => {
    const { decision, ignored } = decideMappingVerdict('material_plan', [verdict({ domain: 'billing' })], CATALOG)
    expect(decision).toBeNull()
    expect(ignored).toEqual([])
  })

  it('ignores a low-confidence precedent with the reason attached', () => {
    const low = verdict({ confidence: ACCEPT_SCORE - 0.1 })
    const { decision, ignored } = decideMappingVerdict('material_plan', [low], CATALOG)
    expect(decision).toBeNull()
    expect(ignored).toEqual([{ domain: 'material_plan', table: 'zc_id_unit', reason: '低置信', confidence: ACCEPT_SCORE - 0.1 }])
  })

  it('ignores a stale table when the catalog is usable', () => {
    const { decision, ignored } = decideMappingVerdict('material_plan', [verdict({ table: 'zc_id_retired' })], CATALOG)
    expect(decision).toBeNull()
    expect(ignored).toEqual([{ domain: 'material_plan', table: 'zc_id_retired', reason: '表不存在', confidence: 1 }])
  })

  it('exempts existence checks when the catalog cannot be read', () => {
    // 目录不可用 ⇒ 不能拿「查不到」当否决证据（上游同判）。
    expect(decideMappingVerdict('material_plan', [verdict()], null).decision?.table).toBe('zc_id_unit')
    expect(decideMappingVerdict('material_plan', [verdict()], { collections: [], lifecycleEntities: [] }).decision?.table).toBe('zc_id_unit')
  })

  it('accepts a lifecycle entity as a table', () => {
    expect(decideMappingVerdict('bill', [verdict({ domain: 'bill', table: 'zc_id_bill' })], CATALOG).decision?.table).toBe('zc_id_bill')
  })
})
