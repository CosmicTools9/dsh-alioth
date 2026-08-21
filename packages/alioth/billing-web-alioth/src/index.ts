/**
 * `@dsh-alioth/billing-web-alioth` — the user center CARRIER over the
 * `ctx.aliothBilling` capability: server-rendered pages (same dark-tech
 * chrome as the auth pages) + a JSON API twin, mounted same-origin on the
 * harness `webServer` (web profile). Cookie-authenticated via the auth
 * capability (`alioth_session`); unauthenticated visits bounce to /login.
 *
 * Pages: /usercenter (概览) · /usercenter/subscription (订阅) ·
 * /usercenter/bills (账单) · /usercenter/invoices (发票).
 * API: GET /api/billing/overview · POST /api/billing/subscribe|cancel|pay|
 * invoice|issue (form urlencoded → styled redirect; JSON → JSON).
 *
 * Payment is OFFLINE (线下确认) until a PSP lands; the admin invoice queue
 * lives on the invoices page for role=admin.
 * @module @dsh-alioth/billing-web-alioth
 */

import type { IncomingMessage, ServerResponse } from 'node:http'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import type { Bill, Invoice, PendingInvoice, Subscription } from '@dsh-alioth/billing-alioth'

export const name = 'billing-web-alioth'
export const inject = ['aliothBilling', 'aliothAuth']

export interface Config {}

export const Config: z<Config> = z.object({})

interface AuthedUser {
  readonly id: string
  readonly username: string
  readonly namespace: string
  readonly role: 'admin' | 'user'
}

/** Structural face of the harness `webServer` service (no runtime dep). */
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

// ── helpers ──────────────────────────────────────────────────────────────

function esc(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function yuan(cents: number): string {
  return `¥${(cents / 100).toLocaleString('zh-CN')}`
}

function bearerToken(request: IncomingMessage): string | null {
  const header = request.headers.authorization
  if (header === undefined) return null
  const match = /^Bearer\s+(.+)$/i.exec(header)
  return match === null ? null : match[1]!
}

function cookieToken(request: IncomingMessage): string | null {
  const header = request.headers.cookie
  if (header === undefined) return null
  for (const part of header.split(';')) {
    const [name, ...rest] = part.trim().split('=')
    if (name === 'alioth_session') return rest.join('=')
  }
  return null
}

function isFormPost(request: IncomingMessage): boolean {
  return (request.headers['content-type'] ?? '').includes('application/x-www-form-urlencoded')
}

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
          for (const [key, value] of params.entries()) body[key] = value
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

function sendJson(response: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body)
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'content-length': Buffer.byteLength(payload) })
  response.end(payload)
}

// ── page chrome (visual kin of the auth pages / landing) ─────────────────

