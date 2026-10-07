/**
 * 账户计量（C2）：AccountMeter 的落账/口径纪律 + 守卫接线后的
 * 「turn 收口落账」与「月度预算阻断」——身份与落账面都是注入的结构化假服务，
 * 事件面走真实的 session/event 观察缝。
 */
import { describe, expect, it, beforeEach } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import type { Session, SessionEvent, SessionId } from '@deepseek-ai/dsh-session'
import type { Agent } from '@deepseek-ai/dsh-agent'
import { AccountMeter, type MeterFlushEntry, type MeteringSink } from '../src/metering.ts'
import * as guard from '../src/index.ts'

describe('AccountMeter (unit)', () => {
  const prices = { 'deepseek-chat': { centsPerInK: 0.2, centsPerOutK: 0.8 } }
  const event = (model: string, tokensIn = 1000, tokensOut = 500) =>
    ({ step: 1, model, tokensIn, tokensOut, latencyMs: 10, turn: 1 })

  it('flushes day-bucketed entries per model with per-model cost', async () => {
    const flushed: { account: string; entries: MeterFlushEntry[] }[] = []
    const sink: MeteringSink = {
      recordUsage: async (account: string, entries: readonly MeterFlushEntry[]) => { flushed.push({ account, entries: [...entries] }) },
      usageMonthly: async () => ({ costCents: null }),
    }
    const meter = new AccountMeter({ prices, sink })
    meter.setAccount('s1', 'U-ada')
    const count = await meter.flush('s1', [event('deepseek-chat'), event('deepseek-chat'), event('mystery-model')], Date.UTC(2026, 9, 7))
    expect(count).toBe(2)
    expect(meter.offsetOf('s1')).toBe(3)
    const byModel = new Map(flushed[0]!.entries.map(entry => [entry.model, entry]))
    expect(byModel.get('deepseek-chat')).toMatchObject({
      day: '2026-10-07', tokensIn: 2000, tokensOut: 1000, calls: 2, costCents: 1.2,
    })
    // 无价模型只毒化自己的桶。
    expect(byModel.get('mystery-model')).toMatchObject({ costCents: null })
  })

  it('does not flush without an account or a sink, but still advances the cursor', async () => {
    const meter = new AccountMeter({ prices })
    meter.setAccount('s1', 'U-ada')
    expect(await meter.flush('s1', [event('deepseek-chat')])).toBe(0)
    expect(meter.offsetOf('s1')).toBe(1)
    const sink: MeteringSink = { recordUsage: async () => {}, usageMonthly: async () => ({ costCents: null }) }
    const noAccount = new AccountMeter({ prices, sink })
    expect(await noAccount.flush('s2', [event('deepseek-chat')])).toBe(0)
  })

  it('unflushedCost is null when any model is unpriced (never 0-faked)', () => {
    const meter = new AccountMeter({ prices })
    expect(meter.unflushedCost('s1', [event('deepseek-chat')])).toBe(0.6)
    expect(meter.unflushedCost('s1', [event('deepseek-chat'), event('mystery')])).toBeNull()
    expect(meter.unflushedCost('s1', [])).toBe(0)
  })

  it('memoizes monthCost per account and invalidates on flush', async () => {
    let reads = 0
    const sink: MeteringSink = {
      recordUsage: async () => {},
      usageMonthly: async () => { reads += 1; return { costCents: reads * 10 } },
    }
    const meter = new AccountMeter({ prices, sink })
    expect(await meter.monthCost('U-ada')).toBe(10)
    expect(await meter.monthCost('U-ada')).toBe(10) // memo 命中
    expect(reads).toBe(1)
    meter.resetMemo()
    expect(await meter.monthCost('U-ada')).toBe(20)
    expect(reads).toBe(2)
    expect(await meter.monthCost('U-eve')).toBe(30)
    expect(reads).toBe(3)
    // 落账即失效：预算判定不吃 memo 窗口里的旧数。
    meter.setAccount('s-flush', 'U-ada')
    await meter.flush('s-flush', [event('deepseek-chat')], Date.now())
    expect(await meter.monthCost('U-ada')).toBe(40)
    expect(reads).toBe(4)
  })
})

// ── 守卫接线：落账与预算阻断 ─────────────────────────────────────────────

const recorded: { account: string; entries: MeterFlushEntry[] }[] = []
let monthlyCost: number | null = 0.01
let quota: number | null = 0.01

function makeAgent(sessionId: string): Agent {
  return { id: sessionId as SessionId, session: { id: sessionId as SessionId } } as unknown as Agent
}

