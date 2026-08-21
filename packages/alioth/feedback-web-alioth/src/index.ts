/**
 * `@dsh-alioth/feedback-web-alioth` — the feedback CARRIER: a standalone
 * node:http server (default 127.0.0.1:14747, the AliothStudio convention)
 * exposing the annotation API over the `ctx.aliothFeedback` capability, the
 * long-poll watch seam, and the bookmarklet overlay.
 *
 * Trust boundary (ported from the AliothStudio original):
 * - Browser writes (POST sessions/annotations) require an Origin header
 *   hitting the allowlist (Config.allowedOrigins; defaults cover the local
 *   GUI origin). `Origin: null` (file:// prototypes) is opt-in via
 *   Config.allowNullOrigin — an opaque origin is forgeable via sandbox
 *   iframes.
 * - Consumer endpoints (pending/watch/patch/prune) answer loopback callers
 *   only — they are the agent/CLI seam, not the browser's.
 * @module @dsh-alioth/feedback-web-alioth
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { ANNOTATION_STATUSES, type AnnotationStatus } from '@dsh-alioth/feedback-alioth'
import { OVERLAY_JS } from './overlay.ts'

export const name = 'feedback-web-alioth'
export const inject = ['aliothFeedback']

export interface Config {
  /** Feedback server port (AliothStudio convention: 14747). */
  port?: number
  /** Bind address; loopback by default. */
  host?: string
  /** Allowed browser origins for annotation writes (CORS allowlist). */
  allowedOrigins?: string[]
  /** Allow `Origin: null` (file:// prototypes) — opt-in, forgeable via sandbox iframes. */
  allowNullOrigin?: boolean
}

export const Config: z<Config> = z.object({
  port: z.number().default(14747),
  host: z.string().default('127.0.0.1'),
  allowedOrigins: z.array(String).default(['http://127.0.0.1:3100', 'http://localhost:3100']),
  allowNullOrigin: z.boolean().default(false),
})

const BOOKMARKLET = "javascript:(function(){var s=document.createElement('script');s.src='%ORIGIN%/feedback/overlay.js';document.body.appendChild(s)})()"

function sendJson(response: ServerResponse, status: number, body: unknown, extra: Record<string, string> = {}): void {
  const payload = JSON.stringify(body)
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'content-length': Buffer.byteLength(payload), ...extra })
  response.end(payload)
}

function readBody(request: IncomingMessage): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on('data', chunk => { chunks.push(Buffer.from(chunk)) })
    request.on('end', () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}') as Record<string, unknown>)
      } catch (error) {
        reject(new Error(`invalid JSON body: ${error instanceof Error ? error.message : String(error)}`))
      }
    })
    request.on('error', reject)
  })
}

function isLoopback(request: IncomingMessage): boolean {
  const address = request.socket.remoteAddress ?? ''
  return address === '127.0.0.1' || address === '::1' || address === '::ffff:127.0.0.1'
}

/** The /feedback landing: bookmarklet + usage instructions. */
function feedbackPage(origin: string, allowed: readonly string[]): string {
  const bookmarklet = BOOKMARKLET.replace('%ORIGIN%', origin)
  return `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>批注工具 — Alioth Feedback</title>
<style>
body{background:#0a0e14;color:#d7e0ea;font-family:system-ui,"PingFang SC",sans-serif;line-height:1.7;
max-width:46rem;margin:0 auto;padding:2.5rem 1.5rem;
background-image:linear-gradient(rgba(62,230,168,.05) 1px,transparent 1px),linear-gradient(90deg,rgba(62,230,168,.05) 1px,transparent 1px);background-size:44px 44px}
h1{font-size:1.5rem}code{font-family:ui-monospace,monospace;color:#3ee6a8;background:#101724;border:1px solid #1e2a3a;border-radius:6px;padding:.15rem .45rem}
a{color:#4fc3f7}.bm{display:inline-block;padding:.6rem 1.2rem;background:#3ee6a8;color:#06251a;font-weight:700;border-radius:8px;text-decoration:none}
ol{margin:1rem 0 1.5rem;padding-left:1.4rem}li{margin:.4rem 0}
.allow{font-size:.85rem;color:#7d8ca0;margin-top:1.5rem}
</style></head><body>
<h1>页面批注工具</h1>
<p>在任意开发页面上圈选元素、留言批注——agent 通过 <code>alioth_feedback_*</code> 工具消费并修复。</p>
<ol>
<li>把下面的按钮拖到浏览器书签栏：</li>
<p><a class="bm" href="${bookmarklet}" draggable="true" onclick="return false">圈选批注</a></p>
<li>打开要调试的页面，点书签注入批注模式；</li>
<li><code>Alt + 点击</code> 目标元素，填写问题，提交；</li>
<li>在对话里让 agent「查看页面批注」即可进入消费闭环（ack → 修复 → resolve）。</li>
</ol>
<p>Overlay 直链（脚本注入器/扩展可复用）：<code>${origin}/feedback/overlay.js</code></p>
<p class="allow">允许批注的来源：${allowed.map(o => `<code>${o}</code>`).join(' · ')}${allowed.length === 0 ? '（空——仅消费者回环可用）' : ''}</p>
</body></html>`
}

