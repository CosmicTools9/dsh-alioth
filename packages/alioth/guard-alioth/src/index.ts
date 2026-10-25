/**
 * `@dsh-alioth/guard-alioth` — AppAgent 执行层守卫插件（Wave 2 / T4）。
 *
 * 把上游 AppAgent 在**工具注册层**做的执行约束落到 harness 的三条缝上：
 *
 * 1. `ctx.tools.guard()`（同步全局层；2026-10-25 起——`tools/pre-execute`
 *    waterfall 在 web profile 的 per-agent preset realm 里派发，宿主面监听
 *    收不到，实测写沙箱静默失效）：
 *    - 工具面强制——调用名必须在当前步骤声明的 harness 工具面内（`enforceToolSurface`）；
 *    - namespace 写沙箱——写类调用的路径必须落在 `Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/`；
 *    - plan 写面收窄——plan 步的写入必须命中该步 `output_glob`；
 *    - 读围栏——读类调用的路径必须落在 `Pre-Proc/{ns}/` 或模型源目录内（`readFence`）。
 *    无 ns 可解析（headless / 未选工作区）时**不猜**：放行并留一条显式降级证据。
 * 2. 修复墙：`RetryBudget` 记调用/错误签名（调用签名在裁决内、错误签名在
 *    `session/event` 的 tool/result 观察面），三档动作（trim-retry / escalate /
 *    terminate）收敛为「下一次调用前置拒绝」+ 带规则码前缀的纠正反馈。
 * 3. `agent/pre-step`：闭环证据追问（上限 1 次/turn，与澄清提问/终态汇报互斥）与
 *    turn 预算超限后的下一步阻断（`reject`，原因带规则码前缀）。
 *
 * `session/event` 是唯一的观察面：喂 `ledger`（用量/turn 台账/预算）与 `scope`
 * （本会话最近一次 `alioth_workflow_*` 调用的 `{ns,app}`）。范围与当前步骤每次判定
 * 现读磁盘，绝不使用过期缓存。
 *
 * 服务面 `ctx.aliothGuard`：`whitelistSource()`（门禁程序白名单生效来源：`file` /
 * `code_default` + 可区分原因）、`activeScope(sessionId)`、`usage(sessionId)`、
 * `degradations()`（降级不得只活在日志里）。
 * @module @dsh-alioth/guard-alioth
 */

import path from 'node:path'
import { readFileSync } from 'node:fs'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import type { Session } from '@deepseek-ai/dsh-session'
import type { ToolExecution } from '@deepseek-ai/dsh-tools'
import { RetryBudget, type RetryDecision } from '@dsh-alioth/skill-alioth'
import type { PriceTable, UsageSummary } from '@dsh-alioth/verify-alioth'
import { SessionLedger, type TurnViolation } from './ledger.ts'
import { AccountMeter, type MeteringSink } from './metering.ts'
import { decideClosureNudge, type NudgeDecision } from './nudge.ts'
import { GUARD_RULE_IDS, guardFeedback, type GuardRuleId } from './rules.ts'
import { RunScopeResolver, WORKFLOW_SCOPE_TOOLS, type ActiveScope } from './scope.ts'
import {
  fullAccessVerdict,
  planWriteVerdict,
  readPathsOf,
  readVerdict,
  sandboxVerdict,
  toolSurfaceVerdict,
  writePathOf,
} from './surface.ts'
import { readWhitelistSource, type WhitelistSourceReport } from './whitelist.ts'

// harness 0.1.7 起 `MessageSourceMap` 是「各 producer 在自身模块声明 kind」的合并可扩展联合，
// **没有 catch-all 的 `plugin` kind**（见 dsh-llm/message.ts 的注释）。本插件注入的消息是
// 自己的生产者，故在此声明自有 kind——上游 dsh-tools 的 `tool-registry` 即同一模式。
declare module '@deepseek-ai/dsh-llm' {
  interface MessageSourceMap {
    /** guard-alioth 注入的闭环证据追问（每 turn 至多一次，与澄清提问/终态汇报互斥）。 */
    'alioth-guard': { kind: 'alioth-guard' }
  }
}

