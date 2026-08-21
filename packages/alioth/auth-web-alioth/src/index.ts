/**
 * `@dsh-alioth/auth-web-alioth` — the B/S auth surface CARRIER over the
 * `ctx.aliothAuth` capability (auth-alioth owns the service and guards).
 *
 * Two transports, one handler set:
 * - standalone node:http server (config.port, default 3900) — the headless /
 *   direct B/S entry;
 * - harness `webServer` routes (web profile, same-origin with the GUI):
 *   `/login`, `/register`, prefix `/api/auth/*`, plus a tapIndex gate script
 *   that bounces cookie-less visitors to the landing page
 *   (`ctx.aliothLanding`, fallback `/login`) and binds every
 *   `sessions.create` result to the caller's token.
 *
 * Browser form posts (urlencoded) get styled HTML pages; API clients (JSON)
 * get JSON. Logins set an HttpOnly `alioth_session` cookie plus a
 * JS-readable `alioth_user` marker (the gate script's presence check).
 *
 * Client face: `lib/client.js` (dsh.client) renders the logged-in user chip
 * in the harness `shell.overlay` slot.
 * @module @dsh-alioth/auth-web-alioth
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'

export const name = 'auth-web-alioth'
export const inject = ['aliothAuth']

export interface Config {
  /** HTTP port for the standalone auth server. */
  readonly port: number
  /** Session lifetime in seconds (cookie Max-Age); default 7 days. */
  readonly sessionTtlSeconds?: number
  /** When the harness `webServer` service is present (web profile), mount the
   * same-origin auth surface + login gate on it. Default true. */
  readonly webGate?: boolean
}

export const Config: z<Config> = z.object({
  port: z.number().default(3900),
  sessionTtlSeconds: z.number().default(7 * 24 * 3600),
  webGate: z.boolean().default(true),
})

/** The landing capability face (structural — landing-alioth provides it). */
interface LandingLike {
  readonly path: string
  readonly html: string
}

/** Structural face of the harness `webServer` service — no runtime dependency
 * on dsh-host-webserver; composed web deployments provide the real one. */
interface WebServerLike {
  register(route: {
    kind: 'exact' | 'prefix'
    path: string
    handler: (req: IncomingMessage, res: ServerResponse) => void | Promise<void>
  }): () => void
  tapIndex(transform: (html: string) => string): () => void
  /** Listen address (optional: the real service exposes both). */
  readonly host?: string
  /** Listening port (optional; the OS-assigned value when configured 0). */
  readonly port?: number
}

function asWebServer(value: unknown): WebServerLike | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined
  }
  const candidate = value as Record<string, unknown>
  return typeof candidate.register === 'function' && typeof candidate.tapIndex === 'function'
    ? value as WebServerLike
    : undefined
}

// ── request/response helpers ────────────────────────────────────────────

/** Parse the request body for both content types the B/S surface uses:
 * application/json (API clients) and application/x-www-form-urlencoded
 * (browser form submissions from the login/register pages). */
function readBody(request: IncomingMessage): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on('data', chunk => { chunks.push(Buffer.from(chunk)) })
    request.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8')
      const contentType = request.headers['content-type'] ?? ''
      try {
        if (contentType.includes('application/json')) {
          resolve(JSON.parse(raw || '{}') as Record<string, unknown>)
        } else if (contentType.includes('application/x-www-form-urlencoded')) {
          const params = new URLSearchParams(raw)
          const body: Record<string, unknown> = {}
          for (const [key, value] of params.entries()) {
            body[key] = value
          }
          resolve(body)
        } else {
          resolve({})
        }
      } catch (error) {
        reject(new Error(`invalid request body: ${error instanceof Error ? error.message : String(error)}`))
      }
    })
    request.on('error', reject)
  })
}

function sendJson(response: ServerResponse, status: number, body: unknown, headers: Record<string, string | string[]> = {}): void {
  const payload = JSON.stringify(body)
  response.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
    ...headers,
  })
  response.end(payload)
}

function bearerToken(request: IncomingMessage): string | null {
  const header = request.headers.authorization
  if (header === undefined) {
    return null
  }
  const match = /^Bearer\s+(.+)$/i.exec(header)
  return match === null ? null : match[1]!
}

