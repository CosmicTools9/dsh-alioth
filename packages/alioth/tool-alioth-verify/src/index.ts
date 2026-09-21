/**
 * `@dsh-alioth/tool-alioth-verify`——模型面验证工具（Wave 2 / T5）。
 *
 * 七个 `action` 风格工具，全部是 `@dsh-alioth/verify-alioth` 的**薄封装**（零 LLM：模型面只读
 * 事实与执行两段式动作，不做语义判断）：
 *
 * | 工具 | 动作 | 落盘 |
 * |---|---|---|
 * | `alioth_verify` | `artifacts` / `extensions` / `stage` | `eval-report.json` / `extension-verify.json` |
 * | `alioth_closure` | `verdict` / `status` | `AppAgentTraces/closure-audit/{seq}.json`（append-only） |
 * | `alioth_version` | `snapshot` / `list` / `rollback`（需 `confirmed: true`） | `versions/{seq:04}-{hash8}/` |
 * | `alioth_patch_assets` | `propose` / `apply`（需 `confirmed: true`） | 目标产物文件（原子写回） |
 * | `alioth_capabilities` | 六组只读能力广告 | 无（零副作用） |
 * | `alioth_deferred` | `register` / `list` / `unlock` | `<dataRoot>/deferred/{scope}.json`（scope = 会话 id 或 app 门 `app-extensions-{ns}-{app}`） |
 * | `alioth_usage` | 会话用量/成本 | 无（零副作用） |
 *
 * 全局纪律（MUST NOT 软化）：两段式的第二段未确认**不写盘**；`degraded ≠ passed`；
 * 不可得显式 `unknown(reason)`；缺失产物/证据一律不通过，绝不静默放行。
 * @module @dsh-alioth/tool-alioth-verify
 */

import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
// 类型锚点：`ctx.aliothEnv` 的模块增强随本包的这一导入进入类型程序（孤立 `tsc -p` 也能解析）。
import type { AliothEnv } from '@dsh-alioth/env-alioth'
import { createDeferredStore } from '@dsh-alioth/verify-alioth'
import { registerAssetTools } from './asset-tools.ts'
import { registerCapabilitiesTool } from './capabilities-tool.ts'
import { registerDeferredTool } from './deferred-tool.ts'
import { registerUsageTool } from './usage-tool.ts'
import { registerVerifyTools } from './verify-tools.ts'

export const name = 'tool-alioth-verify'
export const inject = ['tools', 'aliothEnv']

/** 适配器文件名的部署缺省（与 `tool-alioth-workflow` / `guard-alioth` 同一取值）。 */
export const DEFAULT_ADAPTER = 'alioth-app.yaml'

/** 验证工具的部署选择（全部经 schemastery 校验；无硬编码开关）。 */
export interface Config {
  /** Pre-Proc 产物树根（env `ALIOTH_PRE_PROC_ROOT`）：App 目录 = `{preProcRoot}/{namespace}/Apps/{app}`。 */
  readonly preProcRoot: string
  /** 阻塞登记根；缺省 = 部署状态根（落 `{root}/deferred/{sessionId}.json`，app 级人工门同库）。 */
  readonly deferredRoot?: string
  /** 适配器文件名（模型快照 `skill-adapters/` 下），用于能力广告的 plan 步判定；缺省 `alioth-app.yaml`。 */
  readonly adapter?: string
  /** 价表 JSON 路径（`{模型: {centsPerInK, centsPerOutK}}`）；缺省不给成本估算（显式 `unavailable`）。 */
  readonly priceTable?: string
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  deferredRoot: z.string(),
  adapter: z.string().default(DEFAULT_ADAPTER),
  priceTable: z.string(),
})

export function apply(ctx: Context, config: Config): void {
  const preProcRoot = path.resolve(config.preProcRoot)
  const adapterName = config.adapter ?? DEFAULT_ADAPTER
  // `inject` 已声明 aliothEnv（服务必在位），按名读取需从 `AliothEnv | undefined` 窄化。
  const env = ctx.get('aliothEnv') as AliothEnv
  const dataRoot = env.dataRoot()
  const deferred = createDeferredStore(config.deferredRoot ?? dataRoot)

  registerVerifyTools(ctx, { preProcRoot, deferred })
  registerAssetTools(ctx, { preProcRoot })
  registerDeferredTool(ctx, { store: deferred })
  registerUsageTool(ctx, config.priceTable === undefined ? {} : { priceTable: config.priceTable })
  registerCapabilitiesTool(ctx, {
    env,
    deferred,
    adapterName,
    workflowRoot: () => path.join(dataRoot, 'workflows'),
  })
}