function page(response: ServerResponse, status: number, title: string, body: string): void {
  response.writeHead(status, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
  response.end(`<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title} — 用户中心 · Alioth AppCreator</title>
<style>
:root{--bg:#0a0e14;--panel:#101724;--line:#1e2a3a;--text:#d7e0ea;--dim:#7d8ca0;
--accent:#3ee6a8;--accent-2:#4fc3f7;--warn:#f2718a;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);min-height:100vh;
font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif;line-height:1.6;
background-image:linear-gradient(rgba(62,230,168,.05) 1px,transparent 1px),
linear-gradient(90deg,rgba(62,230,168,.05) 1px,transparent 1px);background-size:44px 44px}
a{color:var(--accent-2);text-decoration:none}
.wrap{max-width:960px;margin:0 auto;padding:0 1.5rem 3rem}
nav{display:flex;justify-content:space-between;align-items:center;max-width:960px;margin:0 auto;padding:1.25rem 1.5rem}
.wordmark{font-family:var(--mono);font-weight:700}
.wordmark span{color:var(--accent)}
h1{font-size:1.5rem;margin:1rem 0 .3rem}
.sub{color:var(--dim);font-size:.9rem;margin-bottom:1.5rem}
.tabs{display:flex;gap:.5rem;margin-bottom:1.5rem;flex-wrap:wrap}
.tabs a{padding:.4rem 1rem;border:1px solid var(--line);border-radius:999px;font-size:.88rem;color:var(--dim)}
.tabs a.active{border-color:var(--accent);color:var(--accent)}
.panel{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:1.25rem;margin-bottom:1rem}
.panel h2{font-size:1.05rem;margin-bottom:.8rem}
.kv{display:grid;grid-template-columns:9rem 1fr;gap:.4rem;font-size:.9rem}
.kv dt{color:var(--dim)}
.kv dd{font-family:var(--mono);color:var(--accent)}
table{width:100%;border-collapse:collapse;font-size:.88rem}
th{text-align:left;color:var(--dim);font-weight:400;padding:.45rem .5rem;border-bottom:1px solid var(--line)}
td{padding:.5rem;border-bottom:1px solid var(--line)}
td.num,th.num{text-align:right;font-family:var(--mono)}
.pill{display:inline-block;padding:.05rem .55rem;border-radius:999px;font-size:.76rem;border:1px solid var(--line);color:var(--dim)}
.pill.ok{border-color:var(--accent);color:var(--accent)}
.pill.warn{border-color:var(--warn);color:var(--warn)}
.btn{display:inline-block;padding:.35rem .9rem;border-radius:6px;border:1px solid var(--accent);
background:var(--accent);color:#06251a;font-weight:600;font-size:.85rem;cursor:pointer}
.btn.ghost{background:none;border-color:var(--line);color:var(--text)}
form.inline{display:inline}
form.grid{display:grid;gap:.8rem;max-width:26rem}
label{display:grid;gap:.3rem;font-size:.85rem;color:var(--dim)}
input,select{background:#070b11;border:1px solid var(--line);border-radius:6px;color:var(--text);
padding:.5rem .65rem;font-size:.92rem;outline:none}
input:focus,select:focus{border-color:var(--accent)}
.banner{border-radius:6px;padding:.55rem .8rem;font-size:.86rem;margin-bottom:1rem}
.banner.error{border:1px solid var(--warn);color:var(--warn);background:rgba(242,113,138,.08)}
.banner.ok{border:1px solid var(--accent);color:var(--accent);background:rgba(62,230,168,.08)}
.note{color:var(--dim);font-size:.8rem;margin-top:.6rem}
.tiers{display:grid;grid-template-columns:repeat(2,1fr);gap:1rem}
@media (max-width:640px){.tiers{grid-template-columns:1fr}}
.tier{border:1px solid var(--line);border-radius:10px;padding:1rem;background:var(--panel)}
.tier.current{border-color:var(--accent)}
.tier h3{font-size:.95rem;margin-bottom:.3rem}
.tier .price{font-family:var(--mono);color:var(--accent);margin:.4rem 0}
.tier p{font-size:.82rem;color:var(--dim)}
</style></head><body>
<nav><a class="wordmark" href="/">Alioth<span>·</span>AppCreator</a><a href="/" style="font-size:.85rem;color:var(--dim)">← 返回首页</a></nav>
<div class="wrap">${body}</div>
</body></html>`)
}

// ── page bodies ──────────────────────────────────────────────────────────

interface ViewData {
  user: AuthedUser
  subscription: Subscription | null
  bills: Bill[]
  invoices: Invoice[]
  pending: PendingInvoice[]
  notice: string
  error: string
}

function tabs(active: string): string {
  const items = [['', '概览'], ['/subscription', '订阅'], ['/bills', '账单'], ['/invoices', '发票']]
  return `<div class="tabs">${items.map(([href, label]) =>
    `<a class="${href === active ? 'active' : ''}" href="/usercenter${href}">${label}</a>`).join('')}</div>`
}

function banners(data: ViewData): string {
  return `${data.notice === '' ? '' : `<p class="banner ok">${esc(data.notice)}</p>`}${data.error === '' ? '' : `<p class="banner error">${esc(data.error)}</p>`}`
}

function overviewBody(data: ViewData): string {
  const { user, subscription, bills, invoices } = data
  const unpaid = bills.filter(b => b.status === 'unpaid').length
  const pendingInv = invoices.filter(i => i.status === 'pending').length
  return `<h1>用户中心</h1><p class="sub">${esc(user.username)} · ${esc(user.namespace)}${user.role === 'admin' ? ' · 管理员' : ''}</p>
${tabs('')}
${banners(data)}
<div class="panel"><h2>账户</h2><dl class="kv">
<dt>用户名</dt><dd>${esc(user.username)}</dd>
<dt>命名空间</dt><dd>${esc(user.namespace)}</dd>
<dt>角色</dt><dd>${user.role === 'admin' ? 'admin（管理员）' : 'user'}</dd>
</dl></div>
<div class="panel"><h2>订阅</h2><dl class="kv">
<dt>当前套餐</dt><dd>${subscription?.status === 'active' ? 'L1 订阅版' : 'L0 社区版（免费）'}</dd>
<dt>状态</dt><dd>${subscription === null ? '—' : subscription.status === 'active' ? '生效中' : '已取消'}</dd>
<dt>下次续期</dt><dd>${subscription?.status === 'active' ? subscription.renewsAt.toISOString().slice(0, 10) : '—'}</dd>
</dl><p class="note"><a href="/usercenter/subscription">管理订阅 →</a></p></div>
<div class="panel"><h2>账单与发票</h2><dl class="kv">
<dt>待支付账单</dt><dd>${unpaid}</dd>
<dt>待开具发票</dt><dd>${pendingInv}</dd>
</dl><p class="note"><a href="/usercenter/bills">查看账单 →</a> · <a href="/usercenter/invoices">申请发票 →</a></p></div>`
}

function subscriptionBody(data: ViewData): string {
  const { subscription } = data
  const active = subscription?.status === 'active'
  return `<h1>订阅</h1><p class="sub">从社区版到私有化的阶梯 — 当前：${active ? 'L1 订阅版' : 'L0 社区版'}</p>
${tabs('/subscription')}
${banners(data)}
<div class="tiers">
<div class="tier current"><h3>L0 · AppCreator 社区版</h3><div class="price">开源免费</div>
<p>对话生成能力全开放，注册即用。${active ? '' : '（当前套餐）'}</p></div>
<div class="tier ${active ? 'current' : ''}"><h3>L1 · AppCreator 订阅</h3><div class="price">¥1,399/月</div>
<p>个人开发者与小团队的规模化引擎。订阅后按月生成账单，支持申请发票。</p>
${active
    ? `<form class="inline" method="post" action="/api/billing/cancel"><button class="btn ghost">取消订阅（期末生效）</button></form>`
    : `<form class="inline" method="post" action="/api/billing/subscribe"><button class="btn">订阅 L1</button></form>`}</div>
</div>
<p class="note">更高层级（L2 源码下载授权 ¥4,999 起 / L3 AliothStudio 私有化 ¥499,999）由原厂商务对接，详见首页。</p>`
}

function billsBody(data: ViewData): string {
  const { bills } = data
  const rows = bills.map(bill => `<tr>
<td>${bill.period}</td>
<td class="num">${yuan(bill.amountCents)}</td>
<td>${bill.status === 'paid'
    ? '<span class="pill ok">已支付</span>'
    : '<span class="pill warn">待支付</span>'}</td>
<td>${bill.paidAt === null ? '—' : bill.paidAt.toISOString().slice(0, 10)}</td>
<td>${bill.status === 'unpaid'
    ? `<form class="inline" method="post" action="/api/billing/pay"><input type="hidden" name="bill" value="${bill.id}"><button class="btn">线下确认支付</button></form>`
    : `<a href="/usercenter/invoices">申请发票</a>`}</td>
</tr>`).join('')
  return `<h1>账单</h1><p class="sub">订阅账单按月生成；当前为线下支付确认，在线支付渠道接入中。</p>
${tabs('/bills')}
${banners(data)}
<div class="panel">${bills.length === 0
    ? '<p class="note">暂无账单 — 订阅 L1 后按月生成。</p>'
    : `<table><thead><tr><th>账期</th><th class="num">金额</th><th>状态</th><th>支付日期</th><th>操作</th></tr></thead><tbody>${rows}</tbody></table>`}
<p class="note">账单数据为过渡内存态，正式计费后端接入后持久化。</p></div>`
}

function invoicesBody(data: ViewData): string {
  const { user, bills, invoices, pending } = data
  const paidBills = bills.filter(b => b.status === 'paid' && !invoices.some(i => i.billId === b.id))
  const billOptions = paidBills.map(b => `<option value="${b.id}">${b.period} · ${yuan(b.amountCents)}</option>`).join('')
  const rows = invoices.map(inv => `<tr>
<td>${esc(inv.title)}</td>
<td>${esc(inv.taxId === '' ? '—' : inv.taxId)}</td>
<td>${inv.status === 'issued' ? '<span class="pill ok">已开具</span>' : '<span class="pill warn">待开具</span>'}</td>
<td>${inv.requestedAt.toISOString().slice(0, 10)}</td>
</tr>`).join('')
  const adminRows = pending.map(inv => `<tr>
<td>${esc(inv.username ?? inv.userId.slice(0, 8))}</td>
<td>${esc(inv.title)}</td>
<td>${esc(inv.taxId === '' ? '—' : inv.taxId)}</td>
<td class="num">${yuan(inv.amountCents)}</td>
<td><form class="inline" method="post" action="/api/billing/issue"><input type="hidden" name="invoice" value="${inv.id}"><button class="btn">开具</button></form></td>
</tr>`).join('')
  return `<h1>发票</h1><p class="sub">已支付账单可申请开票（电子普票）；管理员在下方队列开具。</p>
${tabs('/invoices')}
${banners(data)}
<div class="panel"><h2>申请发票</h2>
${paidBills.length === 0
    ? '<p class="note">暂无可开票账单 — 需已支付且未申请过发票的账单。</p>'
    : `<form class="grid" method="post" action="/api/billing/invoice">
<label>账单<select name="bill" required>${billOptions}</select></label>
<label>发票抬头<input name="title" required placeholder="杭州宇器科技有限公司"></label>
<label>纳税人识别号<input name="tax" placeholder="91XXXXXXXXXXXXXXXXX"></label>
<button class="btn">提交申请</button>
</form>`}
</div>
<div class="panel"><h2>我的发票</h2>${invoices.length === 0
    ? '<p class="note">暂无发票记录。</p>'
    : `<table><thead><tr><th>抬头</th><th>税号</th><th>状态</th><th>申请日期</th></tr></thead><tbody>${rows}</tbody></table>`}
</div>
${user.role === 'admin'
    ? `<div class="panel"><h2>开具队列（管理员）</h2>${pending.length === 0
      ? '<p class="note">队列为空。</p>'
      : `<table><thead><tr><th>用户</th><th>抬头</th><th>税号</th><th class="num">金额</th><th>操作</th></tr></thead><tbody>${adminRows}</tbody></table>`}
<p class="note">开具动作登记开具时间；正式税控开票对接后自动回填票号。</p></div>`
    : ''}`
}

// ── plugin ───────────────────────────────────────────────────────────────

export function apply(ctx: Context, _config: Config): void {
  void _config

  /** Resolve the cookie/bearer user, or null. */
  const authedUser = async (request: IncomingMessage): Promise<AuthedUser | null> => {
    const user = await ctx.aliothAuth.userForToken(bearerToken(request) ?? cookieToken(request))
    return user === null ? null : { id: user.id, username: user.username, namespace: user.namespace, role: user.role }
  }

  const viewData = async (user: AuthedUser, notice = '', error = ''): Promise<ViewData> => ({
    user,
    subscription: await ctx.aliothBilling.getSubscription(user.id),
    bills: await ctx.aliothBilling.bills(user.id),
    invoices: await ctx.aliothBilling.invoices(user.id),
    pending: user.role === 'admin' ? await ctx.aliothBilling.pendingInvoices(user) : [],
    notice,
    error,
  })

  const pages: Record<string, { title: string; body: (data: ViewData) => string }> = {
    '/usercenter': { title: '概览', body: overviewBody },
    '/usercenter/subscription': { title: '订阅', body: subscriptionBody },
    '/usercenter/bills': { title: '账单', body: billsBody },
    '/usercenter/invoices': { title: '发票', body: invoicesBody },
  }

  /** Where each POST action redirects back to (form flow). */
  const backTo: Record<string, string> = {
    subscribe: '/usercenter/subscription', cancel: '/usercenter/subscription',
    pay: '/usercenter/bills', invoice: '/usercenter/invoices', issue: '/usercenter/invoices',
  }

  const handler = async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    const user = await authedUser(request)

    if (request.method === 'GET' && pages[url.pathname] !== undefined) {
      if (user === null) {
        response.writeHead(302, { location: '/login' })
        response.end()
        return
      }
      const view = pages[url.pathname]!
      page(response, 200, view.title, view.body(await viewData(user, url.searchParams.get('notice') ?? '', url.searchParams.get('error') ?? '')))
      return
    }

    if (request.method === 'GET' && url.pathname === '/api/billing/overview') {
      if (user === null) {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      const data = await viewData(user)
      sendJson(response, 200, {
        username: user.username, namespace: user.namespace, role: user.role,
        subscription: data.subscription, bills: data.bills, invoices: data.invoices,
      })
      return
    }

    const actionMatch = /^\/api\/billing\/(subscribe|cancel|pay|invoice|issue)$/.exec(url.pathname)
    if (request.method === 'POST' && actionMatch !== null) {
      // 同源加固：浏览器发出的状态变更必须携带匹配 Host 的 Origin
      // （SameSite=Lax 之上的第二道 CSRF 防线；无 Origin 的 JSON/curl
      // 客户端不受影响）。
      const originHeader = request.headers.origin
      const hostHeader = request.headers.host
      if (isFormPost(request) && originHeader !== undefined && hostHeader !== undefined
        && originHeader !== `http://${hostHeader}` && originHeader !== `https://${hostHeader}`) {
        sendJson(response, 403, { error: 'origin mismatch' })
        return
      }
      if (user === null) {
        sendJson(response, 401, { error: 'unauthorized' })
        return
      }
      const action = actionMatch[1]!
      const body = await readBody(request)
      const target = backTo[action] ?? '/usercenter'
      try {
        let result: unknown
        if (action === 'subscribe') {
          result = await ctx.aliothBilling.subscribe(user.id)
        } else if (action === 'cancel') {
          await ctx.aliothBilling.cancel(user.id)
          result = null
        } else if (action === 'pay') {
          const bill = typeof body.bill === 'string' ? body.bill : ''
          if (bill === '') throw new Error('缺少账单')
          result = await ctx.aliothBilling.payBill(bill, user)
        } else if (action === 'invoice') {
          const bill = typeof body.bill === 'string' ? body.bill : ''
          const title = typeof body.title === 'string' ? body.title : ''
          const tax = typeof body.tax === 'string' ? body.tax : ''
          if (bill === '') throw new Error('缺少账单')
          result = await ctx.aliothBilling.requestInvoice(bill, user, title, tax)
        } else {
          const invoice = typeof body.invoice === 'string' ? body.invoice : ''
          if (invoice === '') throw new Error('缺少发票申请')
          result = await ctx.aliothBilling.issueInvoice(invoice, user)
        }
        if (isFormPost(request)) {
          response.writeHead(302, { location: `${target}?notice=${encodeURIComponent('操作成功')}` })
          response.end()
        } else {
          sendJson(response, 200, result ?? { ok: true })
        }
      } catch (error) {
        const message = error instanceof Error ? error.message.replace(/^aliothBilling\.\w+: /, '') : String(error)
        if (isFormPost(request)) {
          response.writeHead(302, { location: `${target}?error=${encodeURIComponent(message)}` })
          response.end()
        } else {
          sendJson(response, 400, { error: message })
        }
      }
      return
    }

    sendJson(response, 404, { error: 'not found' })
  }

  const inject = ctx.inject as (deps: string[], cb: (webCtx: Context) => void) => void
  inject.call(ctx, ['webServer'], webCtx => {
    const web = asWebServer((webCtx.get as (name: string) => unknown).call(webCtx, 'webServer'))
    if (web === undefined) {
      ctx.logger.warn('billing-web-alioth: webServer present but shape mismatch — user center not mounted')
      return
    }
    webCtx.effect(() => web.register({
      kind: 'prefix',
      path: '/usercenter',
      handler: async (req, res) => { await handler(req, res) },
    }))
    webCtx.effect(() => web.register({
      kind: 'prefix',
      path: '/api/billing',
      handler: async (req, res) => { await handler(req, res) },
    }))
    ctx.logger.info('billing-web-alioth: user center mounted on webServer (/usercenter + /api/billing/*)')
  })
}