describe('guard wiring (turn-end flush + monthly budget)', () => {
  let ctx: Context
  let seq = 0

  const emit = (sessionId: string, type: string, data: unknown, time: number): void => {
    seq += 1
    const event = { type, seq, time, data } as unknown as SessionEvent
    ctx.emit('session/event', { id: sessionId as SessionId } as Session, event)
  }
  const preStep = async (sessionId: string, turn: number) =>
    ctx.waterfall(
      'agent/pre-step',
      { agent: makeAgent(sessionId), messages: [], turn, step: 1, signal: new AbortController().signal },
      async () => ({ kind: 'enter' as const, messages: [] }),
    )

  beforeEach(async () => {
    recorded.length = 0
    monthlyCost = 0.01
    quota = 0.01
    ctx = new Context()
    ctx.provide('aliothBilling')
    ctx.set('aliothBilling', {
      async recordUsage(account: string, entries: readonly MeterFlushEntry[]) { recorded.push({ account, entries: [...entries] }) },
      async usageMonthly() { return { costCents: monthlyCost } },
      async entitlement() { return { plan: 'L1', monthlyCostCents: quota } },
      async planOf() { return 'L1' as const },
    } as never)
    // 守卫 inject ['aliothEnv','tools']：env 用假结构面（本 spec 不触模型树）。
    ctx.provide('aliothEnv')
    ctx.set('aliothEnv', {
      ready: async () => ({ modelDir: '/tmp/guard-metering-model' }),
      sql: async () => ({ rows: [], rowCount: 0 }),
    } as never)
    const system = await ctx.plugin(SystemPrompt)
    ;(ctx as unknown as { __systemDispose?: () => Promise<void> }).__systemDispose = () => system.dispose()
    const tools = await ctx.plugin(ToolRuntime)
    ;(ctx as unknown as { __toolsDispose?: () => Promise<void> }).__toolsDispose = () => tools.dispose()
    ctx.provide('aliothAuth')
    ctx.set('aliothAuth', {
      async accountForSession(sessionId: string) {
        return sessionId.startsWith('acct-')
          ? { userId: 'u1', username: 'ada', namespace: 'U-ada', role: 'user', plan: 'L1', monthlyCostCents: quota }
          : null
      },
    } as never)
    const plugin = await ctx.plugin(guard, { preProcRoot: '/tmp/guard-metering', dataRoot: '/tmp/guard-metering' })
    ;(ctx as unknown as { __guardDispose?: () => Promise<void> }).__guardDispose = () => plugin.dispose()
  })

  it('flushes the ledger delta to the billing account ledger at turn end', async () => {
    const sessionId = 'acct-flush'
    await preStep(sessionId, 1) // pre-step 解析身份 → 账户绑定（无身份即不计量）
    emit(sessionId, 'turn/start', { turn: 1 }, 1_000)
    emit(sessionId, 'assistant/message', {
      step: 1, turn: 1,
      message: { source: { model: 'deepseek-chat' } },
      usage: { inputTokens: 1000, outputTokens: 500 },
    }, 1_100)
    emit(sessionId, 'turn/end', { turn: 1, reason: { kind: 'completed' } }, 1_200)
    await new Promise(resolve => setTimeout(resolve, 20))
    expect(recorded).toHaveLength(1)
    expect(recorded[0]!.account).toBe('U-ada')
    expect(recorded[0]!.entries[0]).toMatchObject({
      model: 'deepseek-chat', tokensIn: 1000, tokensOut: 500, calls: 1,
    })
    expect(typeof recorded[0]!.entries[0]!.costCents).toBe('number')
    await (ctx as unknown as { __guardDispose?: () => Promise<void> }).__guardDispose?.()
  })

  it('rejects the next step when the account is over its monthly budget', async () => {
    monthlyCost = 5 // 账本里已花 5 分
    quota = 1 // 预算 1 分
    const sessionId = 'acct-budget'
    const decision = await preStep(sessionId, 1)
    expect(decision.kind).toBe('reject')
    // 预算充足（或未配置）→ 放行。
    quota = null
    const allowed = await preStep('acct-budget', 2)
    expect(allowed.kind).toBe('enter')
    await (ctx as unknown as { __guardDispose?: () => Promise<void> }).__guardDispose?.()
  })

  it('degrades honestly when the cost basis is unavailable, and never judges a null basis', async () => {
    monthlyCost = null // 账本存在无价桶：口径不可得
    quota = 1
    const sessionId = 'acct-null-basis'
    const decision = await preStep(sessionId, 1)
    expect(decision.kind).toBe('enter') // 不判 ≠ 拒判；MUST NOT 以 0 冒充
    await (ctx as unknown as { __guardDispose?: () => Promise<void> }).__guardDispose?.()
  })

  it('accountUsage exposes the plan/quota/month view and null without identity', async () => {
    const view = await ctx.aliothGuard.accountUsage('acct-view')
    expect(view).toMatchObject({ account: 'U-ada', plan: 'L1', quotaCents: 0.01, monthlyCostCents: 0.01 })
    expect(await ctx.aliothGuard.accountUsage('no-identity')).toBeNull()
    await (ctx as unknown as { __guardDispose?: () => Promise<void> }).__guardDispose?.()
  })
})
