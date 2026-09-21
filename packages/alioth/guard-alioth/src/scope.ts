/**
 * 会话 → 运行范围（scope）解析。
 *
 * 【设计事实】run state 以 `{namespace, app}` 为键落在
 * `<workflowRoot>/{ns}/{app}/run-state.json`（`skill-alioth/src/workspace.ts`），
 * **不按 session 键**。因此解析必须两步：
 *
 * 1. 从 `session/event` 观察本会话最近一次 `alioth_workflow_*` 调用的参数，
 *    缓存 `sessionId → {namespace, app}`（调用参数是唯一的会话侧线索）；
 * 2. 用 `loadAdapter(modelDir)` + `loadRun(workflowRoot, {ns,app}, adapter)` **现取**
 *    当前 step，从 step 的 `phase` / `output_glob` / 工具声明派生 allowedTools 与
 *    planWriteGlobs——每次判定现读磁盘，绝不用过期缓存。
 *
 * 本会话没有任何 workflow 调用记录 → `null`（= 未知范围）：调用点按契约 §3 显式降级
 * （放行 + 留痕），**绝不猜 ns**。范围已知但 run state / adapter 读不出来时，namespace
 * 仍然可信，故返回「无当前步骤」的范围（`stepId: null`：跳过工具面与 plan 判定，
 * 沙箱仍生效）并留一条降级证据——降级不得只活在日志里。
 * @module @dsh-alioth/guard-alioth/scope
 */

import { currentStep, loadAdapter, loadRun, type StepPhase } from '@dsh-alioth/skill-alioth'
import { declaredToolSurface } from './surface.ts'

/** 运行范围键（run state 的键）。 */
export interface ScopeKey {
  readonly namespace: string
  readonly app: string
}

/**
 * 当前生效范围。
 *
 * 契约 §3 的形状（`namespace` / `app` / `phase` / `allowedTools` / `planWriteGlobs`）
 * 是子集，此处加性扩展 `stepId`：`null` 表示运行已结束或步骤不可读——调用点据此
 * 跳过「当前步骤存在时」才成立的判定，而不是把空允许清单误解为「全部禁止」。
 */
export interface ActiveScope {
  readonly namespace: string
  readonly app: string
  readonly phase: StepPhase
  readonly allowedTools: readonly string[]
  readonly planWriteGlobs: readonly string[]
  readonly stepId: string | null
}

/** 触发范围刷新的元工具（都带 `{namespace, app}` 参数）。 */
export const WORKFLOW_SCOPE_TOOLS: readonly string[] = ['alioth_workflow_step', 'alioth_workflow_complete']

/** 解析器的外部依赖（IO 与留痕都在边界上）。 */
export interface ScopeDeps {
  /** 模型快照目录（`ctx.aliothEnv.ready().modelDir`）。 */
  readonly modelDir: () => Promise<string>
  /** run state 根（`<dataRoot>/workflows`）。 */
  readonly workflowRoot: () => string
  /** 适配器文件名（`skill-adapters/` 下）。 */
  readonly adapter: string
  /** 范围已解析但步骤不可读时的显式降级留痕。 */
  readonly onDegrade: (sessionId: string, key: ScopeKey, reason: string) => void
}

/** adapter 模板变量：`{ns}` / `{module}` / `{app}` 等按 run 的键解析（与 workflow 插件同口径）。 */
function scopeVariables(key: ScopeKey): Record<string, string> {
  return {
    ns: key.namespace,
    app: key.app,
    module: key.app,
    service: key.app,
    block: key.app,
    crate: `alioth-service-${key.app}`,
  }
}

/** 解析 adapter 模板（`{ns}` / `{module}` / `{app}`），未注册的占位符原样保留。 */
function resolveTemplate(template: string, variables: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => variables[key] ?? match)
}

/** 会话范围解析器：会话键缓存 + 每次判定现读 run state。 */
export class RunScopeResolver {
  private readonly keys = new Map<string, ScopeKey>()
  private readonly deps: ScopeDeps

  constructor(deps: ScopeDeps) {
    this.deps = deps
  }

  /**
   * 登记一次工具调用；只有 workflow 元工具携带 `{namespace, app}` 时更新缓存。
   * 参数解析失败或字段缺失一律忽略（不猜）。
   */
  observeCall(sessionId: string, tool: string, args: unknown): void {
    if (!WORKFLOW_SCOPE_TOOLS.includes(tool) || typeof args !== 'object' || args === null) {
      return
    }
    const record = args as Record<string, unknown>
    const namespace = record.namespace
    const app = record.app
    if (typeof namespace !== 'string' || typeof app !== 'string' || namespace === '' || app === '') {
      return
    }
    this.keys.set(sessionId, { namespace, app })
  }

  /** 缓存的会话范围键（无 workflow 调用记录 → `null`）。 */
  sessionScope(sessionId: string): ScopeKey | null {
    return this.keys.get(sessionId) ?? null
  }

  /**
   * 现取当前范围：读 adapter + run state，派生阶段、工具面与 plan 写面。
   * @param sessionId - 会话 id（`agent.id` / `session.id`）。
   * @returns 未知范围 → `null`；步骤不可读 → `stepId: null` 的范围 + 降级留痕。
   */
  async activeScope(sessionId: string): Promise<ActiveScope | null> {
    const key = this.keys.get(sessionId)
    if (key === undefined) {
      return null
    }
    try {
      const adapter = await loadAdapter(await this.deps.modelDir(), this.deps.adapter)
      const run = await loadRun(this.deps.workflowRoot(), key, adapter)
      const current = currentStep(run)
      if (current === undefined) {
        return {
          namespace: key.namespace,
          app: key.app,
          phase: 'apply',
          allowedTools: [],
          planWriteGlobs: [],
          stepId: null,
        }
      }
      const variables = scopeVariables(key)
      const outputGlobs = current.step.gates.flatMap(gate =>
        gate.outputGlob === undefined ? [] : [resolveTemplate(gate.outputGlob, variables)])
      return {
        namespace: key.namespace,
        app: key.app,
        phase: current.step.phase,
        allowedTools: declaredToolSurface(adapter, current.step),
        planWriteGlobs: current.step.phase === 'plan' ? outputGlobs : [],
        stepId: current.step.id,
      }
    } catch (error) {
      this.deps.onDegrade(
        sessionId,
        key,
        `adapter/run state 不可读（${error instanceof Error ? error.message : String(error)}）：`
        + '跳过工具面与 plan 判定，写沙箱仍按已知 namespace 生效',
      )
      return {
        namespace: key.namespace,
        app: key.app,
        phase: 'apply',
        allowedTools: [],
        planWriteGlobs: [],
        stepId: null,
      }
    }
  }
}
