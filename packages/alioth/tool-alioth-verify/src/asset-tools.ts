/**
 * `alioth_version` / `alioth_patch_assets`——产物版本快照与两段式局部改动（薄封装
 * `@dsh-alioth/verify-alioth`，零 LLM）。
 *
 * 契约要点（两段式的第二段纪律）：
 * - `alioth_version rollback` 与 `alioth_patch_assets apply` **都**要求 `confirmed: true`；
 *   未确认时拒绝并**一个字都不写**（错误文案说明这是两段式的第二段，先展示再确认）。
 * - `rollback` 走库的「全有或全无」回退：快照清单逐文件校验，任一失配在写入前拒绝整次回退。
 * - 补丁**提案段不写盘**：只读目标现文、返回 unified diff + 基准指纹 + `proposalId`；
 *   提案登记在**本进程**（不落盘），`apply` 按 `proposalId` 取回，基准指纹不符即拒绝
 *   （防覆盖并发改动）。目标路径必须落在 App 产物目录内。
 * @module @dsh-alioth/tool-alioth-verify/asset-tools
 */

import { readFile } from 'node:fs/promises'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import {
  applyPatchProposal,
  listSnapshots,
  proposePatch,
  restoreSnapshot,
  snapshotArtifacts,
  KEEP_VERSIONS,
  type PatchProposal,
} from '@dsh-alioth/verify-alioth'
import { appDirOf, assertNamespaceApp, requireString, resolveAssetTarget } from './paths.ts'

const VERSION_ACTIONS = ['snapshot', 'list', 'rollback'] as const
const PATCH_ACTIONS = ['propose', 'apply'] as const

/** 本进程内的提案登记表（提案段不落盘；进程重启即失效，`apply` 会如实报「不在登记表」）。 */
type PendingPatch = { readonly proposal: PatchProposal }

