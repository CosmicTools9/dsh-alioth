import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as feedbackAlioth from '@dsh-alioth/feedback-alioth'
import * as toolFeedback from '../src/index.ts'

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let counter = 0

function callTool(name: string, args: unknown) {
  return ctx.tools.execute({
    signal: new AbortController().signal,
    callId: ToolCallId(`fb-${++counter}`),
    name,
    arguments: args,
  })
}

beforeAll(async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'fbtool-'))
  ctx = new Context()
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const store = await ctx.plugin(feedbackAlioth, { dbPath: path.join(dir, 'f.db') })
  disposers.push(() => store.dispose())
  const tool = await ctx.plugin(toolFeedback, {})
  disposers.push(() => tool.dispose())
}, 30_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('feedback consumer tools', () => {
  it('registers all four tools', () => {
    const names = ctx.tools.schemas().map(s => s.name)
    expect(names).toContain('alioth_feedback_pending')
    expect(names).toContain('alioth_feedback_ack')
    expect(names).toContain('alioth_feedback_resolve')
    expect(names).toContain('alioth_feedback_dismiss')
  })

  it('pending → ack → resolve full loop through the tools', async () => {
    const a = ctx.aliothFeedback.addAnnotation({ origin: 'o', url: 'u', comment: '间距不对' })

    const pending = await callTool('alioth_feedback_pending', {})
    if (pending.isError) throw new Error(`pending failed: ${pending.error.message}`)
    const list = pending.value as { annotations: Array<{ id: string }>; count: number }
    expect(list.count).toBe(1)
    expect(list.annotations[0]!.id).toBe(a.id)

    const acked = await callTool('alioth_feedback_ack', { id: a.id, reply: '处理中' })
    if (acked.isError) throw new Error(`ack failed: ${acked.error.message}`)
    expect((acked.value as { status: string }).status).toBe('acknowledged')

    const resolved = await callTool('alioth_feedback_resolve', { id: a.id, reply: '已调整间距' })
    if (resolved.isError) throw new Error(`resolve failed: ${resolved.error.message}`)
    expect((resolved.value as { status: string }).status).toBe('resolved')

    const after = await callTool('alioth_feedback_pending', {})
    expect(((after.value as { count: number }).count)).toBe(0)

    // resolve on a terminal state errors loudly.
    const illegal = await callTool('alioth_feedback_dismiss', { id: a.id, reply: '再想想' })
    expect(illegal.isError).toBe(true)
  })
})
