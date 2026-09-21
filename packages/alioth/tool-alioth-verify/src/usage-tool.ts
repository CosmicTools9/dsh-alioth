/**
 * `alioth_usage`——会话用量与成本口径（薄封装 `@dsh-alioth/verify-alioth`，零 LLM）。
 *
 * 事实来源是**守卫**的会话台账（`aliothGuard.usage(sessionId)`，由 `session/event` 累积、
 * 与 `verify-alioth/aggregateUsage` 同一口径）：本包**不**自建第二份台账，避免口径漂移。
 * 守卫未装配 → `available:false` + 显式原因（MUST NOT 编造用量）。
 *
 * 成本：默认沿用台账的成本口径；部署若配置了价表（`priceTable`，JSON 文件：`模型 → {centsPerInK,
 * centsPerOutK}`），按价表重估。**缺价表或存在无单价模型 ⇒ 显式 `unavailable` + 原因**，
 * MUST NOT 以 0 冒充成本（少算成「看起来更便宜」是禁止的）。
 * @module @dsh-alioth/tool-alioth-verify/usage-tool
 */

import { readFile } from 'node:fs/promises'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { estimateCost, type PriceTable, type UsageSummary } from '@dsh-alioth/verify-alioth'
import { GUARD_ABSENT_REASON, guardOf } from './guard-source.ts'
import { asJsonOutput } from './json-output.ts'
import { sessionIdOf } from './paths.ts'

/** 读价表（缺文件/不可解析/形态非法 → throw，由调用方显式降级为 unavailable）。 */
async function readPriceTable(file: string): Promise<PriceTable> {
  const text = await readFile(file, 'utf8').catch(() => null)
  if (text === null) throw new Error(`价表不可读：${file}`)
  const parsed = JSON.parse(text) as unknown
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new Error(`价表形态非法（期望 {模型: {centsPerInK, centsPerOutK}}）：${file}`)
  }
  const table: PriceTable = {}
  for (const [model, price] of Object.entries(parsed as Record<string, unknown>)) {
    if (typeof price !== 'object' || price === null || Array.isArray(price)) {
      throw new Error(`价表项 ${model} 形态非法（期望 {centsPerInK, centsPerOutK}）：${file}`)
    }
    const record = price as Record<string, unknown>
    const centsPerInK = record['centsPerInK']
    const centsPerOutK = record['centsPerOutK']
    if (typeof centsPerInK !== 'number' || typeof centsPerOutK !== 'number') {
      throw new Error(`价表项 ${model} 必须是数值 {centsPerInK, centsPerOutK}：${file}`)
    }
    table[model] = { centsPerInK, centsPerOutK }
  }
  return table
}

/** 注册 `alioth_usage`。 */
export function registerUsageTool(ctx: Context, options: { readonly priceTable?: string }): void {
  ctx.tools.register(defineTool({
    name: 'alioth_usage',
    description:
      'Session usage and cost (aggregated by the execution guard from real model-call events — never estimated by hand). '
      + 'Reports per-model and total token counts, per-turn wall time and call counts. Cost is ESTIMATED from the '
      + 'deployment price table when one is configured; with no price table, or any model missing a price, cost is '
      + 'explicitly `unavailable` with a reason instead of being reported as 0. '
      + 'When the guard is not mounted the whole report is `available:false` with the reason — no fabricated numbers.',
    parameters: {
      sessionId: { type: 'string', description: 'Session scope; defaults to this call\'s agent id, and is required when the call has no agent.' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          available: { type: 'boolean', required: true },
          reason: { type: 'string' },
          sessionId: { type: 'string' },
          priceTable: { type: 'string' },
          summary: { type: 'json' },
        },
      },
      render: (_args, value) => {
        if (value.available === false) {
          return [{ type: 'text', text: `usage unavailable: ${String(value.reason)}` }]
        }
        const summary = value.summary as {
          total?: { calls?: number }
          cost?: { kind?: string; totalCents?: number; reason?: string }
        } | undefined
        const cost = summary?.cost
        const costText = cost?.kind === 'estimated'
          ? `${String(cost.totalCents)}c`
          : `unavailable (${cost?.reason ?? 'no price table'})`
        return [{ type: 'text', text: `usage: ${String(summary?.total?.calls ?? 0)} call(s), cost=${costText}` }]
      },
    },
    async execute(args, exec) {
      const a = args as Record<string, unknown>
      const sessionId = sessionIdOf(exec, a.sessionId)
      const guard = guardOf(ctx)
      if (guard === undefined || typeof guard.usage !== 'function') {
        return { available: false, reason: `${GUARD_ABSENT_REASON}——会话用量台账由守卫累积`, sessionId, summary: null }
      }
      const summary = guard.usage(sessionId)
      if (options.priceTable === undefined) {
        return { available: true, sessionId, priceTable: '', summary: asJsonOutput(summary) }
      }
      let repriced: UsageSummary
      try {
        const prices = await readPriceTable(options.priceTable)
        repriced = { ...summary, cost: estimateCost(summary, prices) }
      } catch (error) {
        repriced = {
          ...summary,
          cost: {
            kind: 'unavailable',
            reason: `配置的价表不可用（${error instanceof Error ? error.message : String(error)}）：MUST NOT 以 0 冒充成本`,
          },
        }
      }
      return { available: true, sessionId, priceTable: options.priceTable, summary: asJsonOutput(repriced) }
    },
    presentCall: args => ({
      card: 'generic',
      title: 'Alioth session usage',
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