/** 注册 `alioth_version` 与 `alioth_patch_assets`。 */
export function registerAssetTools(ctx: Context, options: { readonly preProcRoot: string }): void {
  const preProcRoot = options.preProcRoot
  const proposals = new Map<string, PendingPatch>()

  ctx.tools.register(defineTool({
    name: 'alioth_version',
    description:
      'Artifact version snapshots and rollback (`{appDir}/versions/{seq:04}-{hash8}/`, newest '
      + `${KEEP_VERSIONS} kept). Actions: "snapshot" — snapshot the App's json/yaml/md artifacts (excludes plans/ and versions/); `
      + '"list" — list existing snapshot sequence numbers; '
      + '"rollback" — restore one snapshot and REQUIRES `confirmed: true` (two-phase: this is the second phase; '
      + 'without it nothing is written). Rollback is all-or-nothing: any manifest mismatch refuses the whole restore.',
    parameters: {
      action: { type: 'string', required: true, description: `One of: ${VERSION_ACTIONS.join(', ')}.` },
      namespace: { type: 'string', description: 'Workspace namespace (Pre-Proc/{namespace}); required.' },
      app: { type: 'string', description: 'App code; required.' },
      seq: { type: 'number', description: 'rollback only: snapshot sequence number from action "list".' },
      confirmed: { type: 'boolean', description: 'rollback only: must be exactly true — the second phase of the two-phase flow.' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          seq: { type: 'number' },
          dir: { type: 'string' },
          entries: { type: 'number' },
          versions: { type: 'array', items: { type: 'number' } },
          restored: { type: 'array', items: { type: 'string' } },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'snapshot'
          ? `snapshot #${String(value.seq)}: ${String(value.entries)} file(s) → ${String(value.dir)}`
          : value.action === 'list'
            ? `${value.versions?.length ?? 0} snapshot(s): ${(value.versions ?? []).join(', ')}`
            : `rolled back #${String(value.seq)}: ${value.restored?.length ?? 0} file(s)`,
      }],
    },
    async execute(args) {
      const a = args as Record<string, unknown>
      const action = typeof a.action === 'string' ? a.action : ''
      if (!(VERSION_ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_version: invalid action ${JSON.stringify(a.action)} (expected ${VERSION_ACTIONS.join(', ')})`)
      }
      const namespace = requireString(a, 'namespace', 'alioth_version')
      const app = requireString(a, 'app', 'alioth_version')
      assertNamespaceApp(namespace, app, 'alioth_version')
      const appDir = appDirOf(preProcRoot, namespace, app)

      if (action === 'list') {
        return { action, versions: [...await listSnapshots(appDir)] }
      }

      if (action === 'snapshot') {
        const result = await snapshotArtifacts(appDir)
        return { action, seq: result.seq, dir: result.dir, entries: result.entries.length }
      }

      // rollback：第二段必须显式确认；未确认时在任何写入之前拒绝（一个字都不写）。
      if (a.confirmed !== true) {
        throw new Error(
          'alioth_version rollback: confirmed !== true — 回退会覆盖既有产物，未确认时 MUST NOT 写盘。'
          + '本工具是两段式的第二段：先用 action "list" 展示可选快照与 seq，取得明确确认后以 confirmed: true 重试',
        )
      }
      const seq = a.seq
      if (typeof seq !== 'number' || !Number.isInteger(seq) || seq < 1) {
        throw new Error(`alioth_version: rollback 需要整数 seq（来自 action "list"），收到 ${JSON.stringify(a.seq)}`)
      }
      const result = await restoreSnapshot(appDir, seq)
      return { action, seq, restored: [...result.restored] }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Versions ${String((args as Record<string, unknown>).action ?? '')} ${String((args as Record<string, unknown>).namespace ?? '')}/${String((args as Record<string, unknown>).app ?? '')}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_patch_assets',
    description:
      'Two-phase partial edit of an existing App artifact file. '
      + '"propose" — read the file as it is now and return a unified diff + base fingerprint + proposalId; '
      + 'NOTHING is written and the proposal is registered in-process only. '
      + '"apply" — apply a registered proposal and REQUIRES `confirmed: true` (the second phase); '
      + 'without it nothing is written, and a target that changed since the proposal is refused instead of '
      + 'overwriting concurrent edits. `target` is relative to the App directory (Pre-Proc/{ns}/Apps/{app}) and '
      + 'must resolve inside it.',
    parameters: {
      action: { type: 'string', required: true, description: `One of: ${PATCH_ACTIONS.join(', ')}.` },
      namespace: { type: 'string', description: 'Workspace namespace (Pre-Proc/{namespace}); required.' },
      app: { type: 'string', description: 'App code; required.' },
      target: { type: 'string', description: 'propose only: file path relative to the App directory, e.g. "extensions/rules.yaml".' },
      after: { type: 'string', description: 'propose only: the FULL new file content (not a fragment).' },
      proposalId: { type: 'string', description: 'apply only: proposalId returned by action "propose".' },
      confirmed: { type: 'boolean', description: 'apply only: must be exactly true — the second phase of the two-phase flow.' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          proposalId: { type: 'string' },
          target: { type: 'string' },
          unifiedDiff: { type: 'string' },
          baseFingerprint: { type: 'string' },
          createdTs: { type: 'string' },
          applied: { type: 'boolean' },
          reason: { type: 'string' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'propose'
          ? `proposal ${String(value.proposalId)} for ${String(value.target)} (nothing written)`
          : `apply ${String(value.proposalId)}: applied=${String(value.applied)} — ${String(value.reason)}`,
      }],
    },
    async execute(args) {
      const a = args as Record<string, unknown>
      const action = typeof a.action === 'string' ? a.action : ''
      if (!(PATCH_ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_patch_assets: invalid action ${JSON.stringify(a.action)} (expected ${PATCH_ACTIONS.join(', ')})`)
      }
      const namespace = requireString(a, 'namespace', 'alioth_patch_assets')
      const app = requireString(a, 'app', 'alioth_patch_assets')
      assertNamespaceApp(namespace, app, 'alioth_patch_assets')
      const appDir = appDirOf(preProcRoot, namespace, app)

      if (action === 'propose') {
        const target = requireString(a, 'target', 'alioth_patch_assets')
        const after = requireString(a, 'after', 'alioth_patch_assets')
        const abs = resolveAssetTarget(appDir, target, 'alioth_patch_assets')
        const before = await readFile(abs, 'utf8').catch(() => null)
        if (before === null) {
          throw new Error(`alioth_patch_assets: 目标不可读 ${abs}（本工具只改既有产物文件，不新建）`)
        }
        const proposal = proposePatch({ target: abs, before, after })
        proposals.set(proposal.proposalId, { proposal })
        return {
          action,
          proposalId: proposal.proposalId,
          target: proposal.target,
          unifiedDiff: proposal.unifiedDiff,
          baseFingerprint: proposal.baseFingerprint,
          createdTs: proposal.createdTs,
        }
      }

      // apply：第二段必须显式确认；未确认时在任何写入之前拒绝（一个字都不写）。
      if (a.confirmed !== true) {
        throw new Error(
          'alioth_patch_assets apply: confirmed !== true — 应用提案会改写既有资产，未确认时 MUST NOT 写盘。'
          + '本工具是两段式的第二段：先展示 action "propose" 的 unifiedDiff，取得明确确认后以 confirmed: true 重试',
        )
      }
      const proposalId = requireString(a, 'proposalId', 'alioth_patch_assets')
      const pending = proposals.get(proposalId)
      if (pending === undefined) {
        throw new Error(
          `alioth_patch_assets apply: 提案 ${JSON.stringify(proposalId)} 不在本进程登记表——`
          + '提案段不落盘，请在同一进程内先调用 action "propose"（跨进程/重启后须重新提案）',
        )
      }
      const result = await applyPatchProposal({
        target: pending.proposal.target,
        proposal: pending.proposal,
        confirmed: true,
      })
      if (result.applied) proposals.delete(proposalId)
      return { action, proposalId, applied: result.applied, reason: result.reason }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Patch assets ${String((args as Record<string, unknown>).action ?? '')} ${String((args as Record<string, unknown>).target ?? (args as Record<string, unknown>).proposalId ?? '')}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
