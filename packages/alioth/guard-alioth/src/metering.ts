/**
 * 账户计量（C2 metering）：把会话用量事件聚合成**账户键**的日桶，落进计费
 * 域的用量账本（`aliothBilling.recordUsage`），并回答「该账户本月已花多少」。
 *
 * 分工不变：`session/event` 仍只喂 `SessionLedger` 一个消费者；计量从台账
 * **读**事件（`usageSlice`），不自建第二份解读。账户↔会话的映射由插件在
 * `agent/pre-step` 用 `aliothAuth.accountForSession`（自带 60s memo）现解析。
 *
 * 成本口径纪律与 verify-alioth 相同：任一模型无单价 ⇒ 该桶/该月成本为
 * `null`（MUST NOT 以 0 冒充），预算执行对 null 口径**不判**并留降级证据。
 * @module @dsh-alioth/guard-alioth/metering
 */

import { aggregateUsage, estimateCost, type PriceTable, type UsageEvent } from '@dsh-alioth/verify-alioth'

/** 一条落账条目（与 `aliothBilling.recordUsage` 的条目同形）。 */
export interface MeterFlushEntry {
  readonly day: string
  readonly model: string
  readonly tokensIn: number
  readonly tokensOut: number
  readonly calls: number
  readonly costCents: number | null
}

/** 计量的落账面（billing 服务满足它；缺席 ⇒ 只算不落）。 */
export interface MeteringSink {
  recordUsage(account: string, entries: readonly MeterFlushEntry[]): Promise<void>
  usageMonthly(account: string): Promise<{ readonly costCents: number | null }>
}

export interface AccountMeterOptions {
  readonly prices?: PriceTable
  readonly sink?: MeteringSink
  /** 月度成本查询的 memo 时长（毫秒）；默认 30s（flush 落账后立即失效）。 */
  readonly monthMemoMs?: number
}

const DAY_MS = 24 * 3600 * 1000

function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10)
}

/** 账户计量：会话→账户映射、增量落账、月度成本 memo。 */
export class AccountMeter {
  private readonly accounts = new Map<string, string>()
  private readonly offsets = new Map<string, number>()
  private readonly monthMemo = new Map<string, { cents: number | null; at: number }>()
  private readonly options: AccountMeterOptions

  constructor(options: AccountMeterOptions = {}) {
    this.options = options
  }

  /** 绑定（或解绑）一个会话的账户。 */
  setAccount(sessionId: string, account: string | null): void {
    if (account === null) {
      this.accounts.delete(sessionId)
    } else {
      this.accounts.set(sessionId, account)
    }
  }

  accountOf(sessionId: string): string | null {
    return this.accounts.get(sessionId) ?? null
  }

  /** 已落账的事件下标（台账里该会话用量事件的游标）。 */
  offsetOf(sessionId: string): number {
    return this.offsets.get(sessionId) ?? 0
  }

  /**
   * 把 `events`（调用方从台账 `usageSlice(sessionId, offsetOf(sessionId))` 取的
   * 增量）按 日×模型 聚合落账，并推进游标。无落账面（无 billing）时只推进
   * 游标——成本仍可由台账回答，只是不持久。
   * @returns 本次真正落账的条目数（0 = 无 sink 或无增量）。
   */
  async flush(sessionId: string, events: readonly UsageEvent[], now = Date.now()): Promise<number> {
    this.offsets.set(sessionId, this.offsetOf(sessionId) + events.length)
    const account = this.accounts.get(sessionId)
    const sink = this.options.sink
    if (account === undefined || sink === undefined || events.length === 0) {
      return 0
    }
    const day = utcDay(now)
    const buckets = new Map<string, { tokensIn: number; tokensOut: number; calls: number }>()
    for (const event of events) {
      const bucket = buckets.get(event.model) ?? { tokensIn: 0, tokensOut: 0, calls: 0 }
      bucket.tokensIn += event.tokensIn
      bucket.tokensOut += event.tokensOut
      bucket.calls += 1
      buckets.set(event.model, bucket)
    }
    const entries: MeterFlushEntry[] = []
    for (const [model, bucket] of buckets) {
      // Per-model cost: aggregate only that model's events; a model without a
      // price poisons its own bucket (null), not the others.
      const modelEvents = events.filter(candidate => candidate.model === model)
      const modelCost = estimateCost(aggregateUsage(modelEvents, this.options.prices), this.options.prices)
      entries.push({
        day,
        model,
        tokensIn: bucket.tokensIn,
        tokensOut: bucket.tokensOut,
        calls: bucket.calls,
        costCents: modelCost.kind === 'estimated' ? modelCost.totalCents : null,
      })
    }
    await sink.recordUsage(account, entries)
    // 落账即失效该账户的月度 memo：预算判定永远站在落账后的口径上，
    // 不吃 30s 窗口里的旧数。
    this.monthMemo.delete(account)
    return entries.length
  }

  /** 未落账增量的估算成本（分）；任一模型无价 ⇒ null（不判预算，不以 0 冒充）。 */
  unflushedCost(sessionId: string, events: readonly UsageEvent[]): number | null {
    if (events.length === 0) return 0
    const cost = estimateCost(aggregateUsage(events, this.options.prices), this.options.prices)
    return cost.kind === 'estimated' ? cost.totalCents : null
  }

  /**
   * 账户当月已落账成本（分），带 memo；`null` = 账本里存在无价桶（口径不可得）
   * 或无落账面。**调用方对 null 不得判预算。**
   */
  async monthCost(account: string): Promise<number | null> {
    const sink = this.options.sink
    if (sink === undefined) return null
    const cached = this.monthMemo.get(account)
    if (cached !== undefined && Date.now() - cached.at < (this.options.monthMemoMs ?? 30_000)) {
      return cached.cents
    }
    const month = await sink.usageMonthly(account)
    this.monthMemo.set(account, { cents: month.costCents, at: Date.now() })
    return month.costCents
  }

  /** 会话结束后的清理（长驻进程防无界增长）。 */
  forget(sessionId: string): void {
    this.accounts.delete(sessionId)
    this.offsets.delete(sessionId)
  }

  /** 换价表/测试用：清空月度 memo。 */
  resetMemo(): void {
    this.monthMemo.clear()
  }

  /** 供测试/展示：一天窗口（本进程至今未 flush 的天）——非持久真相源。 */
  dayWindow(now = Date.now()): string {
    return utcDay(Math.floor(now / DAY_MS) * DAY_MS)
  }
}
