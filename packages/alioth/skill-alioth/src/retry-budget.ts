/**
 * 重试预算器：对话循环与技能执行器异常路径的外部强制约束——不依赖 LLM 自觉终止，
 * 由预算器以签名计数裁决。契约来源：上游 `Meta/backend/app-agent/src/retry_budget.rs`。
 *
 * 两个维度（上游 design D4：叠加取更严）：
 * - **调用签名**（ping-pong 检测）：工具名 + 参数规范化哈希；**连续**同一签名达
 *   `maxRepeatCall` 次 → `terminate`（死循环强制终止）。语义取上游修正后的「连续调用集」
 *   口径：只要出现**新的**调用签名（进展）即清零，长链路 turn 里跨步骤合法重复读同一文件
 *   不得被误判为死循环。
 * - **错误签名**（修复墙）：第 2 次 → `trim-retry`（fresh-context 裁剪重试），第
 *   `maxRepeatError` 次 → `escalate`（转人工）。签名优先取 `repair.ts` 的规则码——同一根因
 *   恒得同一签名，终端输出漂移不再产生「新签名」而让修复墙事实上打不响。
 *
 * 动作优先级：`terminate` > `escalate` > `trim-retry` > `allow`。
 * @module @dsh-alioth/skill-alioth/retry-budget
 */

import { createHash } from 'node:crypto'
import { ruleIdFromError } from './repair.ts'

/** 同一调用签名连续重复上限（死循环判定；对齐三次墙纪律）。 */
export const MAX_REPEAT_CALL = 3

/** 同一错误签名重复上限（第 2 次裁剪重试，第 3 次转人工）。 */
export const MAX_REPEAT_ERROR = 3

/** 预算动作（优先级 terminate > escalate > trim-retry > allow）。 */
export type RetryDecision = 'allow' | 'trim-retry' | 'escalate' | 'terminate'

export interface RetryBudgetOptions {
  /** 同一调用签名连续重复上限；默认 {@link MAX_REPEAT_CALL}。 */
  readonly maxRepeatCall?: number
  /** 同一错误签名重复上限；默认 {@link MAX_REPEAT_ERROR}。 */
  readonly maxRepeatError?: number
}

/** sha256 十六进制前 16 位（上游 `result_digest` 的等价摘要口径）。 */
function digest(input: string): string {
  return createHash('sha256').update(input, 'utf8').digest('hex').slice(0, 16)
}

/** 规范化 JSON：键序无关（同语义调用不同键序必须同签名），数组保持顺序。 */
function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value) ?? 'null'
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  const record = value as Record<string, unknown>
  const keys = Object.keys(record).sort()
  return `{${keys.map(key => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(',')}}`
}

/**
 * 调用签名：`{tool}:{sha256(canonical_json(args))[:16]}`——键序无关，同语义调用同签名。
 */
export function callSignature(tool: string, args: unknown): string {
  return `${tool}:${digest(canonicalJson(args))}`
}

/**
 * 错误签名：命中 `[rule:<id>]` 信封时优先取规则码（同根因恒同签名），否则退化为
 * `{tool}:{sha256(errorText)[:16]}`。与上游一致保留工具名维度（同一规则来自不同工具
 * 视为不同修复面）。
 */
export function errorSignature(tool: string, errorText: string): string {
  const ruleId = ruleIdFromError(errorText)
  return ruleId === null ? `${tool}:${digest(errorText)}` : `${tool}:${ruleId}`
}

function positiveInt(value: number | undefined, fallback: number, name: string): number {
  if (value === undefined) return fallback
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`skill-alioth: RetryBudget ${name} must be an integer >= 1 (got ${String(value)})`)
  }
  return value
}

/**
 * 重试预算器：单轮（turn / skill step）作用域，轮次开始新建。
 *
 * `recordCall`/`recordError` 只登记事实，{@link RetryBudget.decide} 按优先级裁决当前动作，
 * 且是**纯查询**（不消费计数）：同一批调用后重复询问得到同一答案，调用方自行决定是否结束
 * 本轮。调用维度按**连续**签名计数（新签名 = 进展 = 清零）；错误维度按签名累计计数。
 */
export class RetryBudget {
  private readonly maxRepeatCall: number
  private readonly maxRepeatError: number
  private readonly errorCounts = new Map<string, number>()
  private lastCallSignature: string | null = null
  private consecutiveCalls = 0

  constructor(options: RetryBudgetOptions = {}) {
    this.maxRepeatCall = positiveInt(options.maxRepeatCall, MAX_REPEAT_CALL, 'maxRepeatCall')
    this.maxRepeatError = positiveInt(options.maxRepeatError, MAX_REPEAT_ERROR, 'maxRepeatError')
  }

  /** 登记一次工具调用；连续相同签名（+1）累加，出现新签名即清零重计。 */
  recordCall(tool: string, args: unknown): void {
    const signature = callSignature(tool, args)
    if (signature === this.lastCallSignature) {
      this.consecutiveCalls += 1
    } else {
      this.lastCallSignature = signature
      this.consecutiveCalls = 1
    }
  }

  /** 登记一次工具失败，返回该签名（规则码优先）的累计次数。 */
  recordError(tool: string, errorText: string): number {
    const signature = errorSignature(tool, errorText)
    const count = (this.errorCounts.get(signature) ?? 0) + 1
    this.errorCounts.set(signature, count)
    return count
  }

  /** 当前动作：terminate > escalate > trim-retry > allow（纯查询，不消费计数）。 */
  decide(): RetryDecision {
    if (this.consecutiveCalls >= this.maxRepeatCall) return 'terminate'
    let worst = 0
    for (const count of this.errorCounts.values()) {
      if (count > worst) worst = count
    }
    if (worst >= this.maxRepeatError) return 'escalate'
    if (worst >= 2) return 'trim-retry'
    return 'allow'
  }
}
