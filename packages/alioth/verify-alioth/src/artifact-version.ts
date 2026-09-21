/**
 * 产物版本快照与回退 —— 对齐上游 `artifact_version.rs`：
 *
 * - 快照范围 = `{appDir}` 下 `json/yaml/yml/md`（排除 `plans/` 与 `versions/`）；
 * - 落点 `{appDir}/versions/{seq:04}-{hash8}/`，内含 `MANIFEST.json`（`files: {rel: sha256}`）；
 * - 原子：先写 `versions/.tmp-{seq:04}/` 再 rename 为正式目录；
 * - 保留：仅最近 `KEEP_VERSIONS` 版（只作用于 `versions/` 内）；
 * - **回退全有或全无**：逐文件校验清单，任一失配即拒绝整次回退（不得半恢复）。
 * @module @dsh-alioth/verify-alioth/artifact-version
 */

import { createHash } from 'node:crypto'
import { copyFile, mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'

/** 保留的版本数（沿原型产物保留 3 版纪律）。 */
export const KEEP_VERSIONS = 3

/** 快照内清单文件名。 */
export const SNAPSHOT_MANIFEST = 'MANIFEST.json'

const VERSIONS_DIR = 'versions'

/** 快照范围内的单文件条目。 */
export interface SnapshotEntry {
  readonly rel: string
  readonly sha256: string
}

/** 快照结果。 */
export interface SnapshotResult {
  readonly seq: number
  readonly dir: string
  readonly entries: readonly SnapshotEntry[]
}

/** 快照清单（`MANIFEST.json` 文档形态）。 */
interface SnapshotManifest {
  readonly seq: number
  readonly content_hash: string
  readonly created_at: string
  readonly files: Record<string, string>
}

const SNAPSHOT_EXCLUDED_DIRS: readonly string[] = ['plans', VERSIONS_DIR]
const SNAPSHOT_EXTENSIONS: readonly string[] = ['.json', '.yaml', '.yml', '.md']

/** 递归枚举快照范围内的产物（相对路径用 `/` 分隔，保证跨平台清单稳定）。 */
async function collectArtifactFiles(root: string, dir: string): Promise<readonly SnapshotEntry[]> {
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
  if (entries === null) return []
  const out: SnapshotEntry[] = []
  for (const entry of entries) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (SNAPSHOT_EXCLUDED_DIRS.includes(entry.name)) continue
      out.push(...(await collectArtifactFiles(root, full)))
      continue
    }
    if (!entry.isFile() || !SNAPSHOT_EXTENSIONS.includes(path.extname(entry.name))) continue
    const bytes = await readFile(full).catch(() => null)
    if (bytes === null) continue
    out.push({
      rel: path.relative(root, full).split(path.sep).join('/'),
      sha256: createHash('sha256').update(bytes).digest('hex'),
    })
  }
  return out.sort((a, b) => (a.rel < b.rel ? -1 : a.rel > b.rel ? 1 : 0))
}

/** 快照目录名：`{seq:04}-{hash8}`（hash8 = 内容指纹 hex 前 8 位，不含 `sha256:` 前缀）。 */
function snapshotDirName(seq: number, contentHash: string): string {
  return `${String(seq).padStart(4, '0')}-${contentHash.replace(/^sha256:/, '').slice(0, 8)}`
}

/** 内容指纹：`sha256:` + 排序后 `rel\0sha\n` 序联摘要（确定性，与文件系统枚举顺序无关）。 */
function contentHash(entries: readonly SnapshotEntry[]): string {
  const hasher = createHash('sha256')
  for (const entry of entries) {
    hasher.update(`${entry.rel}\0${entry.sha256}\n`)
  }
  return `sha256:${hasher.digest('hex')}`
}

/** 列出现有快照序号（升序）。 */
export async function listSnapshots(appDir: string): Promise<readonly number[]> {
  const root = path.join(path.resolve(appDir), VERSIONS_DIR)
  const entries = await readdir(root, { withFileTypes: true }).catch(() => null)
  if (entries === null) return []
  const seqs: number[] = []
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue
    const head = entry.name.split('-')[0]
    const seq = head === undefined ? Number.NaN : Number.parseInt(head, 10)
    if (Number.isInteger(seq)) seqs.push(seq)
  }
  return seqs.sort((a, b) => a - b)
}

/** 保留策略：只保留最近 `KEEP_VERSIONS` 版，超出删序号最小目录（只作用于 `versions/` 内）。 */
async function enforceRetention(appDir: string): Promise<readonly string[]> {
  const root = path.join(path.resolve(appDir), VERSIONS_DIR)
  const seqs = await listSnapshots(appDir)
  if (seqs.length <= KEEP_VERSIONS) return []
  const doomed = seqs.slice(0, seqs.length - KEEP_VERSIONS)
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  const removed: string[] = []
  for (const name of doomed) {
    const match = entries.find(
      entry => entry.isDirectory() && Number.parseInt(entry.name.split('-')[0] ?? '', 10) === name,
    )
    if (match === undefined) continue
    const target = path.join(root, match.name)
    await rm(target, { recursive: true, force: true })
    removed.push(match.name)
  }
  return removed
}

