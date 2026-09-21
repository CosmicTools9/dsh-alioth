/**
 * 扩展验证**降级人工门**（deferred but adjudicated）——对齐上游
 * `Meta/backend/app-agent/src/dialog_tools/verify_extensions.rs:36-50,107-136`：
 * 降级运行只写 `extension-verify.degraded.json`，**永不**占 canonical `extension-verify.json`，
 * 因此这条门的解除条件只能是 canonical 文件里 `/status == "passed"`——**降级运行无法自我解除**
 * （上游注释原话：「降级运行只写 .degraded.json，故不会自我解除」）。没有这一步，上一轮遗留的
 * canonical `passed` 就会把新一次 degraded 运行放过去，fail-closed 形同虚设。
 *
 * 门的键是**按 app**（不是按 session）：`sessionId = app-extensions-{namespace}-{app}`
 * （deferred store 的 sessionId 直接参与落盘文件名，故只由合法标识符拼成），
 * 这样任何一次 `alioth_deferred list` 都能看到未决的人工门，而不必知道登记者会话。
 * @module @dsh-alioth/tool-alioth-verify/extensions-gate
 */

import path from 'node:path'
import type { DeferredItem } from '@dsh-alioth/verify-alioth'

/** app 级门的作用域前缀（`alioth_deferred list` 依此识别人工门）。 */
export const EXTENSIONS_GATE_PREFIX = 'app-extensions-'

/** canonical 证据文件名（发布前置读取的「通过」证据）。 */
const EXTENSION_VERIFY_CANONICAL = 'extension-verify.json'

/** 门的作用域 id（= deferred store 的 `sessionId` 键；只含合法标识符字符）。 */
export function extensionsGateScope(namespace: string, app: string): string {
  return `${EXTENSIONS_GATE_PREFIX}${namespace}-${app}`
}

/**
 * 门条目：解除条件 = canonical `extension-verify.json` 的 `/status` 等于 `passed`
 * （`artifact-json-pointer` 触发：文件缺失 / 不可解析 / pointer 缺失 / 不等 → 未解除，fail-closed）。
 */
export function extensionsGateItem(input: {
  readonly appDir: string
  readonly namespace: string
  readonly app: string
  readonly uncovered: readonly string[]
}): DeferredItem {
  const shown = input.uncovered.slice(0, 3).join('；')
  const rest = input.uncovered.length > 3 ? `（另 ${input.uncovered.length - 3} 条见降级报告）` : ''
  return {
    id: `extensions-degraded:${input.namespace}/${input.app}`,
    sessionId: extensionsGateScope(input.namespace, input.app),
    reason: `extensions 验证降级：${input.uncovered.length} 条声明未被 Gateway loader 覆盖——${shown}${rest}`,
    adjudication: '未覆盖的声明无法离线执行验证：形态/接线需人工确认，确认后方可上线（degraded ≠ passed，不得自动放行）',
    trigger: {
      kind: 'artifact-json-pointer',
      path: path.join(input.appDir, EXTENSION_VERIFY_CANONICAL),
      pointer: '/status',
      equals: 'passed',
    },
    successors: [
      '人工复核未覆盖声明（明细见 extension-verify.degraded.json 的 declarations）',
      '修正或删除 loading 不认的声明后重跑 alioth_verify extensions（只有真实通过才解除本门）',
    ],
    createdTs: new Date().toISOString(),
  }
}
