/**
 * `alioth_deferred`——Deferred But Adjudicated 阻塞登记（薄封装 `@dsh-alioth/verify-alioth`，零 LLM）。
 *
 * 契约（对齐上游 `deferred.rs`，MUST NOT 软化）：
 * - **D1 登记即裁决**：`adjudication` 必填——空串即 throw（无裁决的延期 = 遗忘的阻塞）；
 * - **D2 触发条件仅磁盘可判定式**：`artifact-exists` / `artifact-fingerprint` / `artifact-json-pointer`
 *   （RFC 6901 取值深比较；拒绝任意表达式求值）；
 * - **D3 会话级作用域**：落 `{root}/deferred/{sessionId}.json`；会话 id 缺省取本调用 agent 归属，
 *   两者都不可得 → throw（不猜测会话）；
 * - `unlock` 只解除**触发条件已满足**者并返回之（未满足者留在登记表，`open` 看得见）。
 * @module @dsh-alioth/tool-alioth-verify/deferred-tool
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import type { DeferredStore, DeferredTrigger } from '@dsh-alioth/verify-alioth'
import { EXTENSIONS_GATE_PREFIX } from './extensions-gate.ts'
import { asJsonOutput } from './json-output.ts'
import { requireString, sessionIdOf } from './paths.ts'

const DEFERRED_ACTIONS = ['register', 'list', 'unlock'] as const

/** 解析触发条件（仅磁盘可判定两式；形态非法即 throw，不做宽松兜底）。 */
function parseTrigger(value: unknown): DeferredTrigger {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('alioth_deferred: trigger 必填且必须是对象')
  }
  const record = value as Record<string, unknown>
  const kind = record['kind']
  const triggerPath = record['path']
  if (typeof triggerPath !== 'string' || triggerPath.trim() === '') {
    throw new Error('alioth_deferred: trigger.path 必填（磁盘可判定式的目标路径）')
  }
  if (kind === 'artifact-exists') return { kind, path: triggerPath }
  if (kind === 'artifact-fingerprint') {
    const sha256 = record['sha256']
    if (typeof sha256 !== 'string' || sha256.trim() === '') {
      throw new Error('alioth_deferred: trigger.kind="artifact-fingerprint" 需要非空 sha256')
    }
    return { kind, path: triggerPath, sha256 }
  }
  if (kind === 'artifact-json-pointer') {
    const pointer = record['pointer']
    if (typeof pointer !== 'string' || !pointer.startsWith('/')) {
      throw new Error('alioth_deferred: trigger.kind="artifact-json-pointer" 需要 RFC 6901 pointer（以 "/" 开头）')
    }
    if (!('equals' in record)) {
      throw new Error('alioth_deferred: trigger.kind="artifact-json-pointer" 需要 equals（pointer 处的期望值）')
    }
    return { kind, path: triggerPath, pointer, equals: record['equals'] }
  }
  throw new Error(
    'alioth_deferred: trigger.kind 必须是 artifact-exists|artifact-fingerprint|artifact-json-pointer'
    + `（拒绝任意表达式求值），收到 ${JSON.stringify(kind)}`,
  )
}

/** 解析可选字符串数组（successors）。 */
function parseStringArray(value: unknown, field: string): readonly string[] {
  if (value === undefined) return []
  if (!Array.isArray(value) || value.some(entry => typeof entry !== 'string')) {
    throw new Error(`alioth_deferred: ${field} 必须是字符串数组`)
  }
  return value as readonly string[]
}