export const name = 'guard-alioth'
export const inject = ['aliothEnv', 'tools']

/** 适配器文件名缺省值（与 workflow 插件同一模型分发的 `skill-adapters/` 目录）。 */
export const DEFAULT_ADAPTER = 'alioth-app.yaml'

/** 降级证据表上限（bounded，避免长驻进程无界增长）。 */
const DEGRADATION_LIMIT = 100

/**
 * 内置默认价表（cents / 1K tokens，DeepSeek 官方人民币牌价，缓存未命中口径）：
 * deepseek-chat ¥2/1M 输入、¥8/1M 输出；deepseek-reasoner ¥4/1M、¥16/1M。
 * 部署可用 `priceTable` 文件整体覆盖（给 `{}` 即显式关闭成本口径）。
 */
export const DEFAULT_PRICE_TABLE: PriceTable = {
  'deepseek-chat': { centsPerInK: 0.2, centsPerOutK: 0.8 },
  'deepseek-reasoner': { centsPerInK: 0.4, centsPerOutK: 1.6 },
  'deepseek-v3': { centsPerInK: 0.2, centsPerOutK: 0.8 },
  'deepseek-r1': { centsPerInK: 0.4, centsPerOutK: 1.6 },
}

export interface Config {
  /** Pre-Proc 产物树根；写沙箱的绝对路径按此判定。 */
  readonly preProcRoot: string
  /** 状态根；run state 落在 `<dataRoot>/workflows/{ns}/{app}/run-state.json`。 */
  readonly dataRoot: string
  /** 适配器文件名（`skill-adapters/` 下），与 workflow 插件的部署选择一致。 */
  readonly adapter?: string
  /** 工具面强制开关；默认 true。 */
  readonly enforceToolSurface?: boolean
  /** 读围栏开关（读类工具的路径须落在命名空间根或模型源目录内）；默认 true。 */
  readonly readFence?: boolean
  /** 闭环证据追问开关；默认 true。 */
  readonly nudge?: boolean
  /** 同一调用签名连续重复上限（死循环判定）；默认由 skill-alioth 给出。 */
  readonly maxRepeatCall?: number
  /** 同一错误签名重复上限（第 2 次裁剪重试，第 N 次转人工）。 */
  readonly maxRepeatError?: number
  /** 单 turn 步数上限；缺省不限（部署选择）。 */
  readonly maxStepsPerTurn?: number
  /** 单 turn 墙钟上限（秒）；缺省不限（部署选择）。 */
  readonly turnTimeoutSec?: number
  /** 模型单价表：JSON 文件路径（`{ "<model>": { centsPerInK, centsPerOutK } }`）；缺省 → 成本口径 `unavailable`。 */
  readonly priceTable?: string
  /** 单 turn 估算成本上限（分）；与 `priceTable` 同时给出才生效。 */
  readonly maxTurnCostCents?: number
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  dataRoot: z.string().required(),
  adapter: z.string().default(DEFAULT_ADAPTER),
  enforceToolSurface: z.boolean().default(true),
  readFence: z.boolean().default(true),
  nudge: z.boolean().default(true),
  maxRepeatCall: z.number(),
  maxRepeatError: z.number(),
  maxStepsPerTurn: z.number(),
  turnTimeoutSec: z.number(),
  priceTable: z.string(),
  maxTurnCostCents: z.number(),
})

/** 一条显式降级证据（`unknown-scope` / 步骤不可读等）。 */
export interface Degradation {
  /** 会话 id；无 agent 的调用为 `null`。 */
  readonly sessionId: string | null
  readonly ruleId: GuardRuleId
  readonly reason: string
  /** Unix epoch 毫秒。 */
  readonly time: number
}

