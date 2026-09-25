/**
 * 人工映射裁决沉降（上游 `Meta/backend/app-agent/src/dialog_tools/record_verdict.rs` +
 * `memory::MemoryEntry::mapping_verdict`）。
 *
 * 上游把裁决写进它自己的 `isahl_meta.agent_memory`。本仓不往注册表 schema 里加表——那是基线
 * load-once 契约的一部分，增表会让「结构恒用包内基线」不再成立；因此改为部署自有的文件存储，
 * 与 `deferred` 同一规约：`<root>/mapping-verdicts/{namespace}.json`。
 *
 * 语义照搬上游（由调用方 `verdict-tool.ts` 执行）：
 * - `keep_gap` 不沉降（上游返回 err，防止 LLM 把「拒绝」当成功继续）；
 * - `table` 必须命中平台目录（防 stale：指向已退役表的旧裁决会污染后续召回）；
 * - 目录不可判定时 fail-closed 拒绝（写径宁缺勿污染）。
 *
 * 追加语义与上游一致：同一 `(domain, table)` 的新裁决**追加**在尾部，`recall` 取最新在前
 * （上游 memory 是 append + 近因），不做就地覆盖——审计面因此可见演进。
 * @module @dsh-alioth/verify-alioth/mapping-verdicts
 */

import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

/** 一条人工裁决（来源与置信度固定为人工档，对齐上游 `SOURCE_USER` / `CONFIDENCE_USER`）。 */
export interface MappingVerdict {
  readonly namespace: string
  /** 需求域 id（本体模型的 domain id）。 */
  readonly domain: string
  /** 目标 isahl 表名。 */
  readonly table: string
  readonly verdictText: string
  readonly source: 'user'
  /** 人工档恒为 1（上游承载 人工/evo 两类先例，故类型是 number，不是字面量 1）。 */
  readonly confidence: number
  /** ISO 时间戳。 */
  readonly recordedAt: string
}

export interface MappingVerdictStore {
  /** Append a verdict; returns the stored entry (with the resolved timestamp). */
  record(entry: {
    readonly namespace: string
    readonly domain: string
    readonly table: string
    readonly verdictText: string
  }): Promise<MappingVerdict>
  /** Verdicts for a namespace, newest first, optionally narrowed to a domain/table. */
  recall(query: {
    readonly namespace: string
    readonly domain?: string
    readonly table?: string
  }): Promise<MappingVerdict[]>
}

/** A namespace is a single path segment; anything else is folded so a scope cannot escape the root. */
function scopeFile(root: string, namespace: string): string {
  return path.join(root, 'mapping-verdicts', `${namespace.replace(/[^\w.-]/g, '_')}.json`)
}

async function readAll(file: string): Promise<MappingVerdict[]> {
  const text = await readFile(file, 'utf8').catch(() => null)
  if (text === null) {
    return []
  }
  try {
    const parsed: unknown = JSON.parse(text)
    return Array.isArray(parsed) ? parsed as MappingVerdict[] : []
  } catch {
    return [] // a corrupt file is an empty ledger, never a thrown store
  }
}

/**
 * Create the file-backed verdict store.
 * @param root - deployment state root (`<dataRoot>`); the ledger lives under `mapping-verdicts/`.
 */
export function createMappingVerdictStore(root: string): MappingVerdictStore {
  return {
    async record(entry) {
      const stored: MappingVerdict = {
        namespace: entry.namespace,
        domain: entry.domain,
        table: entry.table,
        verdictText: entry.verdictText,
        source: 'user',
        confidence: 1,
        recordedAt: new Date().toISOString(),
      }
      const file = scopeFile(root, entry.namespace)
      await mkdir(path.dirname(file), { recursive: true })
      const entries = [...await readAll(file), stored]
      // temp + rename: a reader never sees a half-written ledger.
      const temp = `${file}.tmp-${process.pid}`
      await writeFile(temp, `${JSON.stringify(entries, null, 2)}\n`)
      await rename(temp, file)
      return stored
    },

    async recall(query) {
      const entries = await readAll(scopeFile(root, query.namespace))
      return entries
        .filter(entry => (query.domain === undefined || entry.domain === query.domain)
          && (query.table === undefined || entry.table === query.table))
        .reverse()
    },
  }
}
