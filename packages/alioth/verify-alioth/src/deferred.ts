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
  /**
   * 产物 JSON **声明**谓词：文件存在、JSON 可解析、且 `pointers` 中**任一** pointer
   * 解析出非空值即满足。用于「人工/模型决定尚未落地」类门——例如 block 的交互形态
   * （`flows` / `workbenchPosts`，二者其一即声明，BLOCK_SCHEMA §1.2 皆 OPTIONAL）。
   * 与 `artifact-json-pointer` 的差别：那条判「等于某个值」，这条判「已声明」；
   * 空串/空数组/空对象/null 一律不算声明（缺失 ≠ 声明）。
   */
  | { readonly kind: 'artifact-json-pointer-exists'; readonly path: string; readonly pointers: readonly string[] }

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
      // RFC 6901 数组下标 ABNF：`"0" / non-zero-digit *DIGIT`——前导零（"01"）非法。
      // 触发条件是解锁判据：非法下标一律判 not-found（fail-closed），绝不宽松解析。
      if (!/^(0|[1-9]\d*)$/.test(token)) return { found: false, value: undefined }
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
  if (trigger.kind === 'artifact-json-pointer-exists') {
    // 任一命名 pointer 存在且非空 ⇒ 已声明（空值不算：声明必须有内容）
    return trigger.pointers.some((pointer) => {
      const found = resolveJsonPointer(document, pointer)
      if (!found.found) return false
      const value = found.value
      if (value === null || value === undefined) return false
      if (typeof value === 'string') return value.trim() !== ''
      if (Array.isArray(value)) return value.length > 0
      if (typeof value === 'object') return Object.keys(value as object).length > 0
      return true
    })
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
      if (item.trigger.kind === 'artifact-json-pointer-exists') {
        if (item.trigger.pointers.length === 0) {
          throw new Error(`trigger.pointers 必填：${item.id} 的「已声明」判据至少要指名一个 pointer`)
        }
        const bad = item.trigger.pointers.find(pointer => !pointer.startsWith('/'))
        if (bad !== undefined) {
          throw new Error(`trigger.pointers 非法：${item.id} 的 RFC 6901 pointer 必须以 / 开头（收到 ${JSON.stringify(bad)}）`)
        }
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
        // 损坏的登记文件：无法求值就不能解除（也不会误解除），跳过本文件；
        // 它作为「未决门」继续存在——all() 会把它合成为显式阻塞项（见下）。
        const items = await readSession(sessionId).catch(() => null)
        if (items === null || items.length === 0) continue
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
        const file = path.join(dir, entry.name)
        const sessionId = entry.name.slice(0, -'.json'.length)
        const text = await readFile(file, 'utf8').catch(() => null)
        let parsed: DeferredFile | null = null
        if (text !== null) {
          try {
            parsed = JSON.parse(text) as DeferredFile
          } catch {
            parsed = null
          }
        }
        if (parsed !== null && Array.isArray(parsed.items)) {
          collected.push(...parsed.items)
          continue
        }
        // 损坏/不可读的登记 = 隐藏的未决门：跳过它等于把门悄悄打开（正是本模块
        // 文档禁止的「损坏登记被当作无阻塞」）。合成为一条**不限 App** 的显式
        // 阻塞项——按 App 扫描的 publish 前置对缺省 app 采取 fail-closed 匹配
        // （上游 `deferred.rs:269-271` 对缺 app_code 的记录同判）。
        collected.push({
          id: `corrupt:${sessionId}`,
          sessionId,
          reason: 'deferred 登记文件不可解析或不可读',
          adjudication: '损坏登记 MUST NOT 被当作「无阻塞」：修复或删除该文件前，publish 前置持续阻断',
          successors: ['人工核对 deferred/ 目录中该会话的登记并修复'],
          createdTs: '',
          trigger: { kind: 'artifact-exists', path: file },
        })
      }
      return collected.sort((a, b) => {
        if (a.createdTs !== b.createdTs) return a.createdTs < b.createdTs ? -1 : 1
        return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
      })
    },
  }
}
