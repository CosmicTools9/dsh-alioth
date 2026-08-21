import { describe, expect, it } from 'vitest'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import * as feedback from '../src/index.ts'

async function store(): Promise<ReturnType<typeof feedback.createFeedbackStore>> {
  const dir = await mkdtemp(path.join(tmpdir(), 'feedback-'))
  return feedback.createFeedbackStore(path.join(dir, 'f.db'))
}

describe('feedback capability', () => {
  it('sessions are idempotent per (origin, url); annotations land pending', async () => {
    const svc = await store()
    const s1 = svc.ensureSession('http://127.0.0.1:3100', 'http://127.0.0.1:3100/usercenter')
    const s2 = svc.ensureSession('http://127.0.0.1:3100', 'http://127.0.0.1:3100/usercenter')
    expect(s1.id).toBe(s2.id)

    const a = svc.addAnnotation({ sessionId: s1.id, origin: s1.origin, url: s1.url, comment: '按钮错位', element: 'button.export', elementPath: 'div>button.export' })
    expect(a.status).toBe('pending')
    expect(svc.health().pending).toBe(1)
    expect(() => svc.addAnnotation({ origin: s1.origin, url: s1.url, comment: '  ' })).toThrow(/comment/)
  })

  it('state machine: pending→ack→resolved; terminal has no exits; idempotent same-status', async () => {
    const svc = await store()
    const a = svc.addAnnotation({ origin: 'o', url: 'u', comment: 'c1' })
    const acked = svc.setStatus(a.id, 'acknowledged', '处理中')
    expect(acked.status).toBe('acknowledged')
    expect(acked.reply).toBe('处理中')

    // pending ⇄ acknowledged
    expect(svc.setStatus(a.id, 'pending', undefined).status).toBe('pending')
    expect(svc.setStatus(a.id, 'acknowledged', undefined).status).toBe('acknowledged')

    // reply-only patch keeps status
    expect(svc.setStatus(a.id, undefined, '补充说明').reply).toBe('补充说明')
    expect(svc.get(a.id)?.status).toBe('acknowledged')

    const resolved = svc.setStatus(a.id, 'resolved', '已修复')
    expect(resolved.status).toBe('resolved')
    expect(() => svc.setStatus(a.id, 'pending', undefined)).toThrow(/not an allowed transition/)
    // same-status PATCH on terminal state is idempotent
    expect(svc.setStatus(a.id, 'resolved', '再次确认').status).toBe('resolved')
    // reply write still allowed on terminal states
    expect(svc.setStatus(a.id, undefined, '证据已归档').reply).toBe('证据已归档')

    expect(() => svc.setStatus('missing', 'resolved', undefined)).toThrow(/not found/)
    expect(svc.pending()).toHaveLength(0)
  })

  it('watch resolves early on a new annotation and times out to the current batch', async () => {
    const svc = await store()
    const first = svc.watch(10_000)
    setTimeout(() => { svc.addAnnotation({ origin: 'o', url: 'u', comment: 'wake up' }) }, 30)
    const batch = await first
    expect(batch.some(a => a.comment === 'wake up')).toBe(true)

    const timedOut = await svc.watch(20)
    expect(timedOut.some(a => a.comment === 'wake up')).toBe(true)
  })

  it('mounts on a Context with a temp db; prune drops terminal annotations', async () => {
    const dir = await mkdtemp(path.join(tmpdir(), 'feedback-ctx-'))
    const ctx = new Context()
    const plugin = await ctx.plugin(feedback, { dbPath: path.join(dir, 'f.db') })
    const a = ctx.aliothFeedback.addAnnotation({ origin: 'o', url: 'u', comment: 'x' })
    ctx.aliothFeedback.setStatus(a.id, 'dismissed', '不做')
    expect(ctx.aliothFeedback.health().annotations).toBe(1)
    expect(ctx.aliothFeedback.prune(0)).toBe(1)
    expect(ctx.aliothFeedback.health().annotations).toBe(0)
    await plugin.dispose()
  })
})
