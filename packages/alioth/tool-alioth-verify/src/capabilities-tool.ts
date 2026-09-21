/**
 * `alioth_capabilities`——六组只读能力广告（对齐上游 `capabilities.rs`）：
 * `tools` / `skills` / `gates` / `autonomy` / `llm` / `budget`，**每组字段必须在场**，
 * 不可得 MUST 显式 `unknown(reason)`（MUST NOT 空值 / 省略 / 猜测）。组级错误隔离由
 * `verify-alioth/collectCapabilities` 负责（一组失败不影响其余组）。
 *
 * 各组的事实来源（无来源即显式 unknown，本模块不编造）：
 * - `tools` ← 本部署工具注册表（`ctx.tools.schemas()`）。
 * - `skills` ← 模型快照 `skill-adapters/*.yaml`（真解析器 `parseAdapterDocument`）+ 漂移基线
 *   `PROVENANCE.json` 的字节 sha256：**未登记或指纹不符 = drifted**（漂移警报宁可误报、不可漏报）；
 *   快照缺 `skill-adapters/` 或 `PROVENANCE.json` → 该组 unknown。
 * - `gates` ← 守卫（`aliothGuard`）降级证据 + 阻塞登记（`alioth_deferred` 的未解除项）+
 *   运行状态里的未完成 plan 步（只读枚举 `<dataRoot>/workflows/{ns}/{app}/run-state.json`；
 *   **MUST NOT 用 `loadRun`**——它会为缺失状态新建 run）。`human` 恒为空集：适配器门禁词汇
 *   （`skill-alioth` 的 `StepGate`）只有 `output-glob`/`program` 两形态，本部署无人审门禁声明。
 *   守卫未装配 → 该组 unknown（降级证据不可得）。
 * - `autonomy` / `budget` ← 本部署无来源（上游 `autonomy.yaml` 在本仓库只作为写黑名单；
 *   单 turn 预算上限属守卫部署配置，未在服务面暴露）→ 显式 `unknown(reason)`。
 * - `llm` ← 可选 `llm` 服务的 `listProviders()`（就绪性 = 有 provider 路由）；未装配 → unknown。
 *
 * 另附顶层 `guard` 字段承载守卫的**门禁白名单生效来源**（`whitelistSource()`：`file` /
 * `code_default` + 可区分原因，上游 `add-appagent-degradation-evidence` 的落地）与降级明细；
 * 守卫未装配 → 同样显式 unknown。`notes` 记录本次读取的保守口径（损坏状态按未完成计入等）。
 * @module @dsh-alioth/tool-alioth-verify/capabilities-tool
 */

import { createHash } from 'node:crypto'
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import type { AliothEnv } from '@dsh-alioth/env-alioth'
import { loadAdapter, parseAdapterDocument, type Adapter } from '@dsh-alioth/skill-alioth'
import {
  collectCapabilities,
  type CapabilityValue,
  type DeferredStore,
  type SkillFact,
} from '@dsh-alioth/verify-alioth'
import { GUARD_ABSENT_REASON, guardOf, type GuardServiceLike, type GuardWhitelistSource } from './guard-source.ts'
import { capabilityGroupOutput } from './json-output.ts'

/** 门禁白名单来源与降级的显式不可得原因。 */
const GATES_NO_GUARD_REASON =
  `${GUARD_ABSENT_REASON}——gates 组的降级证据（degraded）由守卫提供`

/** 本部署无自主政策来源（可区分原因，不编造等级）。 */
const AUTONOMY_NO_SOURCE_REASON =
  '本部署无自主政策来源：上游 autonomy.yaml 在本仓库只作为写黑名单（guard-alioth/src/surface.ts），'
  + '无等级/失败闭合声明可读'

/** 单 turn 预算上限未在守卫服务面暴露（只有 usage 的 turn 计数）。 */
const BUDGET_NO_SOURCE_REASON =
  'aliothGuard 服务面未暴露单 turn 预算上限（maxSteps/turnTimeoutSec 属守卫部署配置）：'
  + '仅有 usage() 的 turn 计数不足以构成 budget 事实（MUST NOT 以 0/默认值冒充）'

/** 人审门禁面恒为空集的口径说明（写入 notes）。 */
const HUMAN_GATES_NOTE =
  'human：适配器门禁词汇（skill-alioth StepGate）仅 output-glob/program 两形态，本部署无人审门禁声明'

