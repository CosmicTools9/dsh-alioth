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

export interface Config {}

export const Config: z<Config> = z.object({})

/** Structural face of the harness `webServer` service — no runtime dependency
 * on dsh-host-webserver; composed web deployments provide the real one. */
interface WebServerLike {
  register(route: {
    kind: 'exact' | 'prefix'
    path: string
    handler: (req: IncomingMessage, res: ServerResponse) => void | Promise<void>
  }): () => void
}

function asWebServer(value: unknown): WebServerLike | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined
  }
  const candidate = value as Record<string, unknown>
  return typeof candidate.register === 'function' ? value as WebServerLike : undefined
}

export function apply(ctx: Context, _config: Config): void {
  void ctx
  const html = readFileSync(new URL('../public/landing.html', import.meta.url), 'utf8')
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
    ctx.logger.info('landing-alioth: /landing mounted on webServer')
  })
}
