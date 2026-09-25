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
    // Console entry: signed-in visitors land in the console, anonymous ones at
    // /login (the auth carrier's portal route decides per session).
    expect(service.html).toContain('href="/api/auth/portal"')
    // The document asks for the brand mark rather than falling back to the
    // browser's default tab glyph.
    expect(service.html).toContain('href="/favicon.svg"')
    // …and declares no web app manifest: a declared manifest makes the page
    // installable as a local application, which the B/S product is not.
    expect(service.html).not.toContain('rel="manifest"')
    expect(service.html).toContain('name="theme-color"')
    await p2.dispose()
  })

  it('renders the filing number from config only — a bare env var must not reach the page', async () => {
    const saved = process.env.ALIOTH_ICP
    try {
      // The composition decides which surfaces show the number; a global env
      // value read inside the plugin would defeat that scoping.
      process.env.ALIOTH_ICP = '沪ICP备0000000号-1'
      const unfiled = new Context()
      const plain = await unfiled.plugin(landing, {})
      const plainHtml = (unfiled.get('aliothLanding') as { html: string }).html
      expect(plainHtml).not.toContain('beian.miit.gov.cn')
      expect(plainHtml).not.toContain('<!--icp-->') // marker is always consumed
      await plain.dispose()

      const filed = new Context()
      const plugin = await filed.plugin(landing, { icp: '浙ICP备2023013865号-2' })
      const html = (filed.get('aliothLanding') as { html: string }).html
      expect(html).toContain('浙ICP备2023013865号-2')
      expect(html).toContain('href="https://beian.miit.gov.cn/"')
      await plugin.dispose()
    } finally {
      if (saved === undefined) delete process.env.ALIOTH_ICP
      else process.env.ALIOTH_ICP = saved
    }
  })
})

describe('landing-alioth (webServer mounted)', () => {
  it('serves the brand icon set and refuses both manifest paths', async () => {
    const ctx = new Context()
    const routes: Array<{ kind: string; path: string; handler: (req: unknown, res: never) => void }> = []
    ctx.provide('webServer')
    ctx.set('webServer', { register: (route: (typeof routes)[number]) => { routes.push(route); return () => {} } } as never)
    const plugin = await ctx.plugin(landing, {})

    const served: Record<string, { status: number; type: string; body: string }> = {}
    for (const route of routes) {
      let status = 0
      let headers: Record<string, string> = {}
      let body = ''
      const res = {
        writeHead: (code: number, next: Record<string, string>) => { status = code; headers = next },
        end: (payload: unknown) => { body = Buffer.isBuffer(payload) ? payload.toString('utf8') : String(payload) },
      }
      route.handler({}, res as never)
      served[route.path] = { status, type: headers['content-type'] ?? '', body }
    }
    await plugin.dispose()

    expect(Object.keys(served).sort()).toEqual([
      '/apple-touch-icon.png', '/favicon.ico', '/favicon.svg', '/landing',
      '/manifest.webmanifest', '/site.webmanifest',
    ])
    expect(served['/landing']).toMatchObject({ status: 200, type: 'text/html; charset=utf-8' })
    expect(served['/favicon.svg']).toMatchObject({ status: 200, type: 'image/svg+xml' })
    expect(served['/favicon.ico']!.type).toBe('image/x-icon')
    expect(served['/apple-touch-icon.png']!.type).toBe('image/png')
    // The mark is a real vector document, not a placeholder.
    expect(served['/favicon.svg']!.body).toContain('<svg')
    expect(served['/favicon.ico']!.body.length).toBeGreaterThan(300)
    // Both manifest paths are refused, not republished: the console shell's own
    // dist index asks for `/manifest.webmanifest`, and a served manifest of any
    // shape is what a browser turns into an "install this app" offer.
    expect(served['/manifest.webmanifest']).toMatchObject({ status: 404, type: 'text/plain; charset=utf-8' })
    expect(served['/site.webmanifest']).toMatchObject({ status: 404, type: 'text/plain; charset=utf-8' })
    expect(JSON.stringify(served)).not.toContain('application/manifest+json')
  })
})
