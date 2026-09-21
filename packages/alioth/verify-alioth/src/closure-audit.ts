/**
 * 独立结束审计（closure audit）—— 对齐上游 `closure_audit.rs`：
 * 生成与验收分离，裁决 **append-only** 落盘，产物指纹锁死「审计后偷改产物」，
 * 连续 rejected 达阈值升级人工（三次墙纪律）。
 *
 * 契约（MUST NOT 软化）：
 * - 落点 `{appDir}/AppAgentTraces/closure-audit/{seq}.json`，seq 从 1 起单调递增；
 * - 指纹 = `sha256(app.json 字节 + extensions/**\/*.yaml 排序后字节序联)`——只吃内容字节，
 *   不含路径 / mtime（跨平台稳定）；`app.json` 缺失即不可得 → throw（MUST NOT 以空输入冒充）；
 * - 连续 rejected 达 `ESCALATE_THRESHOLD` 次 → 新裁决 verdict='escalate'（转人工，禁止同构重试）。
 * @module @dsh-alioth/verify-alioth/closure-audit
 */

import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

export const CLOSURE_SCHEMA_VERSION = '1.0'

/** 连续 rejected 升级人工的阈值（对齐三次墙纪律）。 */
export const ESCALATE_THRESHOLD = 3

/** 单条审计发现。 */
export interface ClosureFinding {
  readonly dimension: string
  readonly verdict: 'pass' | 'fail' | 'unknown'
  readonly detail: string
}

/** closure 裁决记录（append-only 落盘单元）。 */
export interface ClosureVerdict {
  readonly schema_version: string
  readonly seq: number
  readonly app: string
  readonly namespace: string
  readonly verdict: 'approved' | 'rejected' | 'escalate'
  readonly fingerprint: string
  readonly findings: readonly ClosureFinding[]
  readonly evidence: readonly string[]
  readonly ts: string
}

function auditDir(appDir: string): string {
  return path.join(path.resolve(appDir), 'AppAgentTraces', 'closure-audit')
}

/** 递归收集 `extensions/` 下的 yaml 文件（升序绝对路径；目录不存在 → 空集）。 */
async function collectYamlFiles(dir: string): Promise<readonly string[]> {
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) return []
  const out: string[] = []
  for (const entry of entries) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      out.push(...(await collectYamlFiles(full)))
      continue
    }
    if (entry.isFile() && (entry.name.endsWith('.yaml') || entry.name.endsWith('.yml'))) out.push(full)
  }
  return out.sort()
}

/** 产物指纹（`sha256:` + hex）。app.json 不可读 → throw（不可得 MUST 显式，不得冒充）。 */
export async function artifactFingerprint(appDir: string): Promise<string> {
  const dir = path.resolve(appDir)
  const appJsonPath = path.join(dir, 'app.json')
  const appJson = await readFile(appJsonPath).catch(() => null)
  if (appJson === null) {
    throw new Error(`产物指纹不可得：${appJsonPath} 缺失/不可读（指纹须覆盖 app.json 字节，MUST NOT 以空输入冒充）`)
  }
  const hasher = createHash('sha256')
  hasher.update(appJson)
  for (const file of await collectYamlFiles(path.join(dir, 'extensions'))) {
    hasher.update(await readFile(file))
  }
  return `sha256:${hasher.digest('hex')}`
}

/** 读取全部裁决记录（seq 升序；损坏文件跳过——与上游 trace_store 的跳过纪律一致）。 */
export async function readClosureVerdicts(appDir: string): Promise<readonly ClosureVerdict[]> {
  const dir = auditDir(appDir)
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) return []
  const verdicts: ClosureVerdict[] = []
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith('.json')) continue
    const text = await readFile(path.join(dir, entry.name), 'utf8').catch(() => null)
    if (text === null) continue
    try {
      verdicts.push(JSON.parse(text) as ClosureVerdict)
    } catch {
      continue
    }
  }
  return verdicts.sort((a, b) => a.seq - b.seq)
}

/**
 * 追加一条裁决（append-only）：seq 自增；连续 rejected 达阈值 → verdict 升为 'escalate'。
 */
export async function appendClosureVerdict(
  appDir: string,
  verdict: Omit<ClosureVerdict, 'seq' | 'schema_version' | 'ts'>,
): Promise<ClosureVerdict> {
  const dir = auditDir(appDir)
  await mkdir(dir, { recursive: true })
  const prior = await readClosureVerdicts(appDir)
  const seq = (prior.at(-1)?.seq ?? 0) + 1

  let rejectedStreak = 0
  for (let i = prior.length - 1; i >= 0; i -= 1) {
    if (prior[i]?.verdict !== 'rejected') break
    rejectedStreak += 1
  }
  const escalate = verdict.verdict === 'rejected' && rejectedStreak + 1 >= ESCALATE_THRESHOLD
  const record: ClosureVerdict = {
    ...verdict,
    schema_version: CLOSURE_SCHEMA_VERSION,
    seq,
    verdict: escalate ? 'escalate' : verdict.verdict,
    ts: new Date().toISOString(),
  }

  const target = path.join(dir, `${seq}.json`)
  const tmp = path.join(dir, `.${seq}.json.tmp-${process.pid}`)
  await writeFile(tmp, `${JSON.stringify(record, null, 2)}\n`, 'utf8')
  await rename(tmp, target)
  return record
}

/** 最新一条指纹匹配的裁决（无匹配 → null；指纹失配 = 审计后产物被改，不允许沿用旧裁决）。 */
export async function latestMatchingVerdict(appDir: string, fingerprint: string): Promise<ClosureVerdict | null> {
  const verdicts = await readClosureVerdicts(appDir)
  for (let i = verdicts.length - 1; i >= 0; i -= 1) {
    const record = verdicts[i]
    if (record !== undefined && record.fingerprint === fingerprint) return record
  }
  return null
}
