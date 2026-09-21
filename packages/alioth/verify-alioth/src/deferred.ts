/**
 * Deferred But Adjudicated 阻塞登记 —— 对齐上游 `deferred.rs`：
 *
 * - **D1 登记即裁决**：`adjudication` 必填——空串（含纯空白）即 throw；无裁决的挂起 = 遗忘的阻塞；
 * - **D2 触发条件仅磁盘可判定式**：`artifact-exists` / `artifact-fingerprint` /
 *   `artifact-json-pointer`（RFC 6901 取值深比较；拒绝任意表达式求值）；
 * - **D3 会话级作用域**：落 `{root}/deferred/{sessionId}.json`，不跨会话共享；
 * - 触发满足者由 `unlockDue` 解除并返回；`open` 只返回仍未解除者。
 * @module @dsh-alioth/verify-alioth/deferred
 */

import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

export const DEFERRED_SCHEMA_VERSION = '1.0'

/** 触发条件（仅磁盘可判定三式）。 */
export type DeferredTrigger =
  | { readonly kind: 'artifact-exists'; readonly path: string }
  | { readonly kind: 'artifact-fingerprint'; readonly path: string; readonly sha256: string }
  /**
   * 产物 JSON 内容谓词（RFC 6901）：文件存在 **且** JSON 可解析 **且** pointer 处值与
   * `equals` 深相等才解除。上游 `deferred.rs::TriggerCondition::JsonFieldEquals` 同语义——
   * 用于「降级验证门只能由一次真实通过自我解除」（`verify_extensions.rs:36-37`）。
   */
  | {
    readonly kind: 'artifact-json-pointer'
    readonly path: string
    readonly pointer: string
    readonly equals: unknown
  }

/** 挂起项（`adjudication` = 为何挂起合法）。 */
export interface DeferredItem {
  readonly id: string
  readonly sessionId: string
  /**
   * 归属应用 / 命名空间。跨会话可见性靠它：publish 前置按 **App** 扫描所有会话的挂起项
   * （上游 `deferred.rs:267-271` 原话——"会话级扫描会失明：同一 App 在先前会话登记的未决门
   * 对新会话的发布不可见 = 绕过人工门"）。缺省 = 不限定（`sweep` 仍会按触发条件解除）。
   */
  readonly app?: string
  readonly namespace?: string
  readonly reason: string
  readonly adjudication: string
  readonly trigger: DeferredTrigger
  readonly successors: readonly string[]
  readonly createdTs: string
}

/** 会话级阻塞登记表（`all`/`sweep` 跨会话，供 publish 前置按 App 扫描）。 */
export interface DeferredStore {
  register(item: DeferredItem): Promise<void>
  open(sessionId: string): Promise<readonly DeferredItem[]>
  unlockDue(sessionId: string): Promise<readonly DeferredItem[]>
  /**
   * 全库扫描：凡触发条件已满足的挂起项一律解除并返回（上游 `deferred.rs:362-385`
   * `scan_and_resolve` 同语义）。**只解除触发条件成立的项**——未成立的挂起项永不自动消失。
   */
  sweep(): Promise<readonly DeferredItem[]>
  all(): Promise<readonly DeferredItem[]>
}

interface DeferredFile {
  readonly schema_version: string
  readonly sessionId: string
  readonly items: readonly DeferredItem[]
}

/** 相对路径按 store root 解析（绝对路径原样用）——触发条件 MUST NOT 依赖进程 cwd。 */
function resolveTarget(root: string, target: string): string {
  return path.isAbsolute(target) ? target : path.join(root, target)
}

function sha256Hex(bytes: Buffer): string {
  return createHash('sha256').update(bytes).digest('hex')
}

/** RFC 6901 取值：`''` = 整份文档；`/a/0/b` 逐段下降（`~1`→`/`、`~0`→`~`）。 */
function resolveJsonPointer(document: unknown, pointer: string): { readonly found: boolean; readonly value: unknown } {
  if (pointer === '') {
    return { found: true, value: document }
  }
  if (!pointer.startsWith('/')) {
    return { found: false, value: undefined }
  }
  let current: unknown = document
  for (const rawToken of pointer.slice(1).split('/')) {
    const token = rawToken.replaceAll('~1', '/').replaceAll('~0', '~')
    if (Array.isArray(current)) {
      if (!/^\d+$/.test(token)) return { found: false, value: undefined }
      const index = Number.parseInt(token, 10)
      if (index >= current.length) return { found: false, value: undefined }
      current = current[index]
      continue
    }
    if (typeof current !== 'object' || current === null || !Object.hasOwn(current, token)) {
      return { found: false, value: undefined }
    }
    current = (current as Record<string, unknown>)[token]
  }
  return { found: true, value: current }
}

/** JSON 深相等（对象键序无关）。 */
function jsonEquals(left: unknown, right: unknown): boolean {
  if (left === right) return true
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false
    return left.every((entry, index) => jsonEquals(entry, right[index]))
  }
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) {
    return false
  }
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  return leftKeys.every(key =>
    Object.hasOwn(right, key)
    && jsonEquals((left as Record<string, unknown>)[key], (right as Record<string, unknown>)[key]))
}

