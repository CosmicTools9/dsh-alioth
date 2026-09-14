/**
 * `@dsh-alioth/landing-alioth` — the product landing page as its own plugin.
 *
 * Capability seam (harness plugin model: service + provider + consumer):
 * - **Service**: `ctx.aliothLanding` — `{ path: '/landing', html }`, the
 *   single source of the showcase and of its route path.
 * - **Provider**: mounts the exact `/landing` route on the harness
 *   `webServer` service when present (web profile); nothing otherwise
 *   (headless deployments get the service only).
 * - **Consumer**: `auth-web-alioth` — its gate script redirects unauthenticated
 *   visitors to `aliothLanding.path`, and the standalone B/S server serves
 *   `aliothLanding.html` at `/`.
 *
 * Static asset: `public/landing.html` (zero external resources, offline-safe).
 * @module @dsh-alioth/landing-alioth
 */

import { readFileSync } from 'node:fs'
import type { IncomingMessage, ServerResponse } from 'node:http'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'

export const name = 'landing-alioth'
export const inject = []

/** The landing capability: where it lives and what it serves. */
export interface AliothLandingService {
  /** Canonical route path (the auth gate redirects here). */
  readonly path: '/landing'
  /** The full landing HTML document. */
  readonly html: string
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothLanding: AliothLandingService
  }
}

export interface Config {
  /** Mainland-China ICP filing number rendered in the landing footer.
   * Config-only (no env read inside the plugin): *which* surfaces carry the
   * number is a per-surface deployment choice, so the composition — the bundle
   * patch wiring `ALIOTH_ICP` — decides. A global env read here would defeat
   * that. Empty — the default — renders nothing: an unfiled deployment must not
   * inherit another operator's number. */
  readonly icp?: string
}

export const Config: z<Config> = z.object({ icp: z.string().default('') })

/** 备案 link markup for the landing footer, or '' when nothing is filed.
 * `<!--icp-->` in public/landing.html is replaced with this at apply(). */
function icpMarkup(icp: string | undefined): string {
  const value = (icp ?? '').trim()
  if (value === '') return ''
  const escaped = value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
  return `<a href="https://beian.miit.gov.cn/" target="_blank" rel="noopener noreferrer">${escaped}</a>`
}

/** Structural face of the harness `webServer` service — no runtime dependency
 * on dsh-host-webserver; composed web deployments provide the real one. */
interface WebServerLike {
  register(route: {
    kind: 'exact' | 'prefix'
    path: string
    handler: (req: IncomingMessage, res: ServerResponse) => void | Promise<void>
  }): () => void
}

/**
 * Brand assets served beside the landing page: the address-bar icon set and the
 * site manifest. They live at the ORIGIN's conventional paths, not under a
 * landing-only prefix, because every document on the origin references them —
 * our public pages by declaration, the harness console shell by convention
 * (`./favicon.svg`, `./manifest.webmanifest` in its own index). One origin, one
 * mark.
 */
const BRAND_ASSETS: ReadonlyArray<{ path: string; file: string; type: string }> = [
  { path: '/favicon.svg', file: 'favicon.svg', type: 'image/svg+xml' },
  { path: '/favicon.ico', file: 'favicon.ico', type: 'image/x-icon' },
  { path: '/apple-touch-icon.png', file: 'apple-touch-icon.png', type: 'image/png' },
  { path: '/site.webmanifest', file: 'site.webmanifest', type: 'application/manifest+json' },
]

function asWebServer(value: unknown): WebServerLike | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined
  }
  const candidate = value as Record<string, unknown>
  return typeof candidate.register === 'function' ? value as WebServerLike : undefined
}

export function apply(ctx: Context, config: Config): void {
  void ctx
  const html = readFileSync(new URL('../public/landing.html', import.meta.url), 'utf8')
    .replace('<!--icp-->', icpMarkup(config.icp))
  const landing: AliothLandingService = { path: '/landing', html }
  ctx.provide('aliothLanding', landing)

  // Provider: the route on the harness webServer (web profile). The service
  // is a Service and may not be visible at apply() time — defer through
  // ctx.inject like the harness's own carrier plugins do.
  const inject = ctx.inject as (deps: string[], cb: (webCtx: Context) => void) => void
  inject.call(ctx, ['webServer'], webCtx => {
    const web = asWebServer((webCtx.get as (name: string) => unknown).call(webCtx, 'webServer'))
    if (web === undefined) {
      ctx.logger.warn('landing-alioth: webServer present but shape mismatch — /landing route not mounted')
      return
    }
    webCtx.effect(() => web.register({
      kind: 'exact',
      path: landing.path,
      handler: (_request, res) => {
        res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
        res.end(landing.html)
      },
    }))
    // Read once at mount: a missing brand file is a packaging mistake and must
    // fail the boot loudly rather than 404 in a browser tab.
    for (const asset of BRAND_ASSETS) {
      const body = readFileSync(new URL(`../public/${asset.file}`, import.meta.url))
      webCtx.effect(() => web.register({
        kind: 'exact',
        path: asset.path,
        handler: (_request, res) => {
          res.writeHead(200, { 'content-type': asset.type, 'cache-control': 'public, max-age=86400' })
          res.end(body)
        },
      }))
    }
    ctx.logger.info('landing-alioth: /landing mounted on webServer')
    ctx.logger.info(`landing-alioth: brand assets mounted (${BRAND_ASSETS.map(asset => asset.path).join(', ')})`)
  })
}