export function apply(ctx: Context, config: Config): void {
  // Schema defaults are applied by the Loader; hand-built test contexts may
  // pass partials — normalize once at the boundary.
  const cfg = {
    port: config.port ?? 14747,
    host: config.host ?? '127.0.0.1',
    allowedOrigins: config.allowedOrigins ?? ['http://127.0.0.1:3100', 'http://localhost:3100'],
    allowNullOrigin: config.allowNullOrigin ?? false,
  }
  const allowed = new Set(cfg.allowedOrigins)
  const originAllowed = (request: IncomingMessage): boolean => {
    const origin = request.headers.origin
    if (origin === undefined) return false
    if (origin === 'null') return cfg.allowNullOrigin
    return allowed.has(origin)
  }
  const corsHeaders = (request: IncomingMessage): Record<string, string> => {
    const origin = request.headers.origin
    if (origin === undefined) return {}
    if (origin === 'null' ? cfg.allowNullOrigin : allowed.has(origin)) {
      return { 'access-control-allow-origin': origin, 'access-control-allow-headers': 'content-type', 'access-control-allow-methods': 'GET, POST, PATCH, OPTIONS' }
    }
    return {}
  }
  const origin = `http://${cfg.host === '0.0.0.0' ? '127.0.0.1' : cfg.host}:${cfg.port}`

  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', origin)
    try {
      if (request.method === 'OPTIONS') {
        response.writeHead(204, corsHeaders(request))
        response.end()
        return
      }
      // ── browser-facing (origin-allowlisted) ──────────────────────────
      if (request.method === 'GET' && url.pathname === '/health') {
        sendJson(response, 200, ctx.aliothFeedback.health())
        return
      }
      if (request.method === 'GET' && (url.pathname === '/feedback' || url.pathname === '/feedback/')) {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
        response.end(feedbackPage(origin, cfg.allowedOrigins))
        return
      }
      if (request.method === 'GET' && url.pathname === '/feedback/overlay.js') {
        response.writeHead(200, { 'content-type': 'application/javascript; charset=utf-8', 'cache-control': 'no-cache' })
        response.end(OVERLAY_JS)
        return
      }
      if (request.method === 'POST' && url.pathname === '/api/feedback/annotations') {
        if (!originAllowed(request)) {
          sendJson(response, 403, { error: 'origin not allowed' })
          return
        }
        const body = await readBody(request)
        const annotation = ctx.aliothFeedback.addAnnotation({
          ...(typeof body.sessionId === 'string' && body.sessionId !== '' ? { sessionId: body.sessionId } : {}),
          origin: String(body.origin ?? request.headers.origin ?? ''),
          url: String(body.url ?? ''),
          comment: String(body.comment ?? ''),
          ...(typeof body.element === 'string' ? { element: body.element } : {}),
          ...(typeof body.elementPath === 'string' ? { elementPath: body.elementPath } : {}),
          ...(typeof body.cssClasses === 'string' ? { cssClasses: body.cssClasses } : {}),
        })
        sendJson(response, 201, annotation, corsHeaders(request))
        return
      }
      // ── consumer-facing (loopback only: agent tools / CLI) ───────────
      if (!isLoopback(request)) {
        sendJson(response, 403, { error: 'loopback only' })
        return
      }
      if (request.method === 'GET' && url.pathname === '/api/feedback/pending') {
        sendJson(response, 200, ctx.aliothFeedback.pending())
        return
      }
      if (request.method === 'GET' && url.pathname === '/api/feedback/watch') {
        const timeout = Math.min(60_000, Math.max(0, Number(url.searchParams.get('timeout') ?? 25_000)))
        sendJson(response, 200, await ctx.aliothFeedback.watch(timeout))
        return
      }
      // 变更端点鉴权：回环是最低护栏；auth 能力存在时升级为管理员 bearer
      // （工具/CLI 携带管理员令牌；无 auth 的独立部署维持回环信任）。
      const requireAdmin = async (request: IncomingMessage): Promise<boolean> => {
        const authService = (ctx.get as (name: string) => unknown).call(ctx, 'aliothAuth') as
          { userForToken(token: string | null): Promise<{ role: 'admin' | 'user' } | null> } | undefined
        if (authService === undefined) return true
        const header = request.headers.authorization
        const token = /^Bearer\s+(.+)$/i.exec(header ?? '')?.[1] ?? null
        const user = await authService.userForToken(token)
        return user?.role === 'admin'
      }
      const patchMatch = /^\/api\/feedback\/annotations\/([0-9a-f-]+)$/.exec(url.pathname)
      if (request.method === 'PATCH' && patchMatch !== null) {
        if (!(await requireAdmin(request))) {
          sendJson(response, 401, { error: 'admin token required' })
          return
        }
        const body = await readBody(request)
        const status = ANNOTATION_STATUSES.includes(body.status as AnnotationStatus)
          ? body.status as AnnotationStatus
          : undefined
        const annotation = ctx.aliothFeedback.setStatus(patchMatch[1]!, status, typeof body.reply === 'string' ? body.reply : undefined)
        sendJson(response, 200, annotation)
        return
      }
      if (request.method === 'POST' && url.pathname === '/api/feedback/prune') {
        if (!(await requireAdmin(request))) {
          sendJson(response, 401, { error: 'admin token required' })
          return
        }
        sendJson(response, 200, { pruned: ctx.aliothFeedback.prune(24 * 3600 * 1000) })
        return
      }
      sendJson(response, 404, { error: 'not found' })
    } catch (error) {
      sendJson(response, 400, { error: error instanceof Error ? error.message : String(error) })
    }
  })

  server.once('error', error => {
    ctx.logger.error(`feedback-web-alioth: HTTP server failed: ${error instanceof Error ? error.message : String(error)}`)
  })
  server.listen(cfg.port, cfg.host)
  ctx.logger.info(`feedback-web-alioth: annotation API on ${origin} (bookmarklet: ${origin}/feedback)`)

  ctx.effect(() => () => {
    server.close()
  })
}
