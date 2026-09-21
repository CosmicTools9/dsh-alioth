/**
 * 两段式局部改动（propose / apply）—— 对齐上游 `dialog_tools/patch_assets.rs`：
 *
 * - **提案段不写盘**：只返回 unified diff + 基准指纹（baseFingerprint）；
 * - **应用段**必须 `confirmed === true`（否则一个字都不写）；且目标内容仍与基准指纹一致，
 *   否则拒绝（防覆盖并发改动）；
 * - 写回原子：同目录 `.tmp-patch-{pid}` → rename。
 * @module @dsh-alioth/verify-alioth/patch-assets
 */

import { createHash } from 'node:crypto'
import { readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

/** unified diff 上下文行数（单 hunk 一次性改动，与上游一致）。 */
const CONTEXT_LINES = 3

/** 提案（应用段的输入；不落盘、不含 `after` 明文，只含 diff 与基准指纹）。 */
export interface PatchProposal {
  readonly proposalId: string
  readonly target: string
  readonly unifiedDiff: string
  readonly baseFingerprint: string
  readonly createdTs: string
}

function shaOf(text: string): string {
  return `sha256:${createHash('sha256').update(text, 'utf8').digest('hex')}`
}

/** 生成单 hunk unified diff：行级公共前后缀剥离后，中间段即变更体。 */
function buildUnifiedDiff(target: string, before: string, after: string): string {
  const oldLines = before.split('\n')
  const newLines = after.split('\n')

  let prefix = 0
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) prefix += 1
  let suffix = 0
  while (
    suffix < oldLines.length - prefix &&
    suffix < newLines.length - prefix &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix += 1
  }

  const ctxStart = Math.max(0, prefix - CONTEXT_LINES)
  const oldCount = oldLines.length - ctxStart - suffix
  const newCount = newLines.length - ctxStart - suffix
  const out: string[] = [
    `--- a/${target}`,
    `+++ b/${target}`,
    `@@ -${ctxStart + 1},${oldCount} +${ctxStart + 1},${newCount} @@`,
  ]
  for (const line of oldLines.slice(ctxStart, prefix)) out.push(` ${line}`)
  for (const line of oldLines.slice(prefix, oldLines.length - suffix)) out.push(`-${line}`)
  for (const line of newLines.slice(prefix, newLines.length - suffix)) out.push(`+${line}`)
  for (const line of oldLines.slice(oldLines.length - suffix)) out.push(` ${line}`)
  return `${out.join('\n')}\n`
}

type PatchResult = { readonly ok: true; readonly text: string } | { readonly ok: false; readonly reason: string }

/** 应用 unified diff（上下文行必须逐一命中；多 hunk 以起始行号续接）。 */
function applyUnifiedDiff(content: string, diff: string): PatchResult {
  const target = content.split('\n')
  const out: string[] = []
  const body = diff.split('\n')
  let cursor = 0
  let sawHunk = false

  for (const [index, raw] of body.entries()) {
    if (raw === '' && index === body.length - 1) break
    if (raw.startsWith('--- ') || raw.startsWith('+++ ') || raw.startsWith('\\')) continue
    if (raw.startsWith('@@')) {
      const header = /^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@/.exec(raw)
      if (header === null) return { ok: false, reason: `非法 hunk 头：${raw}` }
      const startIndex = Number.parseInt(header[1] ?? '', 10) - 1
      if (startIndex < cursor) return { ok: false, reason: `hunk 起始行 ${startIndex + 1} 早于已消费位置（补丁乱序）` }
      while (cursor < startIndex) {
        const line = target[cursor]
        if (line === undefined) return { ok: false, reason: 'hunk 起始行越界（目标内容短于补丁）' }
        out.push(line)
        cursor += 1
      }
      sawHunk = true
      continue
    }
    if (!sawHunk) return { ok: false, reason: `hunk 体出现在任何 @@ 头之前：${raw}` }

    const marker = raw[0]
    const text = raw.slice(1)
    if (marker === ' ' || marker === '-') {
      if (target[cursor] !== text) {
        return { ok: false, reason: `第 ${cursor + 1} 行与补丁不一致（期望 ${JSON.stringify(text)}，实际 ${JSON.stringify(target[cursor])}）` }
      }
      cursor += 1
      if (marker === ' ') out.push(text)
      continue
    }
    if (marker === '+') {
      out.push(text)
      continue
    }
    return { ok: false, reason: `非法 diff 行：${raw}` }
  }

  while (cursor < target.length) {
    const line = target[cursor]
    if (line === undefined) break
    out.push(line)
    cursor += 1
  }
  return { ok: true, text: out.join('\n') }
}

/** 提案段：返回 diff + 基准指纹，**不触盘**。 */
export function proposePatch(input: { readonly target: string; readonly before: string; readonly after: string }): PatchProposal {
  const baseFingerprint = shaOf(input.before)
  return {
    proposalId: createHash('sha256')
      .update(`${input.target}|${shaOf(input.after)}|${baseFingerprint}`)
      .digest('hex')
      .slice(0, 12),
    target: input.target,
    unifiedDiff: buildUnifiedDiff(input.target, input.before, input.after),
    baseFingerprint,
    createdTs: new Date().toISOString(),
  }
}

/** 应用段：`confirmed !== true` 不写盘；基准指纹不符即拒绝。 */
export async function applyPatchProposal(input: {
  readonly target: string
  readonly proposal: PatchProposal
  readonly confirmed: boolean
}): Promise<{ applied: boolean; reason: string }> {
  if (input.confirmed !== true) {
    return {
      applied: false,
      reason:
        'confirmed !== true：应用段会改写既有资产，未确认时 MUST NOT 写盘——请先展示提案段的 unified diff 并取得明确确认后以 confirmed: true 重试',
    }
  }
  if (input.proposal.target !== input.target) {
    return {
      applied: false,
      reason: `提案目标（${input.proposal.target}）与本次目标（${input.target}）不一致：拒绝跨文件套用提案`,
    }
  }

  const abs = path.resolve(input.target)
  const current = await readFile(abs, 'utf8').catch(() => null)
  if (current === null) {
    return { applied: false, reason: `目标文件不可读：${abs}（本函数只改既有文件，不新建）` }
  }
  const currentFingerprint = shaOf(current)
  if (currentFingerprint !== input.proposal.baseFingerprint) {
    return {
      applied: false,
      reason: `目标已变更（提案基准 ${input.proposal.baseFingerprint} ≠ 当前 ${currentFingerprint}）：拒绝应用以防覆盖并发改动，请重新提案`,
    }
  }

  const patched = applyUnifiedDiff(current, input.proposal.unifiedDiff)
  if (!patched.ok) return { applied: false, reason: `补丁无法应用：${patched.reason}` }

  const tmp = `${abs}.tmp-patch-${process.pid}`
  await writeFile(tmp, patched.text, 'utf8')
  await rename(tmp, abs)
  return { applied: true, reason: `已应用提案 ${input.proposal.proposalId}（原子写回，基准指纹复核通过）` }
}