/** `ctx.aliothGuard` 服务面。 */
export interface AliothGuardService {
  /** 门禁程序白名单的生效来源（`file` = 运行时镜像；`code_default` = 回退代码常量 + 可区分原因）。 */
  whitelistSource(): Promise<WhitelistSourceReport>
  /** 现取会话的运行范围；无 workflow 调用记录 → `null`（未知范围）。 */
  activeScope(sessionId: string): Promise<ActiveScope | null>
  /** 会话用量与成本口径（成本不可得时显式 `unavailable`）。 */
  usage(sessionId: string): UsageSummary
  /**
   * 账户计量视图（C2）：会话绑定账户时的计划/当月已落账成本/配额 + 本会话台账。
   * 无账户（headless/未绑定）→ `null`。
   */
  accountUsage(sessionId: string): Promise<AccountUsageView | null>
  /** 已记录的降级证据（有界列表）。 */
  degradations(): readonly Degradation[]
}

/** 账户计量视图（`alioth_usage` 工具与运营面共用）。 */
export interface AccountUsageView {
  readonly account: string
  readonly plan: 'L0' | 'L1'
  /** 当月已落账成本（分）；账本存在无价桶时为 `null`（口径不可得，不以 0 冒充）。 */
  readonly monthlyCostCents: number | null
  /** 部署配置的月度预算（分）；未配置为 `null`（不判上限）。 */
  readonly quotaCents: number | null
  /** 本会话的台账汇总（与 `usage(sessionId)` 同口径）。 */
  readonly session: UsageSummary
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothGuard: AliothGuardService
  }
}

/** 修复墙三档动作 → 纠正反馈文本（规则码前缀 + 明确的下一步）。 */
function wallFeedback(
  decision: Exclude<RetryDecision, 'allow'>,
  tool: string,
  target: string,
): string {
  if (decision === 'terminate') {
    return guardFeedback({
      ruleId: GUARD_RULE_IDS.repairWall,
      repairClass: 'not-fixable',
      message: `检测到重复调用死循环：${tool} 的同一调用签名已连续重复达到上限`,
      action: '停止重复该调用：改变参数或换工具；确认无法推进时转人工',
      evidence: target,
    })
  }
  if (decision === 'escalate') {
    return guardFeedback({
      ruleId: GUARD_RULE_IDS.repairWall,
      repairClass: 'not-fixable',
      message: `已达修复墙：同一错误签名重复达到上限（${tool}）`,
      action: '停止自动修复：列出未完成项与阻塞点转人工',
      evidence: target,
    })
  }
  return guardFeedback({
    ruleId: GUARD_RULE_IDS.repairWall,
    repairClass: 'retryable',
    message: `上下文已裁剪：${tool} 的同一错误签名已重复出现`,
    action: '重读该错误的完整文本，换策略后再试；同一路径连续失败即触发裁剪重试',
    evidence: target,
  })
}

