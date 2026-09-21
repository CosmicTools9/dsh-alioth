/**
 * 构建回归基线（build-eval）—— 对齐上游 `scripts/eval/appagent-build-eval.ts`
 * 与 spec `appagent-build-eval-baseline`：
 *
 * - 任务集结构非法 → throw（含定位），MUST NOT 静默跑残缺任务集；
 * - 机械证据优先：维度权重和为 1 的白名单维度加权；**证据缺失/非法 → 该维度 0 分且整轮 degraded**
 *   （缺失 ≠ 满分、degraded ≠ 通过）；
 * - 回归裁决：overall < threshold 或任一 case 降幅 > tolerance ⇒ regressed。
 * @module @dsh-alioth/verify-alioth/eval-baseline
 */

import { parse as parseYaml } from 'yaml'

export const EVAL_BASELINE_SCHEMA = 'appagent-build-eval/v1'

/** 维度白名单（非法键即拒绝执行，防「自定义维度」绕过证据面）。 */
export const EVAL_DIMENSIONS: readonly string[] = [
  'extension_verify',
  'e2e',
  'closure_audit',
  'eval_report_rules',
  'artifacts',
]

const WEIGHT_SUM_TOLERANCE = 1e-6

/** 单个评测基准 case。 */
export interface EvalCase {
  readonly id: string
  readonly namespace: string
  readonly app: string
  readonly goal: string
  readonly weight: number
  readonly dimensions: Record<string, number>
  readonly rubricWeight?: number
}

/** 评测基准任务集。 */
export interface EvalCaseSet {
  readonly schema: string
  readonly threshold: number
  readonly tolerance: number
  readonly cases: readonly EvalCase[]
}

/** 单 case 每维度的机械证据分（0..1；缺失键 = 该维度未执行 → 0 分 + degraded）。 */
export interface EvalEvidence {
  readonly extension_verify?: number
  readonly e2e?: number
  readonly closure_audit?: number
  readonly eval_report_rules?: number
  readonly artifacts?: number
}

/** 结构非法即 throw（含定位：`<字段路径>：<原因>`）。 */
function reject(at: string, message: string): never {
  throw new Error(`基准任务集非法 @${at}：${message}`)
}