/** 触发条件求值（只读磁盘；不可读 = 未满足，fail-closed）。 */
async function triggerSatisfied(root: string, trigger: DeferredTrigger): Promise<boolean> {
  const target = resolveTarget(root, trigger.path)
  const bytes = await readFile(target).catch(() => null)
  if (bytes === null) return false
  if (trigger.kind === 'artifact-exists') {
    return true
  }
  if (trigger.kind === 'artifact-fingerprint') {
    const expected = trigger.sha256.replace(/^sha256:/, '')
    return sha256Hex(bytes) === expected
  }
  let document: unknown
  try {
    document = JSON.parse(bytes.toString('utf8'))
  } catch {
    // 解析失败 = 未满足：判据是「产物内容说通过」，不是「文件在那儿」。
    return false
  }
  const resolved = resolveJsonPointer(document, trigger.pointer)
  return resolved.found && jsonEquals(resolved.value, trigger.equals)
}

/** 创建会话级阻塞登记表（`root/deferred/{sessionId}.json`）。 */
export function createDeferredStore(root: string): DeferredStore {
  const base = path.resolve(root)
  const dir = path.join(base, 'deferred')

  const sessionFile = (sessionId: string): string => {
    const id = sessionId.trim()
    if (id === '' || id === '.' || id === '..' || id.includes('/') || id.includes('\\')) {
      throw new Error(`sessionId 非法：${JSON.stringify(sessionId)}（会话 id 直接参与落盘路径，拒绝穿越）`)
    }
    return path.join(dir, `${id}.json`)
  }

  const readSession = async (sessionId: string): Promise<readonly DeferredItem[]> => {
    const file = sessionFile(sessionId)
    const text = await readFile(file, 'utf8').catch(() => null)
    if (text === null) return []
    try {
      const parsed = JSON.parse(text) as DeferredFile
      return Array.isArray(parsed.items) ? parsed.items : []
    } catch (error) {
      throw new Error(
        `deferred 记录不可解析（${file}）：${error instanceof Error ? error.message : String(error)}——损坏登记 MUST NOT 被当作「无阻塞」`,
      )
    }
  }

  const writeSession = async (sessionId: string, items: readonly DeferredItem[]): Promise<void> => {
    await mkdir(dir, { recursive: true })
    const target = sessionFile(sessionId)
    const payload: DeferredFile = { schema_version: DEFERRED_SCHEMA_VERSION, sessionId, items }
    const tmp = `${target}.tmp-${process.pid}`
    await writeFile(tmp, `${JSON.stringify(payload, null, 2)}\n`, 'utf8')
    await rename(tmp, target)
  }

  return {
    async register(item: DeferredItem): Promise<void> {
      if (item.adjudication.trim() === '') {
        throw new Error(
          `adjudication 必填（D1）：挂起必须携带裁决——无裁决的延期等于遗忘的阻塞，拒绝登记 ${JSON.stringify(item.id)}`,
        )
      }
      if (item.id.trim() === '') throw new Error('id 必填：挂起项须可在会话内唯一寻址')
      if (item.reason.trim() === '') throw new Error(`reason 必填：${item.id} 须说明挂起原因`)
      if (item.trigger.path.trim() === '') {
        throw new Error(`trigger.path 必填：${item.id} 的触发条件 MUST 是磁盘可判定式`)
      }
      if (item.trigger.kind === 'artifact-fingerprint' && item.trigger.sha256.trim() === '') {
        throw new Error(`trigger.sha256 必填：${item.id} 的指纹触发条件不得为空`)
      }
      if (item.trigger.kind === 'artifact-json-pointer' && item.trigger.pointer !== '' && !item.trigger.pointer.startsWith('/')) {
        throw new Error(`trigger.pointer 非法：${item.id} 的 RFC 6901 pointer 必须为空串或以 / 开头`)
      }
      const existing = await readSession(item.sessionId)
      const next = existing.filter(entry => entry.id !== item.id)
      next.push(item)
      await writeSession(item.sessionId, next)
    },

    async open(sessionId: string): Promise<readonly DeferredItem[]> {
      return readSession(sessionId)
    },

    async unlockDue(sessionId: string): Promise<readonly DeferredItem[]> {
      const items = await readSession(sessionId)
      const due: DeferredItem[] = []
      const remaining: DeferredItem[] = []
      for (const item of items) {
        if (await triggerSatisfied(base, item.trigger)) due.push(item)
        else remaining.push(item)
      }
      if (due.length > 0) await writeSession(sessionId, remaining)
      return due
    },

    async sweep(): Promise<readonly DeferredItem[]> {
      const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
      if (entries === null) return []
      const released: DeferredItem[] = []
      for (const entry of entries) {
        if (!entry.isFile() || !entry.name.endsWith('.json')) continue
        const sessionId = entry.name.slice(0, -'.json'.length)
        const items = await readSession(sessionId)
        if (items.length === 0) continue
        const remaining: DeferredItem[] = []
        for (const item of items) {
          if (await triggerSatisfied(base, item.trigger)) released.push(item)
          else remaining.push(item)
        }
        // 只写回真正解除过的会话：未解除者原地不动（避免无谓重写与 mtime 噪声）。
        if (remaining.length !== items.length) await writeSession(sessionId, remaining)
      }
      return released
    },

    async all(): Promise<readonly DeferredItem[]> {
      const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
      if (entries === null) return []
      const collected: DeferredItem[] = []
      for (const entry of entries) {
        if (!entry.isFile() || !entry.name.endsWith('.json')) continue
        const text = await readFile(path.join(dir, entry.name), 'utf8').catch(() => null)
        if (text === null) continue
        try {
          const parsed = JSON.parse(text) as DeferredFile
          if (Array.isArray(parsed.items)) collected.push(...parsed.items)
        } catch {
          continue
        }
      }
      return collected.sort((a, b) => {
        if (a.createdTs !== b.createdTs) return a.createdTs < b.createdTs ? -1 : 1
        return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
      })
    },
  }
}