export function apply(ctx: Context, config: Config): void {
  const preProcRoot = path.resolve(config.preProcRoot)
  const adapter = config.adapter ?? DEFAULT_ADAPTER
  const enforceToolSurface = config.enforceToolSurface ?? true
  const readFence = config.readFence ?? true
  const nudgeEnabled = config.nudge ?? true
  const degradations: Degradation[] = []
  const degradedKeys = new Set<string>()
  const budgets = new Map<string, RetryBudget>()

  const prices = loadPrices()
  const ledger = new SessionLedger({
    ...config.maxStepsPerTurn === undefined ? {} : { maxStepsPerTurn: config.maxStepsPerTurn },
    ...config.turnTimeoutSec === undefined ? {} : { turnTimeoutSec: config.turnTimeoutSec },
    ...config.maxTurnCostCents === undefined ? {} : { maxTurnCostCents: config.maxTurnCostCents },
    ...prices === undefined ? {} : { prices },
  })

  // ── 账户计量（C2）：身份/落账面都是结构化查找（缺席即只算不落/不判） ──────
  interface BillingMeterLike {
    recordUsage(account: string, entries: readonly {
      day: string; model: string; tokensIn: number; tokensOut: number; calls: number; costCents: number | null
    }[]): Promise<void>
    usageMonthly(account: string): Promise<{ costCents: number | null }>
  }
  interface AuthAccountLike {
    accountForSession(sessionId: string): Promise<{
      userId: string; username: string; namespace: string; role: 'admin' | 'user'
      plan: 'L0' | 'L1'; monthlyCostCents: number | null
    } | null>
  }
  function serviceOf<T>(name: string, probe: (value: unknown) => boolean): T | undefined {
    try {
      const value = (ctx.get as (name: string) => unknown).call(ctx, name)
      return typeof value === 'object' && value !== null && probe(value) ? value as T : undefined
    } catch {
      return undefined
    }
  }
  // 结构化查找是惰性的：本插件在 bundle 行序上先于 billing/auth 挂载，启动期
  // 捕获会把「尚未 provide」误固化为「永不存在」（m2 实测 authAccount=false 导致
  // 读围栏的 namespace 后备通道失效）——每次使用时现查。
  const billingMeterOf = (): BillingMeterLike | undefined => serviceOf<BillingMeterLike>('aliothBilling', value =>
    typeof (value as BillingMeterLike).recordUsage === 'function' && typeof (value as BillingMeterLike).usageMonthly === 'function')
  const authAccountOf = (): AuthAccountLike | undefined => serviceOf<AuthAccountLike>('aliothAuth', value =>
    typeof (value as AuthAccountLike).accountForSession === 'function')
  const meter = new AccountMeter({
    ...prices === undefined ? {} : { prices },
    ...billingMeterOf() === undefined ? {} : { sink: billingMeterOf() as MeteringSink },
  })

  /**
   * 价表：默认用内置 DeepSeek 牌价（成本口径从第一轮起可得）；部署用
   * `priceTable` 指向 JSON 文件整体覆盖——文件不可读/结构非法 → 降级留痕并置
   * `undefined`：成本口径如实 `unavailable` 且**不判**上限（MUST NOT 以 0 冒充，
   * 也不静默吞掉配置错误）。
   */
  function loadPrices(): PriceTable | undefined {
    const file = config.priceTable
    if (file === undefined) {
      return DEFAULT_PRICE_TABLE
    }
    try {
      const parsed: unknown = JSON.parse(readFileSync(path.resolve(file), 'utf8'))
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        throw new Error('价表须为对象：{ "<model>": { centsPerInK, centsPerOutK } }')
      }
      const table: Record<string, { centsPerInK: number; centsPerOutK: number }> = {}
      for (const [model, entry] of Object.entries(parsed as Record<string, unknown>)) {
        const record = entry as { centsPerInK?: unknown; centsPerOutK?: unknown } | null
        if (typeof record?.centsPerInK !== 'number' || typeof record?.centsPerOutK !== 'number') {
          throw new Error(`价表条目 ${model} 须含数字 centsPerInK/centsPerOutK`)
        }
        table[model] = { centsPerInK: record.centsPerInK, centsPerOutK: record.centsPerOutK }
      }
      return table
    } catch (error) {
      noteDegradation(
        null,
        GUARD_RULE_IDS.turnCost,
        `价表不可用（${file}）：${error instanceof Error ? error.message : String(error)}`,
      )
      return undefined
    }
  }

  /** 降级留痕：日志 + 有界证据表，按（会话, 规则码）去重，避免每次调用重复告警。 */
  function noteDegradation(sessionId: string | null, ruleId: GuardRuleId, reason: string): void {
    const dedupeKey = `${sessionId ?? 'no-agent'}\u0000${ruleId}`
    if (degradedKeys.has(dedupeKey)) {
      return
    }
    degradedKeys.add(dedupeKey)
    ctx.logger.warn(`guard-alioth: 降级 [rule:${ruleId}] ${reason}`)
    degradations.push({ sessionId, ruleId, reason, time: Date.now() })
    if (degradations.length > DEGRADATION_LIMIT) {
      degradations.shift()
    }
  }

  const resolver = new RunScopeResolver({
    modelDir: async () => (await ctx.aliothEnv.ready()).modelDir,
    workflowRoot: () => path.join(path.resolve(config.dataRoot), 'workflows'),
    adapter,
    onDegrade: (sessionId, key, reason) =>
      noteDegradation(sessionId, GUARD_RULE_IDS.unknownScope, `${key.namespace}/${key.app}: ${reason}`),
  })

  // 同步判定的三个依赖面（tools.guard() 不支持 await）：
  // 模型源目录在挂载时预载（首个工具调用远晚于 boot 完成）；
  // 会话命名空间由 agent/pre-step（异步、每步必经）填充——工具调用必然晚于本步 pre-step；
  // callId→工具名映射供观察面记录错误签名。
  let modelDirCache: string | null = null
  void ctx.aliothEnv.ready().then(env => { modelDirCache = env.modelDir }, () => {})
  const sessionNamespace = new Map<string, string>()
  const callToolNames = new Map<string, string>()

  function budgetFor(sessionId: string): RetryBudget {
    const existing = budgets.get(sessionId)
    if (existing !== undefined) {
      return existing
    }
    const created = new RetryBudget({
      ...config.maxRepeatCall === undefined ? {} : { maxRepeatCall: config.maxRepeatCall },
      ...config.maxRepeatError === undefined ? {} : { maxRepeatError: config.maxRepeatError },
    })
    budgets.set(sessionId, created)
    return created
  }

  /** 账户上下文解析（auth 缺席/解析失败 ⇒ null——不猜身份）。 */
  async function resolveAccountContext(sessionId: string): Promise<{
    userId: string; username: string; namespace: string; role: 'admin' | 'user'
    plan: 'L0' | 'L1'; monthlyCostCents: number | null
  } | null> {
    const authAccount = authAccountOf()
    if (authAccount === undefined) return null
    try {
      return await authAccount.accountForSession(sessionId)
    } catch {
      return null
    }
  }

  const aliothGuard: AliothGuardService = {
    async whitelistSource(): Promise<WhitelistSourceReport> {
      return readWhitelistSource((await ctx.aliothEnv.ready()).modelDir)
    },
    activeScope: (sessionId: string) => resolver.activeScope(sessionId),
    usage: (sessionId: string) => ledger.usage(sessionId),
    async accountUsage(sessionId: string): Promise<AccountUsageView | null> {
      const account = await resolveAccountContext(sessionId)
      if (account === null) return null
      const monthly = await meter.monthCost(account.namespace)
      return {
        account: account.namespace,
        plan: account.plan,
        monthlyCostCents: monthly,
        quotaCents: account.monthlyCostCents,
        session: ledger.usage(sessionId),
      }
    },
    degradations: () => [...degradations],
  }

  // ── 观察面：会话事件（唯一的观测入口） ──────────────────────────────────
  ctx.on('session/event', (session: Session, event) => {
    const sessionId = String(session.id)
    ledger.observe(sessionId, event)
    if (event.type === 'tool/call') {
      resolver.observeCall(sessionId, event.data.name, parseArguments(event.data.arguments))
      if (event.data.callId !== undefined) callToolNames.set(String(event.data.callId), event.data.name)
      if (WORKFLOW_SCOPE_TOOLS.includes(event.data.name)) {
        void resolver.refresh(sessionId)
      }
    }
    if (event.type === 'tool/result') {
      // 修复墙的错误签名记账（观察面）：判定面在 tools.guard() 里同步 decide。
      const message = (event.data as { message?: { toolCallId?: string; isError?: boolean; content?: unknown } }).message
      if (message?.isError === true) {
        const tool = message.toolCallId !== undefined ? callToolNames.get(String(message.toolCallId)) : undefined
        if (tool !== undefined) {
          budgetFor(sessionId).recordError(tool, errorTextOf(message.content))
        }
      }
    }
    if (event.type === 'turn/start') {
      budgets.delete(sessionId)
    }
    // turn 收口即落账（C2）：把台账增量按 日×模型 汇进账户账本。失败只留痕，
    // 不打断会话——落账是计量义务，不是执行前提。
    if (event.type === 'turn/end') {
      const pending = ledger.usageSlice(sessionId, meter.offsetOf(sessionId))
      if (pending.length > 0) {
        void meter.flush(sessionId, pending, event.time).catch(error =>
          ctx.logger.warn(
            `guard-alioth: 用量落账失败（${sessionId}）：${error instanceof Error ? error.message : String(error)}`,
          ))
      }
    }
  })

  // ── 执行面：同步裁决（tools.guard() 全局层，preset realm 内同样生效） ────
  // 【2026-10-25 迁移】原先挂在 tools/pre-execute / tools/post-execute waterfall
  // 上——这两个事件经 scopeTarget(agent) 在 per-agent preset realm 里派发，
  // 宿主面插件的监听在 web profile 收不到（m2 实测：写沙箱静默失效，靠 harness
  // 自带 fs-sandbox 兜底掩盖；headless 无 realm 故一直有效、测试全绿）。
  // ctx.tools.guard() 是 harness 的官方宿主级单调守卫缝：global 层先于 scope
  // 链判定、同步、返回拒绝理由即拒——把全部判定同步化后注册于此。
  // 修复墙同步化：调用签名在裁决内记账；错误签名由 session/event 的
  // tool/result 观察记账；超限动作转为「下一次调用前置拒绝」（反馈文本同源）。

  /** 修复墙证据（同步口径：缓存快照，读不出则如实说明）。 */
  function wallTarget(sessionId: string): string {
    const scope = resolver.cachedScope(sessionId)
    if (scope === null) {
      return '未完成项：本会话尚无 alioth_workflow_* 调用记录，无法解析运行范围'
    }
    return scope.stepId === null
      ? `未完成项：${scope.namespace}/${scope.app} 的运行已结束（无当前步骤）`
      : `未完成项：${scope.namespace}/${scope.app} 的当前步骤 ${scope.stepId} 仍未通过`
  }

  /** 同步裁决一次工具调用；返回拒绝理由（单行规则信封）或 undefined 放行。 */
  function adjudicate(exec: ToolExecution): string | undefined {
    // 共享在线空间：完全控制的单次升级在参数层直接拒绝，审批卡不会渲染。
    const escalation = fullAccessVerdict(exec.name, exec.arguments)
    if (!escalation.ok) {
      ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（完全控制升级）`)
      return escalation.reason
    }
    const sessionId = exec.agent === undefined ? null : String(exec.agent.id)
    if (sessionId === null) {
      noteDegradation(
        null,
        GUARD_RULE_IDS.unknownScope,
        `${exec.name}: 调用没有 agent（headless），无法解析 {ns,app}——按未知范围放行`,
      )
      return undefined
    }
    // 修复墙记账与判定（同步面）。
    const budget = budgetFor(sessionId)
    budget.recordCall(exec.name, exec.arguments)
    const decision = budget.decide()
    if (decision !== 'allow') {
      const evidence = `${wallTarget(sessionId)}；触发：同一调用签名连续重复或同一错误签名达到上限`
      ctx.logger.warn(`guard-alioth: 修复墙 ${decision}（${exec.name}）`)
      return wallFeedback(decision, exec.name, evidence)
    }
    const writePath = writePathOf(exec.name, exec.arguments)
    if (writePath === undefined && !enforceToolSurface) {
      return undefined
    }
    const scope = resolver.cachedScope(sessionId)
    if (scope === null) {
      // workflow 元工具是范围建立调用，自身免判定；其余读面在下方读围栏判定。
      if (!WORKFLOW_SCOPE_TOOLS.includes(exec.name)) {
        if (readFence) {
          const readPaths = readPathsOf(exec.name, exec.arguments)
          if (readPaths !== undefined && readPaths.length > 0) {
            // 读围栏的 namespace 后备通道：workflow 范围缺席时用会话绑定账户的
            // namespace（agent/pre-step 已填充缓存）；无缓存（headless/未绑定）→
            // 显式降级放行，与写沙箱同一姿态。
            const namespace = sessionNamespace.get(sessionId) ?? null
            if (namespace !== null && modelDirCache !== null) {
              const fence = readVerdict(readPaths, namespace, preProcRoot, path.resolve(modelDirCache))
              if (!fence.ok) {
                ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（读围栏）`)
                return fence.reason
              }
              return undefined
            }
            noteDegradation(
              sessionId,
              GUARD_RULE_IDS.unknownScope,
              `${exec.name}: 无法解析会话命名空间（无 workflow 范围与账户绑定）——读围栏按未知范围放行`,
            )
          }
        }
        noteDegradation(
          sessionId,
          GUARD_RULE_IDS.unknownScope,
          `${exec.name}: 本会话没有 alioth_workflow_* 调用记录，无法解析 {ns,app}——按未知范围放行（不猜 ns）`,
        )
      }
      return undefined
    }
    if (readFence) {
      const readPaths = readPathsOf(exec.name, exec.arguments)
      if (readPaths !== undefined && readPaths.length > 0 && modelDirCache !== null) {
        const fence = readVerdict(readPaths, scope.namespace, preProcRoot, path.resolve(modelDirCache))
        if (!fence.ok) {
          ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（读围栏）`)
          return fence.reason
        }
      }
    }
    if (enforceToolSurface && scope.stepId !== null) {
      const verdict = toolSurfaceVerdict(exec.name, scope.allowedTools)
      if (!verdict.ok) {
        ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（工具面）`)
        return verdict.reason
      }
    }
    if (writePath !== undefined) {
      const sandbox = sandboxVerdict(writePath, scope.namespace, preProcRoot)
      if (!sandbox.ok) {
        ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（写沙箱）`)
        return sandbox.reason
      }
      if (scope.stepId !== null && scope.phase === 'plan') {
        const plan = planWriteVerdict(writePath, scope.namespace, preProcRoot, scope.planWriteGlobs)
        if (!plan.ok) {
          ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（plan 写面）`)
          return plan.reason
        }
      }
    }
    return undefined
  }

  ctx.tools.guard(exec => adjudicate(exec))

  // ── 执行前（步骤级）：账户绑定/月度预算、turn 预算阻断、闭环证据追问 ─────
  ctx.on('agent/pre-step', async ({ agent, turn }, next) => {
    const sessionId = String(agent.id)
    // 步进缝刷新两份同步判定的缓存：范围快照（步进/推进后立即新）与
    // 会话命名空间（读围栏的后备通道）。
    void resolver.refresh(sessionId)
    const boundAccount = await resolveAccountContext(sessionId)
    if (boundAccount !== null) sessionNamespace.set(sessionId, boundAccount.namespace)
    const violation: TurnViolation | null = ledger.takeViolation(sessionId)
    if (violation !== null) {
      ctx.logger.warn(`guard-alioth: 阻断下一步（turn 预算）${violation.reason}`)
      return { kind: 'reject' }
    }
    // 账户月度预算（C2）：auth 缺席 / 无身份 / 部署未配配额 ⇒ 不判；成本口径
    // 不可得（存在无价模型）⇒ 留降级证据不判——两者都绝不以 0 冒充。
    const account = await resolveAccountContext(sessionId)
    meter.setAccount(sessionId, account?.namespace ?? null)
    if (account !== null && account.monthlyCostCents !== null) {
      const budget = account.monthlyCostCents
      const persisted = await meter.monthCost(account.namespace)
      if (persisted === null) {
        noteDegradation(
          sessionId,
          GUARD_RULE_IDS.accountBudget,
          `账户 ${account.namespace} 的月度成本口径不可得（存在无价模型或无落账面）——不判预算`,
        )
      } else {
        const inflight = meter.unflushedCost(sessionId, ledger.usageSlice(sessionId, meter.offsetOf(sessionId)))
        if (inflight !== null && persisted + inflight > budget) {
          const reason = guardFeedback({
            ruleId: GUARD_RULE_IDS.accountBudget,
            repairClass: 'not-fixable',
            message: `账户 ${account.namespace} 本月估算成本 ${persisted + inflight} 分已超过月度预算 ${budget} 分`,
            action: '停止继续调用模型；需要更高预算时由运营调整该账户的月度配额',
            evidence: `persisted=${persisted} sessionInflight=${inflight} quota=${budget}`,
          })
          ctx.logger.warn(`guard-alioth: 阻断下一步（账户预算）${reason}`)
          return { kind: 'reject' }
        }
      }
    }
    const decision: NudgeDecision = decideClosureNudge(
      nudgeEnabled,
      ledger.turnRecord(sessionId, turn),
    )
    if (!decision.nudge) {
      return next()
    }
    const admitted = await next()
    if (admitted.kind !== 'enter') {
      return admitted
    }
    ledger.markNudged(sessionId, turn)
    ctx.logger.warn(`guard-alioth: 注入闭环证据追问（turn ${turn}）`)
    return {
      kind: 'enter',
      messages: [
        ...admitted.messages,
        createUserMessage({
          content: [{ type: 'text', text: decision.text }],
          source: { kind: 'alioth-guard' },
        }),
      ],
      ...admitted.startsRequestSeries === true ? { startsRequestSeries: true as const } : {},
    }
  })

  ctx.provide('aliothGuard', aliothGuard)
}