/** 工具参数 schema 的六组共享形态（`CapabilityValue<T>` 的三态）。 */
const CAPABILITY_VALUE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    kind: { type: 'string', required: true },
    reason: { type: 'string' },
    value: { type: 'json' },
  },
} as const

/** 只读枚举运行状态，统计「当前步是 plan 相位且未完成」的 run 数（保守：损坏状态按未完成计入）。 */
async function countPendingPlans(workflowRoot: string, adapter: Adapter, notes: string[]): Promise<number> {
  const namespaceDirs = await readdir(workflowRoot, { withFileTypes: true }).catch(() => null)
  if (namespaceDirs === null) return 0
  let pending = 0
  for (const namespaceEntry of namespaceDirs) {
    if (!namespaceEntry.isDirectory()) continue
    const appDirs = await readdir(path.join(workflowRoot, namespaceEntry.name), { withFileTypes: true }).catch(() => null)
    if (appDirs === null) continue
    for (const appEntry of appDirs) {
      if (!appEntry.isDirectory()) continue
      const file = path.join(workflowRoot, namespaceEntry.name, appEntry.name, 'run-state.json')
      const text = await readFile(file, 'utf8').catch(() => null)
      if (text === null) continue
      let parsed: unknown
      try {
        parsed = JSON.parse(text) as unknown
      } catch {
        pending += 1
        notes.push(`run-state 不可解析，按未完成计划计入（欠报会放行未完成工作）：${file}`)
        continue
      }
      const record = parsed as { position?: { trackIndex?: unknown; stepIndex?: unknown }; completed?: unknown }
      const trackIndex = record.position?.trackIndex
      const stepIndex = record.position?.stepIndex
      const completed = record.completed
      if (
        typeof trackIndex !== 'number' || !Number.isInteger(trackIndex)
        || typeof stepIndex !== 'number' || !Number.isInteger(stepIndex)
        || !Array.isArray(completed) || completed.some(id => typeof id !== 'string')
      ) {
        pending += 1
        notes.push(`run-state 形态非法，按未完成计划计入：${file}`)
        continue
      }
      const step = adapter.tracks[trackIndex]?.steps[stepIndex]
      if (step === undefined) {
        notes.push(`run-state 位置越界（${namespaceEntry.name}/${appEntry.name}）：track ${trackIndex} step ${stepIndex} 无对应步`)
        continue
      }
      if (step.phase === 'plan' && !(completed as readonly string[]).includes(step.id)) pending += 1
    }
  }
  return pending
}

/** 模型快照的适配器事实 + 漂移判定（PROVENANCE.json 字节 sha256 对照）。 */
async function collectSkillFacts(modelDir: string): Promise<readonly SkillFact[]> {
  const dir = path.join(modelDir, 'skill-adapters')
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) throw new Error(`模型快照缺 skill-adapters/：${dir}`)
  const provenancePath = path.join(modelDir, 'PROVENANCE.json')
  const provenanceText = await readFile(provenancePath, 'utf8').catch(() => null)
  if (provenanceText === null) throw new Error(`漂移基线不可得：${provenancePath} 缺失/不可读`)
  const provenance = JSON.parse(provenanceText) as { files?: Record<string, string> }
  const recorded = provenance.files ?? {}

  const facts: SkillFact[] = []
  for (const entry of entries) {
    if (!entry.isFile()) continue
    if (!entry.name.endsWith('.yaml') && !entry.name.endsWith('.yml')) continue
    // `_runtime.yaml` 是门禁程序白名单镜像，不是技能适配器。
    if (entry.name.startsWith('_')) continue
    const source = await readFile(path.join(dir, entry.name), 'utf8')
    const adapter = parseAdapterDocument(source, path.join('skill-adapters', entry.name))
    const sha256 = createHash('sha256').update(source).digest('hex')
    facts.push({
      name: adapter.name,
      version: adapter.version,
      drifted: recorded[`skill-adapters/${entry.name}`] !== sha256,
    })
  }
  if (facts.length === 0) throw new Error(`模型快照 ${dir} 下无适配器可广告（空清单不得冒充完整能力面）`)
  return facts.sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0))
}