/** 注册 `alioth_deferred`。 */
export function registerDeferredTool(ctx: Context, options: { readonly store: DeferredStore }): void {
  ctx.tools.register(defineTool({
    name: 'alioth_deferred',
    description:
      'Register blockers as "deferred but adjudicated" instead of silently dropping them. '
      + 'Actions: "register" — record one blocker WITH an adjudication (why deferring is legitimate; empty is rejected) '
      + 'and a disk-decidable trigger (`{kind:"artifact-exists", path}` / `{kind:"artifact-fingerprint", path, sha256}` / '
      + '`{kind:"artifact-json-pointer", path, pointer, equals}` = RFC 6901 value comparison); '
      + '"list" — open blockers for this session PLUS every per-App human gate (extension verification degradation gates '
      + 'are App-scoped, so they are visible regardless of which session asks); '
      + '"unlock" — release and return the blockers whose trigger is now satisfied (session blockers + App gates). '
      + 'Records are session/App-scoped on disk; a trigger is never an arbitrary expression.',
    parameters: {
      action: { type: 'string', required: true, description: `One of: ${DEFERRED_ACTIONS.join(', ')}.` },
      sessionId: { type: 'string', description: 'Session scope; defaults to this call\'s agent id, and is required when the call has no agent.' },
      id: { type: 'string', description: 'register only: unique blocker id within the session.' },
      reason: { type: 'string', description: 'register only: what is blocked.' },
      adjudication: { type: 'string', description: 'register only: why deferring is legitimate (must not be empty).' },
      trigger: { type: 'json', description: 'register only: {kind, path, sha256?} disk-decidable trigger.' },
      successors: { type: 'array', items: { type: 'string' }, description: 'register only: follow-up ids that depend on this blocker.' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          scope: { type: 'string' },
          count: { type: 'number' },
          item: { type: 'json' },
          items: { type: 'json' },
          unlocked: { type: 'json' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'register'
          ? `deferred registered: ${String((value.item as { id?: unknown } | undefined)?.id ?? '')}`
          : value.action === 'list'
            ? `${String(value.count ?? 0)} open blocker(s) (${String(value.scope)})`
            : `${String(value.count ?? 0)} blocker(s) unlocked`,
      }],
    },
    async execute(args, exec) {
      const a = args as Record<string, unknown>
      const action = typeof a.action === 'string' ? a.action : ''
      if (!(DEFERRED_ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_deferred: invalid action ${JSON.stringify(a.action)} (expected ${DEFERRED_ACTIONS.join(', ')})`)
      }

      if (action === 'register') {
        const sessionId = sessionIdOf(exec, a.sessionId)
        const item = {
          id: requireString(a, 'id', 'alioth_deferred'),
          sessionId,
          reason: requireString(a, 'reason', 'alioth_deferred'),
          adjudication: requireString(a, 'adjudication', 'alioth_deferred'),
          trigger: parseTrigger(a.trigger),
          successors: parseStringArray(a.successors, 'successors'),
          createdTs: new Date().toISOString(),
        }
        await options.store.register(item)
        return { action, count: (await options.store.open(sessionId)).length, item: asJsonOutput(item) }
      }

      if (action === 'list') {
        // 人工门按 **app** 作用域登记（见 extensions-gate.ts），不能只按会话过滤——
        // 否则调用方在自己的会话里看到的永远是空集，未决门就"看不见"了。
        const all = await options.store.all()
        const appGates = all.filter(item => item.sessionId.startsWith(EXTENSIONS_GATE_PREFIX))
        const explicit = typeof a.sessionId === 'string' && a.sessionId.trim() !== ''
        const hasAgent = exec.agent?.id !== undefined && String(exec.agent.id).trim() !== ''
        if (!explicit && !hasAgent) {
          return { action, scope: 'all', count: all.length, items: asJsonOutput(all) }
        }
        const sessionId = sessionIdOf(exec, a.sessionId)
        const sessionItems = await options.store.open(sessionId)
        const items = [...sessionItems, ...appGates.filter(gate => gate.sessionId !== sessionId)]
        return {
          action,
          scope: `session:${sessionId}+app-gates`,
          count: items.length,
          items: asJsonOutput(items),
        }
      }

      const sessionId = sessionIdOf(exec, a.sessionId)
      // 解除 = 会话自身的项 + 全部 app 人工门（门的触发条件是磁盘可判定式，其值不受调用方影响）。
      const due = [...await options.store.unlockDue(sessionId)]
      for (const gateScope of new Set((await options.store.all())
        .filter(item => item.sessionId.startsWith(EXTENSIONS_GATE_PREFIX))
        .map(item => item.sessionId))) {
        due.push(...await options.store.unlockDue(gateScope))
      }
      return { action, count: due.length, unlocked: asJsonOutput(due) }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Deferred ${String((args as Record<string, unknown>).action ?? '')}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