/** 从 tool/result 的 content 块提取错误文本（修复墙的错误签名口径）。 */
function errorTextOf(content: unknown): string {
  if (typeof content !== 'object' || content === null || !Array.isArray(content)) return 'tool error'
  return content
    .filter((block): block is { type: string; text?: string } =>
      typeof block === 'object' && block !== null && (block as { type?: string }).type === 'text')
    .map(block => block.text ?? '')
    .join('\n')
    .slice(0, 400) || 'tool error'
}

/** 会话事件里的工具参数是模型原始 JSON 字符串（未解析）；坏输入按「无参数」处理。 */
function parseArguments(raw: string): unknown {
  try {
    return JSON.parse(raw)
  } catch {
    return undefined
  }
}

export { SessionLedger, type SessionLedgerOptions, type TurnLedger, type TurnViolation } from './ledger.ts'
export { AccountMeter, type AccountMeterOptions, type MeterFlushEntry, type MeteringSink } from './metering.ts'
export { decideClosureNudge, CLOSURE_NUDGE_TEXT, type NudgeDecision, type NudgeView } from './nudge.ts'
export { GUARD_RULE_IDS, guardDenyReason, guardFeedback, type GuardRuleId } from './rules.ts'
export { RunScopeResolver, WORKFLOW_SCOPE_TOOLS, type ActiveScope, type ScopeKey } from './scope.ts'
export {
  declaredToolSurface,
  fullAccessVerdict,
  pathOrigin,
  planWriteVerdict,
  readPathsOf,
  readVerdict,
  sandboxVerdict,
  toolSurfaceVerdict,
  writePathOf,
  META_TOOL_PREFIX,
  SANDBOX_WRITE_BLACKLIST,
  SANDBOX_ZONES,
  WRITE_TOOL_PATH_ARG,
  type Adjudication,
  type PathOrigin,
} from './surface.ts'
export {
  classifyRuntimeMirror,
  readWhitelistSource,
  RUNTIME_MIRROR_PATH,
  type AllowedProgramsSource,
  type WhitelistSourceReport,
} from './whitelist.ts'
