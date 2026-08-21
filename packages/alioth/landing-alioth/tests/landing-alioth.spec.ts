import { describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import * as landing from '../src/index.ts'

describe('landing-alioth (no webServer — service only)', () => {
  it('provides ctx.aliothLanding with the showcase html', async () => {
    const ctx = new Context()
    const plugin = await ctx.plugin(landing, {})
    await plugin.dispose()
    // The service survives disposal semantics checks in cordis; assert the
    // content contract through a fresh mount instead.
    const ctx2 = new Context()
    const p2 = await ctx2.plugin(landing, {})
    const service = (ctx2.get as (name: string) => unknown).call(ctx2, 'aliothLanding') as
      { path: string; html: string }
    expect(service.path).toBe('/landing')
    expect(service.html).toContain('Alioth AppCreator')
    expect(service.html).toContain('app-creation')
    expect(service.html).toContain('e2e-verification')
    expect(service.html).toContain('Scene 场景') // ontology coordinates (BP narrative)
    expect(service.html).toContain('2026108466144') // patent filing signal
    await p2.dispose()
  })
})