function describeValue(value: unknown): string {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  return typeof value
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

/**
 * 解析并校验任务集（接受对象或 YAML/JSON 字符串，一律走真解析器 `yaml`）。
 * 非法即 throw——含 schema / threshold / tolerance / case 字段 / 维度白名单 / 权重合计的定位。
 */
export function parseEvalCases(document: unknown): EvalCaseSet {
  let raw: unknown = document
  if (typeof document === 'string') {
    try {
      raw = parseYaml(document) as unknown
    } catch (error) {
      reject('document', `YAML 解析失败：${error instanceof Error ? error.message : String(error)}`)
    }
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    reject('document', `期望对象，实际 ${describeValue(raw)}`)
  }
  const root = raw as Record<string, unknown>

  if (root['schema'] !== EVAL_BASELINE_SCHEMA) {
    reject('schema', `须为 ${EVAL_BASELINE_SCHEMA}，实际 ${JSON.stringify(root['schema'])}`)
  }
  const threshold = root['threshold']
  if (!isFiniteNumber(threshold) || threshold <= 0 || threshold > 1) {
    reject('threshold', `须为 (0,1] 的数值，实际 ${JSON.stringify(threshold)}`)
  }
  const tolerance = root['tolerance']
  if (!isFiniteNumber(tolerance) || tolerance < 0) {
    reject('tolerance', `须为 ≥0 的数值，实际 ${JSON.stringify(tolerance)}`)
  }
  const rawCases = root['cases']
  if (!Array.isArray(rawCases) || rawCases.length === 0) {
    reject('cases', '须为非空数组')
  }

  const seen = new Set<string>()
  const cases: EvalCase[] = []
  let weightTotal = 0

  for (const [index, rawCase] of rawCases.entries()) {
    const at = `cases[${index}]`
    if (typeof rawCase !== 'object' || rawCase === null || Array.isArray(rawCase)) {
      reject(at, `期望对象，实际 ${describeValue(rawCase)}`)
    }
    const entry = rawCase as Record<string, unknown>
    for (const field of ['id', 'namespace', 'app', 'goal'] as const) {
      const value = entry[field]
      if (typeof value !== 'string' || value.trim() === '') {
        reject(`${at}.${field}`, `须为非空字符串，实际 ${JSON.stringify(value)}`)
      }
    }
    const id = entry['id'] as string
    if (seen.has(id)) reject(`${at}.id`, `id 重复：${id}`)
    seen.add(id)

    const weight = entry['weight']
    if (!isFiniteNumber(weight) || weight <= 0) {
      reject(`${at}.weight`, `须为 >0 的数值，实际 ${JSON.stringify(weight)}`)
    }
    weightTotal += weight

    const dimensions = entry['dimensions']
    if (typeof dimensions !== 'object' || dimensions === null || Array.isArray(dimensions)) {
      reject(`${at}.dimensions`, `期望对象，实际 ${describeValue(dimensions)}`)
    }
    const dimensionRecord = dimensions as Record<string, unknown>
    const dimensionKeys = Object.keys(dimensionRecord)
    if (dimensionKeys.length === 0) reject(`${at}.dimensions`, '须至少声明一个维度')
    let dimensionSum = 0
    for (const key of dimensionKeys) {
      if (!EVAL_DIMENSIONS.includes(key)) {
        reject(`${at}.dimensions.${key}`, `非法维度（白名单：${EVAL_DIMENSIONS.join(', ')}）`)
      }
      const value = dimensionRecord[key]
      if (!isFiniteNumber(value) || value < 0 || value > 1) {
        reject(`${at}.dimensions.${key}`, `须为 [0,1] 的数值，实际 ${JSON.stringify(value)}`)
      }
      dimensionSum += value
    }
    if (Math.abs(dimensionSum - 1) > WEIGHT_SUM_TOLERANCE) {
      reject(`${at}.dimensions`, `维度权重合计 ${dimensionSum} ≠ 1.0`)
    }

    const rubricWeight = entry['rubricWeight']
    if (rubricWeight !== undefined && (!isFiniteNumber(rubricWeight) || rubricWeight < 0 || rubricWeight > 1)) {
      reject(`${at}.rubricWeight`, `须在 [0,1]，实际 ${JSON.stringify(rubricWeight)}`)
    }

    cases.push({
      id,
      namespace: entry['namespace'] as string,
      app: entry['app'] as string,
      goal: entry['goal'] as string,
      weight,
      dimensions: Object.fromEntries(dimensionKeys.map(key => [key, dimensionRecord[key] as number])),
      ...(rubricWeight === undefined ? {} : { rubricWeight }),
    })
  }

  if (Math.abs(weightTotal - 1) > WEIGHT_SUM_TOLERANCE) {
    reject('cases', `case 权重合计 ${weightTotal} ≠ 1.0`)
  }

  return { schema: EVAL_BASELINE_SCHEMA, threshold, tolerance, cases }
}

/**
 * 按案例维度权重加权评分。**证据缺失或非法即该维度 0 分，并把整轮标为 degraded**
 * （degraded ≠ 通过：调用方 MUST NOT 以 degraded 结果更新基线）。
 */
export function scoreRun(input: {
  readonly caseSet: EvalCaseSet
  readonly evidence: Record<string, EvalEvidence>
}): { score: number; degraded: boolean; perCase: Record<string, number> } {
  const perCase: Record<string, number> = {}
  let degraded = false
  let weightedSum = 0
  let weightTotal = 0

  for (const evalCase of input.caseSet.cases) {
    const record = input.evidence[evalCase.id] ?? {}
    let caseScore = 0
    for (const [dimension, dimensionWeight] of Object.entries(evalCase.dimensions)) {
      const raw = record[dimension as keyof EvalEvidence]
      const usable = isFiniteNumber(raw) && raw >= 0 && raw <= 1
      if (!usable) degraded = true
      caseScore += dimensionWeight * (usable ? raw : 0)
    }
    const rounded = Math.round(caseScore * 1e6) / 1e6
    perCase[evalCase.id] = rounded
    weightedSum += evalCase.weight * rounded
    weightTotal += evalCase.weight
  }

  return {
    score: weightTotal === 0 ? 0 : Math.round((weightedSum / weightTotal) * 1e6) / 1e6,
    degraded,
    perCase,
  }
}

/**
 * 回归裁决：overall < threshold，或任一 case 相对基线降幅 > tolerance ⇒ regressed。
 * 本次缺某 case 分数 → 按 0 计并记入 detail（fail-closed，不得当「无变化」）。
 */
export function decideRegression(input: {
  readonly baseline: Record<string, number>
  readonly current: Record<string, number>
  readonly caseSet: EvalCaseSet
}): { verdict: 'pass' | 'regressed'; detail: string } {
  const reasons: string[] = []
  let weightedSum = 0
  let weightTotal = 0

  for (const evalCase of input.caseSet.cases) {
    const current = input.current[evalCase.id]
    if (!isFiniteNumber(current)) reasons.push(`case ${evalCase.id} 无本次评分（按 0 计）`)
    const score = isFiniteNumber(current) ? current : 0
    weightedSum += evalCase.weight * score
    weightTotal += evalCase.weight
  }
  const overall = weightTotal === 0 ? 0 : weightedSum / weightTotal
  const overallRounded = Math.round(overall * 1e3) / 1e3

  if (overall < input.caseSet.threshold) {
    reasons.push(`overall ${overallRounded} < threshold ${input.caseSet.threshold}`)
  }
  for (const evalCase of input.caseSet.cases) {
    const baseline = input.baseline[evalCase.id]
    if (!isFiniteNumber(baseline)) continue
    const current = isFiniteNumber(input.current[evalCase.id]) ? (input.current[evalCase.id] as number) : 0
    const drop = baseline - current
    if (drop > input.caseSet.tolerance) {
      reasons.push(`case ${evalCase.id} 降幅 ${Math.round(drop * 1e3) / 1e3} > tolerance ${input.caseSet.tolerance}`)
    }
  }

  return reasons.length > 0
    ? { verdict: 'regressed', detail: reasons.join('；') }
    : {
        verdict: 'pass',
        detail: `overall ${overallRounded} ≥ threshold ${input.caseSet.threshold}；逐 case 降幅均 ≤ tolerance ${input.caseSet.tolerance}`,
      }
}