/** HTML-escape user-adjacent strings before interpolating into pages. */
function esc(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

/** Browser-facing error text: strip the service prefix, keep the reason. */
function friendlyError(error: unknown, op: 'login' | 'register'): string {
  const message = error instanceof Error ? error.message : String(error)
  if (op === 'login' && /invalid credentials/.test(message)) {
    return '用户名或密码错误'
  }
  return message.replace(/^aliothAuth\.\w+: /, '')
}

/** Browser form posts (urlencoded) get styled HTML; API clients (json) get JSON. */
function isFormPost(request: IncomingMessage): boolean {
  return (request.headers['content-type'] ?? '').includes('application/x-www-form-urlencoded')
}

// ── styled auth pages (visual kin of landing.html) ──────────────────────

/** Shared dark-tech chrome for the B/S auth pages — same palette, zero
 * external assets. */
function sendAuthPage(response: ServerResponse, status: number, title: string, body: string, headers: Record<string, string | string[]> = {}): void {
  response.writeHead(status, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache', ...headers })
  response.end(`<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title} — Alioth AppCreator</title>
<style>
:root{--bg:#0a0e14;--panel:#101724;--line:#1e2a3a;--text:#d7e0ea;--dim:#7d8ca0;
--accent:#3ee6a8;--accent-2:#4fc3f7;--error:#f2718a;
--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);min-height:100vh;display:flex;flex-direction:column;
font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif;line-height:1.6;
background-image:linear-gradient(rgba(62,230,168,.05) 1px,transparent 1px),
linear-gradient(90deg,rgba(62,230,168,.05) 1px,transparent 1px);background-size:44px 44px}
a{color:var(--accent-2);text-decoration:none}
nav{display:flex;justify-content:space-between;align-items:center;
max-width:1080px;width:100%;margin:0 auto;padding:1.25rem 1.5rem}
.wordmark{font-family:var(--mono);font-weight:700;letter-spacing:.04em}
.wordmark span{color:var(--accent)}
main{flex:1;display:flex;align-items:center;justify-content:center;padding:2rem 1.5rem}
.card{background:var(--panel);border:1px solid var(--line);border-radius:10px;
padding:2rem;width:100%;max-width:24rem}
.card h1{font-size:1.4rem;margin-bottom:1.25rem}
form{display:grid;gap:.9rem}
label{display:grid;gap:.35rem;font-size:.88rem;color:var(--dim)}
input{background:#070b11;border:1px solid var(--line);border-radius:6px;color:var(--text);
padding:.55rem .7rem;font-size:.95rem;outline:none;transition:border-color .2s}
input:focus{border-color:var(--accent)}
button{margin-top:.4rem;background:var(--accent);border:1px solid var(--accent);border-radius:6px;
color:#06251a;font-weight:600;font-size:.95rem;padding:.6rem;cursor:pointer}
button:hover{filter:brightness(1.1)}
.banner{border-radius:6px;padding:.55rem .8rem;font-size:.88rem;margin-bottom:1rem}
.banner.error{border:1px solid var(--error);color:var(--error);background:rgba(242,113,138,.08)}
.banner.ok{border:1px solid var(--accent);color:var(--accent);background:rgba(62,230,168,.08)}
.alt{margin-top:1.1rem;font-size:.85rem;color:var(--dim)}
.hint{font-size:.85rem;color:var(--dim);margin:.9rem 0 .4rem}
.token{font-family:var(--mono);font-size:.8rem;word-break:break-all;background:#070b11;
border:1px solid var(--line);border-radius:6px;padding:.7rem;color:var(--accent);user-select:all}
code{font-family:var(--mono);color:var(--accent)}
</style></head><body>
<nav><a class="wordmark" href="/">Alioth<span>·</span>AppCreator</a></nav>
<main><div class="card"><h1>${title}</h1>${body}</div></main>
</body></html>`)
}

function loginForm(error: string): string {
  return `${error === '' ? '' : `<p class="banner error">${esc(error)}</p>`}
<form method="post" action="/api/auth/login">
<label>用户名<input name="username" required autocomplete="username"></label>
<label>密码<input name="password" type="password" required autocomplete="current-password"></label>
<button>登录</button></form>
<p class="alt">还没有账号？<a href="/register">注册</a> · <a href="/">返回首页</a></p>`
}

function registerForm(error: string): string {
  return `${error === '' ? '' : `<p class="banner error">${esc(error)}</p>`}
<form method="post" action="/api/auth/register">
<label>用户名（小写字母/数字/连字符，≥3 位）<input name="username" required autocomplete="username"></label>
<label>密码（≥8 位）<input name="password" type="password" required autocomplete="new-password"></label>
<button>注册</button></form>
<p class="alt">已有账号？<a href="/login">登录</a> · <a href="/">返回首页</a></p>`
}

/** One-time token reveal after a browser form login/register. */
function successBody(action: string, token: string, namespace: string, workspaceHref: string): string {
  const handoff = workspaceHref === '/'
    ? `<p class="alt"><a href="/">进入工作台</a></p>`
    : `<form method="post" action="${esc(workspaceHref)}/api/auth/accept" style="margin-top:1rem">
<input type="hidden" name="token" value="${token}">
<button>进入工作台</button></form>`
  return `<p class="banner ok">${action}成功 — 命名空间 <code>${esc(namespace)}</code></p>
<p class="hint">会话令牌（仅此一次显示，请立即保存）：</p>
<p class="token">${token}</p>
${handoff}`
}

// ── workspace page (工作区 local / 应用 production) ────────────────────

/** Extended chrome: wider card + list rows for the workspace browser. */
function sendWorkspacePage(
  response: ServerResponse,
  environment: 'local' | 'production',
  list: ReadonlyArray<{
    namespace: string
    preProcPath: string
    deployPath: string
    apps: ReadonlyArray<{ code: string; name: string }>
  }>,
): void {
  const title = environment === 'local' ? '工作区' : '应用'
  const rows = list.map(ws => `
<article class="ws">
  <header><h2>${esc(ws.namespace)}</h2>${environment === 'local'
    ? `<span class="count">${ws.apps.length} 个应用</span>` : ''}</header>
  ${environment === 'local' ? `
  <p class="paths"><code>Pre-Proc/${esc(ws.namespace)}/</code></p>
  <p class="paths"><code>Deploy/${esc(ws.namespace)}/</code></p>` : ''}
  ${ws.apps.length === 0 ? '<p class="dim">暂无应用 — 在对话中让 Alioth 助手创建</p>' : `
  <ul class="apps">${ws.apps.map(app => `<li><span class="code">${esc(app.code)}</span>${app.name === '' ? '' : ` — ${esc(app.name)}`}</li>`).join('')}</ul>`}
</article>`).join('')
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
  response.end(`<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title} — Alioth AppCreator</title>
<style>
:root{--bg:#0a0e14;--panel:#101724;--line:#1e2a3a;--text:#d7e0ea;--dim:#7d8ca0;
--accent:#3ee6a8;--accent-2:#4fc3f7;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);min-height:100vh;display:flex;flex-direction:column;
font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif;line-height:1.6;
background-image:linear-gradient(rgba(62,230,168,.05) 1px,transparent 1px),
linear-gradient(90deg,rgba(62,230,168,.05) 1px,transparent 1px);background-size:44px 44px}
a{color:var(--accent-2);text-decoration:none}
nav{display:flex;justify-content:space-between;align-items:center;
max-width:1080px;width:100%;margin:0 auto;padding:1.25rem 1.5rem}
.wordmark{font-family:var(--mono);font-weight:700;letter-spacing:.04em}
.wordmark span{color:var(--accent)}
main{flex:1;width:100%;max-width:1080px;margin:0 auto;padding:0 1.5rem 3rem}
h1{font-size:1.4rem;margin:1rem 0 1.25rem}
.ws{background:var(--panel);border:1px solid var(--line);border-radius:10px;
padding:1.1rem 1.25rem;margin-bottom:1rem}
.ws header{display:flex;justify-content:space-between;align-items:baseline;gap:1rem}
.ws h2{font-family:var(--mono);font-size:1.05rem;color:var(--accent)}
.count{font-size:.8rem;color:var(--dim)}
.paths{font-size:.82rem;color:var(--dim);margin-top:.35rem}
.paths code{color:var(--accent-2)}
.dim{color:var(--dim);font-size:.88rem;margin-top:.5rem}
.apps{list-style:none;margin-top:.6rem;display:grid;gap:.3rem}
.apps li{font-size:.92rem}
.apps .code{font-family:var(--mono);color:var(--text)}
.back{margin-top:1.25rem;font-size:.85rem;color:var(--dim)}
</style></head><body>
<nav><a class="wordmark" href="/">Alioth<span>·</span>AppCreator</a></nav>
<main><h1>${title}</h1>${rows === '' ? '<p class="dim">（空）</p>' : rows}
<p class="back"><a href="/usercenter">← 用户中心</a> · <a href="/">返回首页</a></p></main>
</body></html>`)
}

// ── cookies ──────────────────────────────────────────────────────────────
/** HttpOnly session cookie (server-side authority). */
const SESSION_COOKIE = 'alioth_session'
/** JS-readable marker cookie: the tapIndex gate script checks presence only. */
const MARKER_COOKIE = 'alioth_user'

function authCookies(token: string, username: string, maxAgeSeconds: number): string[] {
  return [
    `${SESSION_COOKIE}=${token}; HttpOnly; Path=/; SameSite=Lax; Max-Age=${maxAgeSeconds}`,
    `${MARKER_COOKIE}=${encodeURIComponent(username)}; Path=/; SameSite=Lax; Max-Age=${maxAgeSeconds}`,
  ]
}

const CLEAR_COOKIES: string[] = [
  `${SESSION_COOKIE}=; HttpOnly; Path=/; SameSite=Lax; Max-Age=0`,
  `${MARKER_COOKIE}=; Path=/; SameSite=Lax; Max-Age=0`,
]

/** Session token from the HttpOnly cookie (browser clients), else null. */
function cookieToken(request: IncomingMessage): string | null {
  const header = request.headers.cookie
  if (header === undefined) {
    return null
  }
  for (const part of header.split(';')) {
    const [name, ...rest] = part.trim().split('=')
    if (name === SESSION_COOKIE) {
      return rest.join('=')
    }
  }
  return null
}

// ── gate script (tapIndex) ──────────────────────────────────────────────

function gateScript(landingPath: string): string {
  return `<script>(function(){`
    + `if(!/(^|;)\\s*${MARKER_COOKIE}=/.test(document.cookie)){location.replace('${landingPath}');return;}`
    // Bind every sessions.create result to the caller's identity: the
    // harness API is in-process — HTTP identity never reaches tool execution;
    // only session binding does. Identity rides the same-origin session
    // cookie (no raw token in the browser's storage).
    + `var of=window.fetch;`
    + `window.fetch=function(i){var a=arguments;return of.apply(this,a).then(function(res){`
    + `try{var u=typeof i==='string'?i:(i&&i.url)||'';`
    + `if(u.indexOf('/api/sessions.create')!==-1&&res.ok){res.clone().json().then(function(b){`
    + `var p=b&&(b.payload||b);var sid=p&&p.sessionId;`
    + `if(sid){of('/api/auth/bind',{method:'POST',headers:{'content-type':'application/json'},`
    + `body:JSON.stringify({sessionId:sid})}).catch(function(){});}`
    + `}).catch(function(){});}}catch(e){}`
    + `return res;});};`
    + `})();</script>`
}

export function apply(ctx: Context, config: Config): void {
  const ttlSeconds = config.sessionTtlSeconds ?? 7 * 24 * 3600
  const auth = () => ctx.aliothAuth
  /** GUI origin once the webServer carrier mounts (set by the inject callback
   * below); the standalone success page links across origins with it —
   * cookies are per-origin, so a same-origin "/" link would silently keep
   * the visitor unauthenticated on the GUI. */
  let guiOrigin: string | undefined
  /** Bind host of the harness webServer (environment auto-detection hint). */
  let webHostHint: string | undefined
  const workspaceHref = (): string => guiOrigin ?? '/'
  /** Landing capability lookup at request/tap time (optional provider). */
  const landing = (): LandingLike | undefined => {
    const value = (ctx.get as (name: string) => unknown).call(ctx, 'aliothLanding')
    if (typeof value !== 'object' || value === null) {
      return undefined
    }
    const candidate = value as Record<string, unknown>
    return typeof candidate.path === 'string' && typeof candidate.html === 'string'
      ? value as LandingLike
      : undefined
  }

  /** Shared auth API surface — the standalone server and the webServer
   * prefix mount both route here. Owns every /api/auth/* path. */
  const handleAuthApi = async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    if (request.method === 'POST' && url.pathname === '/api/auth/register') {
      const body = await readBody(request)
      const username = typeof body.username === 'string' ? body.username : ''
      const password = typeof body.password === 'string' ? body.password : ''
      if (isFormPost(request)) {
        try {
          const result = await auth().register(username, password)
          sendAuthPage(response, 201, '注册', successBody('注册', result.token, result.namespace, workspaceHref()),
            { 'set-cookie': authCookies(result.token, username, ttlSeconds) })
        } catch (error) {
          sendAuthPage(response, 400, '注册', registerForm(friendlyError(error, 'register')))
        }
        return
      }
      const result = await auth().register(username, password)
      sendJson(response, 201, result, { 'set-cookie': authCookies(result.token, username, ttlSeconds) })
      return
    }
    if (request.method === 'POST' && url.pathname === '/api/auth/login') {
      const body = await readBody(request)
      const username = typeof body.username === 'string' ? body.username : ''
      const password = typeof body.password === 'string' ? body.password : ''
      if (isFormPost(request)) {
        try {
          const result = await auth().login(username, password)
          sendAuthPage(response, 200, '登录', successBody('登录', result.token, result.namespace, workspaceHref()),
            { 'set-cookie': authCookies(result.token, username, ttlSeconds) })
        } catch (error) {
          sendAuthPage(response, 401, '登录', loginForm(friendlyError(error, 'login')))
        }
        return
      }
      const result = await auth().login(username, password)
      sendJson(response, 200, result, { 'set-cookie': authCookies(result.token, username, ttlSeconds) })
      return
    }
    if (request.method === 'POST' && url.pathname === '/api/auth/logout') {
      await auth().logout(bearerToken(request) ?? cookieToken(request))
      sendJson(response, 204, null, { 'set-cookie': CLEAR_COOKIES })
      return
    }
    if (request.method === 'GET' && url.pathname === '/api/auth/me') {
      const user = await auth().userForToken(bearerToken(request) ?? cookieToken(request))
      if (user === null) {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      sendJson(response, 200, { ...user, environment: auth().environment(webHostHint) })
      return
    }
    // Workspace browser: the environment decides the presentation — local
    // exposes the 工作区 list (custom workspaces), production is fixed to
    // the user's namespace shown as 应用.
    if (request.method === 'GET' && url.pathname === '/api/workspace') {
      const user = await auth().userForToken(bearerToken(request) ?? cookieToken(request))
      if (user === null) {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      sendJson(response, 200, await auth().workspaces({ namespace: user.namespace, role: user.role }))
      return
    }
    // Cross-origin SSO handoff: the standalone (:3900) success page
    // auto-POSTs the fresh token here so the GUI origin gets its own
    // cookies — cookies are per-origin, a bare cross-origin link would
    // silently land the visitor back on the landing page.
    if (request.method === 'POST' && url.pathname === '/api/auth/accept') {
      const body = await readBody(request)
      const token = typeof body.token === 'string' ? body.token : ''
      const user = await auth().userForToken(token)
      if (user === null) {
        response.writeHead(302, { location: '/login' })
        response.end()
        return
      }
      response.writeHead(302, { location: '/', 'set-cookie': authCookies(token, user.username, ttlSeconds) })
      response.end()
      return
    }
    // Bind a freshly created agent session to the caller (the web gate
    // script calls this after every sessions.create; the harness API is
    // in-process, so session binding is the only identity carrier). Identity:
    // explicit token (API clients) or the same-origin session cookie.
    if (request.method === 'POST' && url.pathname === '/api/auth/bind') {
      const body = await readBody(request)
      const token = typeof body.token === 'string' && body.token !== '' ? body.token : (bearerToken(request) ?? cookieToken(request) ?? '')
      const sessionId = typeof body.sessionId === 'string' ? body.sessionId : ''
      const user = await auth().userForToken(token)
      if (user === null || sessionId === '') {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      await auth().bind(token, sessionId)
      sendJson(response, 204, null)
      return
    }
    sendJson(response, 404, { error: 'not found' })
  }

  // ── standalone HTTP server ────────────────────────────────────────────
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    try {
      if (request.method === 'GET' && url.pathname === '/') {
        const l = landing()
        if (l !== undefined) {
          response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
          response.end(l.html)
        } else {
          response.writeHead(302, { location: '/login' })
          response.end()
        }
        return
      }
      if (request.method === 'GET' && url.pathname === '/login') {
        sendAuthPage(response, 200, '登录', loginForm(''))
        return
      }
      if (request.method === 'GET' && url.pathname === '/register') {
        sendAuthPage(response, 200, '注册', registerForm(''))
        return
      }
      if (request.method === 'GET' && url.pathname === '/workspace') {
        const user = await auth().userForToken(bearerToken(request) ?? cookieToken(request))
        if (user === null) {
          response.writeHead(302, { location: '/login' })
          response.end()
          return
        }
        const list = await auth().workspaces({ namespace: user.namespace, role: user.role })
        sendWorkspacePage(response, list.environment, list.workspaces)
        return
      }
      if (url.pathname === '/api/auth' || url.pathname.startsWith('/api/auth/')
        || url.pathname === '/api/workspace' || url.pathname.startsWith('/api/workspace/')) {
        await handleAuthApi(request, response)
        return
      }
      sendJson(response, 404, { error: 'not found' })
    } catch (error) {
      sendJson(response, 400, { error: error instanceof Error ? error.message : String(error) })
    }
  })

  server.once('error', error => {
    ctx.logger.error(`auth-web-alioth: HTTP server failed: ${error instanceof Error ? error.message : String(error)}`)
  })
  server.listen(config.port)
  ctx.logger.info(`auth-web-alioth: B/S auth API on :${config.port}`)

  ctx.effect(() => () => {
    server.close()
  })

  // ── web gate: same-origin auth surface on the harness webServer ────────
  // Web profile only: /login, /register, prefix /api/auth/* mount on the GUI
  // origin (longest-prefix-wins over the client-connection /api route), and
  // the index gate script bounces unauthenticated visitors to the landing
  // page. webServer is a Service that may not be visible at apply() time —
  // defer through ctx.inject like the harness's own carrier plugins do.
  if (config.webGate !== false) {
    const inject = ctx.inject as (deps: string[], cb: (webCtx: Context) => void) => void
    inject.call(ctx, ['webServer'], webCtx => {
      const web = asWebServer((webCtx.get as (name: string) => unknown).call(webCtx, 'webServer'))
      if (web === undefined) {
        ctx.logger.warn('auth-web-alioth: webServer present but shape mismatch — web gate not mounted')
        return
      }
      webCtx.effect(() => web.register({
        kind: 'exact',
        path: '/login',
        handler: (_request, res) => {
          sendAuthPage(res, 200, '登录', loginForm(''))
        },
      }))
      webCtx.effect(() => web.register({
        kind: 'exact',
        path: '/register',
        handler: (_request, res) => {
          sendAuthPage(res, 200, '注册', registerForm(''))
        },
      }))
      webCtx.effect(() => web.register({
        kind: 'exact',
        path: '/workspace',
        handler: async (req, res) => {
          const user = await auth().userForToken(bearerToken(req) ?? cookieToken(req))
          if (user === null) {
            res.writeHead(302, { location: '/login' })
            res.end()
            return
          }
          const list = await auth().workspaces({ namespace: user.namespace, role: user.role })
          sendWorkspacePage(res, list.environment, list.workspaces)
        },
      }))
      webCtx.effect(() => web.register({
        kind: 'prefix',
        path: '/api/auth',
        handler: async (req, res) => {
          await handleAuthApi(req, res)
        },
      }))
      webCtx.effect(() => web.register({
        kind: 'prefix',
        path: '/api/workspace',
        handler: async (req, res) => {
          await handleAuthApi(req, res)
        },
      }))
      // The gate target resolves per tap: landing plugin when present,
      // otherwise the login page (no-landing compositions stay functional).
      webCtx.effect(() => web.tapIndex(html => {
        const target = landing()?.path ?? '/login'
        return html.includes('</head>') ? html.replace('</head>', `${gateScript(target)}</head>`) : html
      }))
      if (typeof web.port === 'number') {
        guiOrigin = `http://${typeof web.host === 'string' ? web.host : '127.0.0.1'}:${web.port}`
        webHostHint = typeof web.host === 'string' ? web.host : undefined
      }
      ctx.logger.info('auth-web-alioth: web gate mounted on webServer (login/register + /api/auth/* + index gate)')
    })
  }
}