/**
 * 建快照（原子：`.tmp-{seq:04}` → rename，再执行保留清理）。
 * 快照范围内无产物 → throw（无内容可快照时 MUST NOT 落空快照冒充版本）。
 */
export async function snapshotArtifacts(appDir: string): Promise<SnapshotResult> {
  const dir = path.resolve(appDir)
  const entries = await collectArtifactFiles(dir, dir)
  if (entries.length === 0) {
    throw new Error(`产物枚举为空：${dir} 下无可快照内容（json/yaml/yml/md，排除 plans/ 与 versions/）`)
  }
  const root = path.join(dir, VERSIONS_DIR)
  await mkdir(root, { recursive: true })

  const existing = await listSnapshots(dir)
  const nextSeq = (existing.at(-1) ?? 0) + 1
  const hash = contentHash(entries)
  const manifest: SnapshotManifest = {
    seq: nextSeq,
    content_hash: hash,
    created_at: new Date().toISOString(),
    files: Object.fromEntries(entries.map(entry => [entry.rel, entry.sha256])),
  }

  const tmp = path.join(root, `.tmp-${String(nextSeq).padStart(4, '0')}`)
  await rm(tmp, { recursive: true, force: true })
  await mkdir(tmp, { recursive: true })
  for (const entry of entries) {
    const dst = path.join(tmp, entry.rel)
    await mkdir(path.dirname(dst), { recursive: true })
    await copyFile(path.join(dir, entry.rel), dst)
  }
  await writeFile(path.join(tmp, SNAPSHOT_MANIFEST), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')

  const finalDir = path.join(root, snapshotDirName(nextSeq, hash))
  await rm(finalDir, { recursive: true, force: true })
  await rename(tmp, finalDir)
  await enforceRetention(dir)

  return { seq: nextSeq, dir: finalDir, entries }
}

/** 按序号解析快照目录（不存在 → throw，不猜回退目标）。 */
async function resolveSnapshotDir(appDir: string, seq: number): Promise<string> {
  const root = path.join(path.resolve(appDir), VERSIONS_DIR)
  const entries = await readdir(root, { withFileTypes: true }).catch(() => null)
  const match = (entries ?? []).find(
    entry => entry.isDirectory() && Number.parseInt(entry.name.split('-')[0] ?? '', 10) === seq,
  )
  if (match === undefined) {
    throw new Error(`无序号 ${seq} 的版本快照（${root}）：本函数不猜测回退目标，请先 listSnapshots`)
  }
  return path.join(root, match.name)
}

/**
 * 回退到指定快照。**全有或全无**：先读清单并逐文件重算 sha256，
 * 任一失配（缺文件 / 指纹不符 / 清单不可解析）在**写入任何文件之前**拒绝整次回退。
 */
export async function restoreSnapshot(appDir: string, seq: number): Promise<{ restored: readonly string[] }> {
  const dir = path.resolve(appDir)
  const snapshotDir = await resolveSnapshotDir(dir, seq)

  const manifestText = await readFile(path.join(snapshotDir, SNAPSHOT_MANIFEST), 'utf8').catch(() => null)
  if (manifestText === null) {
    throw new Error(`快照清单缺失/不可读：${path.join(snapshotDir, SNAPSHOT_MANIFEST)}（备份不完整，拒绝整次回退）`)
  }
  let manifest: SnapshotManifest
  try {
    manifest = JSON.parse(manifestText) as SnapshotManifest
  } catch (error) {
    throw new Error(`快照清单解析失败：${error instanceof Error ? error.message : String(error)}（拒绝整次回退）`)
  }

  const mismatches: string[] = []
  for (const [rel, expected] of Object.entries(manifest.files)) {
    const bytes = await readFile(path.join(snapshotDir, rel)).catch(() => null)
    if (bytes === null) {
      mismatches.push(`${rel}: 快照缺文件`)
      continue
    }
    const actual = createHash('sha256').update(bytes).digest('hex')
    if (actual !== expected) mismatches.push(`${rel}: 指纹失配（快照 ${expected} ≠ 实际 ${actual}）`)
  }
  if (mismatches.length > 0) {
    throw new Error(
      `快照完整性校验失败（${mismatches.length} 处）：${mismatches.join('; ')} —— 拒绝整次回退，不半恢复`,
    )
  }

  const restored: string[] = []
  for (const rel of Object.keys(manifest.files).sort()) {
    const dst = path.join(dir, rel)
    await mkdir(path.dirname(dst), { recursive: true })
    await copyFile(path.join(snapshotDir, rel), dst)
    restored.push(rel)
  }
  return { restored }
}