/** 注册 `alioth_capabilities`。 */
export function registerCapabilitiesTool(
  ctx: Context,
  options: {
    readonly env: AliothEnv
    readonly deferred: DeferredStore
    readonly workflowRoot: () => string
    readonly adapterName: string
  },
): void {
  ctx.tools.register(defineTool({
    name: 'alioth_capabilities',
    description:
      'Deployment capability advertisement (six read-only groups: tools / skills / gates / autonomy / llm / budget). '
      + 'Every group is ALWAYS present as either {kind:"value", value} or {kind:"unknown", reason} — an unavailable '
      + 'fact is never omitted, never guessed and never a default (a group whose source is missing reports why). '
      + 'Skills report `drifted` against the model snapshot PROVENANCE.json bytes; gates carry the execution guard\'s '
      + 'degradation evidence, open deferred blockers and pending plan steps. The extra `guard` field carries the '
      + 'gate-program whitelist provenance (file vs code_default). Read-only: nothing is written.',
    parameters: {},
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          tools: CAPABILITY_VALUE_SCHEMA,
          skills: CAPABILITY_VALUE_SCHEMA,
          gates: CAPABILITY_VALUE_SCHEMA,
          autonomy: CAPABILITY_VALUE_SCHEMA,
          llm: CAPABILITY_VALUE_SCHEMA,
          budget: CAPABILITY_VALUE_SCHEMA,
          guard: CAPABILITY_VALUE_SCHEMA,
          notes: { type: 'array', items: { type: 'string' } },
        },
      },
      render: (_args, value) => {
        const groups = ['tools', 'skills', 'gates', 'autonomy', 'llm', 'budget'] as const
        const unknown = groups.filter(group => value[group]?.kind === 'unknown')
        return [{
          type: 'text',
          text: `capabilities: ${groups.length - unknown.length}/${groups.length} groups available`
            + `${unknown.length > 0 ? ` (unknown: ${unknown.join(', ')})` : ''}`,
        }]
      },
    },
    async execute() {
      const notes: string[] = []
      const report = await collectCapabilities({
        tools: () => ctx.tools.schemas().map(schema => schema.name).sort(),
        skills: async () => collectSkillFacts((await options.env.ready()).modelDir),
        gates: async () => {
          const guard = guardOf(ctx)
          if (guard === undefined) throw new Error(GATES_NO_GUARD_REASON)
          const adapter = await loadAdapter((await options.env.ready()).modelDir, options.adapterName)
          notes.push(HUMAN_GATES_NOTE)
          return {
            degraded: guard.degradations().map(entry => `[rule:${String(entry.ruleId)}] ${String(entry.reason)}`),
            human: [],
            deferredOpen: (await options.deferred.all()).length,
            plansPending: await countPendingPlans(options.workflowRoot(), adapter, notes),
          }
        },
        autonomy: () => {
          throw new Error(AUTONOMY_NO_SOURCE_REASON)
        },
        llm: () => {
          const llm = ctx.get('llm') as { listProviders?: () => readonly unknown[] } | undefined
          if (llm === undefined || typeof llm.listProviders !== 'function') {
            throw new Error('llm 服务未装配或未提供 listProviders()：模型面就绪性不可得')
          }
          const providers = llm.listProviders()
          return { ready: providers.length > 0, probe: `llm.listProviders() → ${providers.length} 条 provider 路由` }
        },
        budget: () => {
          throw new Error(BUDGET_NO_SOURCE_REASON)
        },
      })

      const guard: GuardServiceLike | undefined = guardOf(ctx)
      const guardFact: CapabilityValue<{
        readonly whitelistSource: GuardWhitelistSource
        readonly degradations: readonly { ruleId: string; reason: string; sessionId: string | null; time: number }[]
      }> = guard === undefined
        ? { kind: 'unknown', reason: GUARD_ABSENT_REASON }
        : {
            kind: 'value',
            value: {
              whitelistSource: await guard.whitelistSource(),
              degradations: guard.degradations().map(entry => ({
                ruleId: String(entry.ruleId),
                reason: String(entry.reason),
                sessionId: entry.sessionId === null ? null : String(entry.sessionId),
                time: Number(entry.time),
              })),
            },
          }
      // 只读库值 → 输出 JSON 口（六组 + guard，逐一窄化；不做深拷贝）。
      return {
        tools: capabilityGroupOutput(report.tools),
        skills: capabilityGroupOutput(report.skills),
        gates: capabilityGroupOutput(report.gates),
        autonomy: capabilityGroupOutput(report.autonomy),
        llm: capabilityGroupOutput(report.llm),
        budget: capabilityGroupOutput(report.budget),
        guard: capabilityGroupOutput(guardFact),
        notes,
      }
    },
    presentCall: () => ({
      card: 'generic',
      title: 'Alioth capabilities',
      kind: 'other',
      rawInput: {},
    }),
  }))
}
