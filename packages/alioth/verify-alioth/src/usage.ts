/**
 * 会话用量与成本聚合 —— 对齐上游 `usage.rs`：
 *
 * - 真相源 = 已落盘的模型调用事件（零 DDL、可重算、跨进程一致）；
 * - 三维聚合：按模型 / 总量 / 按 turn（无 turn 归属 → turn 0）；
 * - 成本口径不可得（未配置价表，或存在无单价模型）⇒ MUST 显式 `unavailable` + 原因，
 *   **MUST NOT 以 0 冒充**（少算成「看起来更便宜」是禁止的）。
 * @module @dsh-alioth/verify-alioth/usage
 */

/** 单次模型调用事件。 */
export interface UsageEvent {
  readonly step: number
  readonly model: string
  readonly tokensIn: number
  readonly tokensOut: number
  readonly latencyMs: number
  readonly turn?: number
}

/** 用量报表。 */
export interface UsageSummary {
  readonly byModel: Record<string, { tokensIn: number; tokensOut: number; calls: number }>
  readonly total: { tokensIn: number; tokensOut: number; calls: number }
  readonly turns: readonly { turn: number; wallMs: number; calls: number }[]
  readonly cost: { kind: 'estimated'; totalCents: number } | { kind: 'unavailable'; reason: string }
}

/** 单价表：cents / 1K tokens。 */
export type PriceTable = Record<string, { centsPerInK: number; centsPerOutK: number }>

function roundCost(value: number): number {
  return Math.round(value * 1e6) / 1e6
}

/**
 * 成本估算：**全部**模型都有单价才给估算；缺价表或存在缺价模型 ⇒ 显式 unavailable。
 */
export function estimateCost(summary: UsageSummary, prices?: PriceTable): UsageSummary['cost'] {
  const models = Object.keys(summary.byModel)
  if (prices === undefined || Object.keys(prices).length === 0) {
    return { kind: 'unavailable', reason: '未配置单价表（口径不可得，MUST NOT 以 0 冒充成本）' }
  }
  const unpriced = models.filter(model => prices[model] === undefined)
  if (unpriced.length > 0) {
    return {
      kind: 'unavailable',
      reason: `存在无单价模型：${unpriced.join(', ')}（全模型都有单价才给估算，避免少算）`,
    }
  }
  if (models.length === 0) {
    return { kind: 'estimated', totalCents: 0 }
  }
  let totalCents = 0
  for (const model of models) {
    const price = prices[model] as { centsPerInK: number; centsPerOutK: number }
    const usage = summary.byModel[model] as { tokensIn: number; tokensOut: number; calls: number }
    totalCents += (usage.tokensIn / 1000) * price.centsPerInK + (usage.tokensOut / 1000) * price.centsPerOutK
  }
  return { kind: 'estimated', totalCents: roundCost(totalCents) }
}

/** 聚合用量事件（纯函数，不触盘）：按模型 / 总量 / 按 turn 三维，成本由 `prices` 决定可得性。 */
export function aggregateUsage(events: readonly UsageEvent[], prices?: PriceTable): UsageSummary {
  const byModel: Record<string, { tokensIn: number; tokensOut: number; calls: number }> = {}
  const turnMap = new Map<number, { wallMs: number; calls: number }>()
  let tokensIn = 0
  let tokensOut = 0
  let calls = 0

  for (const event of events) {
    const bucket = byModel[event.model] ?? { tokensIn: 0, tokensOut: 0, calls: 0 }
    bucket.tokensIn += event.tokensIn
    bucket.tokensOut += event.tokensOut
    bucket.calls += 1
    byModel[event.model] = bucket

    const turn = event.turn ?? 0
    const turnBucket = turnMap.get(turn) ?? { wallMs: 0, calls: 0 }
    turnBucket.wallMs += event.latencyMs
    turnBucket.calls += 1
    turnMap.set(turn, turnBucket)

    tokensIn += event.tokensIn
    tokensOut += event.tokensOut
    calls += 1
  }

  const turns = [...turnMap.entries()]
    .map(([turn, bucket]) => ({ turn, wallMs: bucket.wallMs, calls: bucket.calls }))
    .sort((a, b) => a.turn - b.turn)

  const summary: UsageSummary = {
    byModel,
    total: { tokensIn, tokensOut, calls },
    turns,
    cost: { kind: 'unavailable', reason: '尚未估算' },
  }
  return { ...summary, cost: estimateCost(summary, prices) }
}
