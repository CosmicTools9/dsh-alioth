/**
 * 产物落点与标识符纪律——本包所有工具读写的 App 目录都是
 * `{preProcRoot}/{namespace}/Apps/{app}`（部署前缀解析后的 `Pre-Proc/{ns}/Apps/{app}`），
 * 与 `tool-alioth-workflow` / `tool-alioth-orchestrator` 同一布局口径。
 * namespace/app 按 workflow 插件同一模式校验（拒绝穿越、绝对路径与大小写异常）。
 * @module @dsh-alioth/tool-alioth-verify/paths
 */

import path from 'node:path'

const NAMESPACE_PATTERN = /^[A-Z][a-zA-Z0-9-]*$/
const APP_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/** 必填字符串参数（缺失/非字符串/纯空白即 throw，错误文案带工具名与字段名）。 */
export function requireString(args: Record<string, unknown>, field: string, tool: string): string {
  const value = args[field]
  if (typeof value !== 'string' || value.trim() === '') {
    throw new Error(`${tool}: "${field}" 必填且必须是非空字符串`)
  }
  return value
}

/** 校验 namespace/app（与 workflow 插件的模式一致）。 */
export function assertNamespaceApp(namespace: string, app: string, tool: string): void {
  if (!NAMESPACE_PATTERN.test(namespace)) {
    throw new Error(`${tool}: 非法 namespace ${JSON.stringify(namespace)}（期望 ^[A-Z][a-zA-Z0-9-]*$）`)
  }
  if (!APP_PATTERN.test(app)) {
    throw new Error(`${tool}: 非法 app ${JSON.stringify(app)}（期望 ^[a-zA-Z0-9][a-zA-Z0-9-]*$）`)
  }
}

/** `Pre-Proc/{namespace}` 根（7 阶段判据里 `ontology-mapping` 的 `preProcRoot` 口径）。 */
export function namespaceRootOf(preProcRoot: string, namespace: string): string {
  return path.join(preProcRoot, namespace)
}

/** `Pre-Proc/{namespace}/Apps/{app}`。 */
export function appDirOf(preProcRoot: string, namespace: string, app: string): string {
  return path.join(namespaceRootOf(preProcRoot, namespace), 'Apps', app)
}

/**
 * 解析补丁目标路径：相对 `appDir` 解析，结果 MUST 严格落在 `appDir` 内
 * （拒绝 `..` 穿越、绝对路径与指向目录本身的路径）。
 */
export function resolveAssetTarget(appDir: string, target: string, tool: string): string {
  const abs = path.resolve(appDir, target)
  const rel = path.relative(appDir, abs)
  if (rel === '' || rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error(
      `${tool}: target 必须落在 App 产物目录内（${appDir}）：拒绝 ${JSON.stringify(target)}`,
    )
  }
  return abs
}

/**
 * 会话 id 解析：显式参数优先，其次取本调用的 agent 归属（`exec.agent.id`）。
 * 两者都不可得（headless / 无 agent 的子派发）→ throw，不猜会话。
 */
export function sessionIdOf(exec: { readonly agent?: { readonly id?: unknown } | undefined }, explicit?: unknown): string {
  const inline = typeof explicit === 'string' ? explicit.trim() : ''
  if (inline !== '') return inline
  const fromAgent = exec.agent?.id
  if (fromAgent !== undefined && String(fromAgent).trim() !== '') return String(fromAgent)
  throw new Error(
    '本调用无可解析的会话归属（exec.agent 缺失）且未显式给出 sessionId——会话级事实不可得时 MUST NOT 猜测',
  )
}
