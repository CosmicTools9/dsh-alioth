/**
 * 构建回归基线：任务集结构校验、证据缺失的 degraded、tolerance 回归裁决。
 * 〔契约 §4/T3 必需测试〕
 * @module @dsh-alioth/verify-alioth/tests/eval-baseline
 */

import { describe, expect, it } from 'vitest'
import { decideRegression, parseEvalCases, scoreRun, type EvalCaseSet } from '../src/eval-baseline.ts'

const VALID_CASES_YAML = `schema: appagent-build-eval/v1
threshold: 0.8
tolerance: 0.05
cases:
  - id: case-a
    namespace: TestNS
    app: tm-app-a
    goal: 建一个订单应用
    weight: 0.6
    dimensions:
      extension_verify: 0.5
      artifacts: 0.5
  - id: case-b
    namespace: TestNS
    app: tm-app-b
    goal: 建一个库存应用
    weight: 0.4
    dimensions:
      e2e: 1.0
`

describe('parseEvalCases', () => {
  it('合法 YAML → 结构化任务集', () => {
    const caseSet = parseEvalCases(VALID_CASES_YAML)
    expect(caseSet.schema).toBe('appagent-build-eval/v1')
    expect(caseSet.threshold).toBe(0.8)
    expect(caseSet.tolerance).toBe(0.05)
    expect(caseSet.cases.map(entry => entry.id)).toEqual(['case-a', 'case-b'])
  })

  it('结构非法即 throw 且给出定位', () => {
    expect(() => parseEvalCases({ ...(parseEvalCases(VALID_CASES_YAML) as object), schema: 'other/v9' })).toThrow(
      /@schema/,
    )
    expect(() =>
      parseEvalCases(`schema: appagent-build-eval/v1
threshold: 1.5
tolerance: 0
cases:
  - {id: a, namespace: n, app: p, goal: g, weight: 1, dimensions: {artifacts: 1}}
`),
    ).toThrow(/@threshold/)

    const unknownDimension = VALID_CASES_YAML.replace('artifacts: 0.5', 'custom_dimension: 0.5')
    expect(() => parseEvalCases(unknownDimension)).toThrow(/@cases\[0\]\.dimensions\.custom_dimension/)

    const duplicated = VALID_CASES_YAML.replace('id: case-b', 'id: case-a')
    expect(() => parseEvalCases(duplicated)).toThrow(/id 重复/)

    const badWeight = VALID_CASES_YAML.replace('weight: 0.4', 'weight: 0.9')
    expect(() => parseEvalCases(badWeight)).toThrow(/权重合计/)

    const badDimensionSum = VALID_CASES_YAML.replace('extension_verify: 0.5', 'extension_verify: 0.9')
    expect(() => parseEvalCases(badDimensionSum)).toThrow(/@cases\[0\]\.dimensions/)

    expect(() => parseEvalCases('not: [a yaml')).toThrow(/YAML 解析失败/)
  })
})

describe('scoreRun — 证据缺失 → 0 分 + degraded', () => {
  const caseSet: EvalCaseSet = parseEvalCases(VALID_CASES_YAML)

  it('全部维度有证据 → 加权满分且 degraded=false', () => {
    const result = scoreRun({
      caseSet,
      evidence: {
        'case-a': { extension_verify: 1, artifacts: 1 },
        'case-b': { e2e: 1 },
      },
    })
    expect(result.score).toBe(1)
    expect(result.perCase).toEqual({ 'case-a': 1, 'case-b': 1 })
    expect(result.degraded).toBe(false)
  })

  it('声明维度缺证据 → 该维度 0 分、整轮 degraded（MUST NOT 按满分/按通过处理）', () => {
    const result = scoreRun({
      caseSet,
      evidence: {
        'case-a': { extension_verify: 1 },
        'case-b': { e2e: 1 },
      },
    })
    expect(result.perCase['case-a']).toBe(0.5)
    expect(result.score).toBeCloseTo(0.7, 6)
    expect(result.degraded).toBe(true)
  })

  it('整 case 无证据记录 → 该 case 0 分且 degraded', () => {
    const result = scoreRun({ caseSet, evidence: {} })
    expect(result.score).toBe(0)
    expect(result.degraded).toBe(true)
  })

  it('越界/非数值证据视同缺失（fail-closed，不得当满分）', () => {
    const result = scoreRun({
      caseSet,
      evidence: { 'case-a': { extension_verify: 2, artifacts: Number.NaN }, 'case-b': { e2e: 1 } },
    })
    expect(result.perCase['case-a']).toBe(0)
    expect(result.degraded).toBe(true)
  })
})

describe('decideRegression — tolerance 判据', () => {
  const caseSet: EvalCaseSet = parseEvalCases(VALID_CASES_YAML)
  const baseline = { 'case-a': 1, 'case-b': 1 }

  it('逐 case 降幅在 tolerance 内且 overall ≥ threshold → pass', () => {
    const verdict = decideRegression({
      baseline,
      current: { 'case-a': 0.97, 'case-b': 0.96 },
      caseSet,
    })
    expect(verdict.verdict).toBe('pass')
    expect(verdict.detail).toContain('tolerance')
  })

  it('单 case 降幅超出 tolerance（overall 仍达标）→ regressed 且载明降幅', () => {
    const verdict = decideRegression({
      baseline,
      current: { 'case-a': 0.9, 'case-b': 1 },
      caseSet,
    })
    expect(verdict.verdict).toBe('regressed')
    expect(verdict.detail).toContain('case case-a 降幅')
  })

  it('overall 低于 threshold → regressed', () => {
    const verdict = decideRegression({
      baseline,
      current: { 'case-a': 0.7, 'case-b': 0.7 },
      caseSet,
    })
    expect(verdict.verdict).toBe('regressed')
    expect(verdict.detail).toContain('threshold')
  })

  it('缺本次评分的 case 按 0 计并记入 detail（fail-closed）', () => {
    const verdict = decideRegression({ baseline, current: { 'case-b': 1 }, caseSet })
    expect(verdict.verdict).toBe('regressed')
    expect(verdict.detail).toContain('无本次评分')
  })
})
