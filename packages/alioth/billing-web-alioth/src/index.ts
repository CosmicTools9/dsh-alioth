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

import { createHmac, timingSafeEqual } from 'node:crypto'
import { ActionRateLimiter, clientKeyOf } from '@dsh-alioth/auth-web-alioth/throttle'
import type { IncomingMessage, ServerResponse } from 'node:http'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import type {
  AuditEntry, Bill, Invoice, LicenseView, Order, PendingInvoice, ReconciliationReport, Subscription,
} from '@dsh-alioth/billing-alioth'

export const name = 'billing-web-alioth'
export const inject = ['aliothBilling', 'aliothAuth']

export interface Config {
  /** Mainland-China ICP filing number rendered in the user-center footer
   * (env `ALIOTH_ICP` wins). Empty — the default — renders nothing. */
  readonly icp?: string
  /**
   * Shared secret for external channel settlement callbacks (ext-adapter
   * contract, env `ALIOTH_BILLING_CHANNEL_SECRET`): `POST /api/billing/channel/
   * <channel>/callback` must carry `x-alioth-signature: t=<unix>,v1=<hmac-
   * sha256(secret, "<t>.<body>")>` and a timestamp within 5 minutes. Empty —
   * the default — disables the endpoint entirely (fail-closed 503).
   */
  readonly channelSecret?: string
}

export const Config: z<Config> = z.object({
  icp: z.string().default(''),
  channelSecret: z.string().default(''),
})

/** 备案 footer for the user-center pages, or '' when nothing is filed. */
function icpFooter(icp: string | undefined): string {
  const value = (icp ?? '').trim()
  if (value === '') return ''
  const escaped = value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
  return `<footer class="icp"><a href="https://beian.miit.gov.cn/" target="_blank" rel="noopener noreferrer">${escaped}</a></footer>`
}

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

/** Raw body text (channel callbacks are HMAC'd over the exact bytes). */
function rawBody(request: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on('data', chunk => { chunks.push(Buffer.from(chunk)) })
    request.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    request.on('error', reject)
  })
}

/** `x-alioth-signature: t=<unix>,v1=<hex>` over `<t>.<rawBody>`, ±5 min window. */
function verifyChannelSignature(
  header: string | string[] | undefined,
  secret: string,
  raw: string,
): { ok: true } | { ok: false; reason: string } {
  if (typeof header !== 'string' || header === '') {
    return { ok: false, reason: 'missing x-alioth-signature' }
  }
  const match = /^t=(\d+),v1=([0-9a-f]{64})$/.exec(header.trim())
  if (match === null) {
    return { ok: false, reason: 'malformed x-alioth-signature (expected t=<unix>,v1=<hex>)' }
  }
  const timestamp = Number(match[1])
  if (Math.abs(Date.now() / 1000 - timestamp) > 300) {
    return { ok: false, reason: 'signature timestamp outside the 5-minute window' }
  }
  const expected = createHmac('sha256', secret).update(`${match[1]}.${raw}`, 'utf8').digest()
  const provided = Buffer.from(match[2]!, 'hex')
  if (provided.length !== expected.length || !timingSafeEqual(provided, expected)) {
    return { ok: false, reason: 'signature mismatch' }
  }
  return { ok: true }
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

function page(response: ServerResponse, status: number, title: string, body: string, icp = ''): void {
  response.writeHead(status, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
  response.end(`<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" type="image/svg+xml" href="/favicon.svg">
<link rel="icon" href="/favicon.ico" sizes="16x16 32x32">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<meta name="theme-color" content="#0a0e14">
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
footer.icp{text-align:center;font-size:.8rem;color:var(--dim);padding:0 1.5rem 2.5rem}
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
${icpFooter(icp)}
</body></html>`)
}

// ── page bodies ──────────────────────────────────────────────────────────

interface ViewData {
  user: AuthedUser
  subscription: Subscription | null
  /** L2 source-download authorization row (null = never asked). */
  source: LicenseView | null
  bills: Bill[]
  invoices: Invoice[]
  pending: PendingInvoice[]
  /** Admin-only: every order (null off the admin page / for plain users). */
  orders: Order[] | null
  /** Admin-only: the cash/order reconciliation sweep. */
  report: ReconciliationReport | null
  /** Admin-only: audit trail tail (newest first). */
  audit: AuditEntry[] | null
  notice: string
  error: string
}

function tabs(active: string, role: 'admin' | 'user' = 'user'): string {
  const items = [['', '概览'], ['/subscription', '订阅'], ['/bills', '账单'], ['/invoices', '发票']]
  if (role === 'admin') items.push(['/admin', '管理'])
  return `<div class="tabs">${items.map(([href, label]) =>
    `<a class="${href === active ? 'active' : ''}" href="/usercenter${href}">${label}</a>`).join('')}</div>`
}

function banners(data: ViewData): string {
  return `${data.notice === '' ? '' : `<p class="banner ok">${esc(data.notice)}</p>`}${data.error === '' ? '' : `<p class="banner error">${esc(data.error)}</p>`}`
}

/** Subscription status text for the two views that show it. */
function statusText(subscription: Subscription | null): string {
  if (subscription === null) return '—'
  if (subscription.status === 'active') return '生效中'
  if (subscription.status === 'canceling') {
    return `取消中（${subscription.renewsAt.toISOString().slice(0, 10)} 期末生效）`
  }
  return '已取消'
}

function overviewBody(data: ViewData): string {
  const { user, subscription, bills, invoices } = data
  const unpaid = bills.filter(b => b.status === 'unpaid').length
  const pendingInv = invoices.filter(i => i.status === 'pending').length
  return `<h1>用户中心</h1><p class="sub">${esc(user.username)} · ${esc(user.namespace)}${user.role === 'admin' ? ' · 管理员' : ''}</p>
${tabs('', user.role)}
${banners(data)}
<div class="panel"><h2>账户</h2><dl class="kv">
<dt>用户名</dt><dd>${esc(user.username)}</dd>
<dt>命名空间</dt><dd>${esc(user.namespace)}</dd>
<dt>角色</dt><dd>${user.role === 'admin' ? 'admin（管理员）' : 'user'}</dd>
</dl></div>
<div class="panel"><h2>订阅</h2><dl class="kv">
<dt>当前套餐</dt><dd>${subscription?.status === 'active' || subscription?.status === 'canceling' ? 'L1 订阅版' : 'L0 社区版（免费）'}</dd>
<dt>状态</dt><dd>${statusText(subscription)}</dd>
<dt>下次续期</dt><dd>${subscription?.status !== 'canceled' && subscription !== null ? subscription.renewsAt.toISOString().slice(0, 10) : '—'}</dd>
</dl><p class="note"><a href="/usercenter/subscription">管理订阅 →</a></p></div>
<div class="panel"><h2>账单与发票</h2><dl class="kv">
<dt>待支付账单</dt><dd>${unpaid}</dd>
<dt>待开具发票</dt><dd>${pendingInv}</dd>
</dl><p class="note"><a href="/usercenter/bills">查看账单 →</a> · <a href="/usercenter/invoices">申请发票 →</a></p></div>`
}

function subscriptionBody(data: ViewData): string {
  const { subscription } = data
  const active = subscription?.status === 'active' || subscription?.status === 'canceling'
  const statusLine = subscription?.status === 'canceling'
    ? `<p class="note">已申请取消，${subscription.renewsAt.toISOString().slice(0, 10)} 期末生效；在此之前仍可继续使用。</p>
<form class="inline" method="post" action="/api/billing/subscribe"><button class="btn ghost">撤销取消（继续订阅）</button></form>`
    : ''
  return `<h1>订阅</h1><p class="sub">从社区版到私有化的阶梯 — 当前：${active ? 'L1 订阅版' : 'L0 社区版'}</p>
${tabs('/subscription', data.user.role)}
${banners(data)}
<div class="tiers">
<div class="tier current"><h3>L0 · AppCreator 社区版</h3><div class="price">开源免费</div>
<p>对话生成能力全开放，注册即用。${active ? '' : '（当前套餐）'}</p></div>
<div class="tier ${active ? 'current' : ''}"><h3>L1 · AppCreator 订阅</h3><div class="price">¥1,399/月</div>
<p>个人开发者与小团队的规模化引擎。订阅后按月生成账单，支持申请发票。</p>
${active
    ? `<form class="inline" method="post" action="/api/billing/cancel"><button class="btn ghost">取消订阅（期末生效）</button></form>`
    : `<form class="inline" method="post" action="/api/billing/subscribe"><button class="btn">订阅 L1</button></form>`}
${statusLine}</div>
</div>
${sourceTier(data.source)}
<p class="note">更高层级（L3 AliothStudio 私有化 ¥499,999）由原厂商务对接，详见首页。</p>`
}

/** Day precision is all the operator's `until` promises; render it that way. */
function untilOf(date: Date): string {
  return date.toISOString().slice(0, 10)
}

/**
 * The L2 source tier. Source is 商务对接 (no self-serve path), so this card offers
 * exactly one action — 申请 — and then reports what the operator recorded.
 */
function sourceTier(source: LicenseView | null): string {
  const granted = source !== null && source.grantedAt !== null && source.until !== null
  const expired = granted && source.until !== null && source.until.getTime() < Date.now()
  const request = `<form class="inline" method="post" action="/api/billing/source"><button class="btn">${granted ? '重新申请 L2 授权' : '申请 L2 授权'}</button></form>`
  const state = source === null
    ? `<p class="note">尚未申请。提交后由商务对接确认并开通。</p>${request}`
    : granted && !expired
      ? `<p class="note">已开通至 ${untilOf(source.until as Date)}。可在「原型」页下载源码。${source.note === '' ? '' : `（${esc(source.note)}）`}</p>`
      : granted
        ? `<p class="note">授权已于 ${untilOf(source.until as Date)} 到期。</p>${request}`
        : `<p class="note">已提交申请（${untilOf(source.requestedAt)}），等待商务对接开通。</p>`
  return `<div class="tier ${granted && !expired ? 'current' : ''}"><h3>L2 · 源码下载授权</h3><div class="price">¥4,999 起</div>
<p>下载应用的完整源码包（app.json、modules/、extensions/、Sources/），由原厂商务对接开通。</p>
${state}</div>`
}

function billsBody(data: ViewData): string {
  const { bills } = data
  const pill = (status: Bill['status']): string =>
    status === 'paid'
      ? '<span class="pill ok">已支付</span>'
      : status === 'void'
        ? '<span class="pill">已作废</span>'
        : status === 'refunded'
          ? '<span class="pill">已退款</span>'
          : '<span class="pill warn">待支付</span>'
  const rows = bills.map(bill => `<tr>
<td>${bill.period}</td>
<td class="num">${yuan(bill.amountCents)}</td>
<td>${pill(bill.status)}</td>
<td>${bill.paidAt === null ? '—' : bill.paidAt.toISOString().slice(0, 10)}</td>
<td>${bill.status === 'unpaid'
    ? `<form class="inline" method="post" action="/api/billing/pay"><input type="hidden" name="bill" value="${bill.id}"><button class="btn">线下确认支付</button></form>`
    : `<a href="/usercenter/invoices">申请发票</a>`}</td>
</tr>`).join('')
  return `<h1>账单</h1><p class="sub">订阅账单按月生成；当前为线下支付确认，在线支付渠道接入中。</p>
${tabs('/bills', data.user.role)}
${banners(data)}
<div class="panel">${bills.length === 0
    ? '<p class="note">暂无账单 — 订阅 L1 后按月生成。</p>'
    : `<table><thead><tr><th>账期</th><th class="num">金额</th><th>状态</th><th>支付日期</th><th>操作</th></tr></thead><tbody>${rows}</tbody></table>`}
<p class="note">账单、订单与用量数据持久化在部署数据库（dsh_alioth_billing）中。</p></div>`
}

function invoicesBody(data: ViewData): string {
  const { user, bills, invoices, pending } = data
  const paidBills = bills.filter(b => b.status === 'paid' && !invoices.some(i => i.billId === b.id))
  const billOptions = paidBills.map(b => `<option value="${b.id}">${b.period} · ${yuan(b.amountCents)}</option>`).join('')
  const rows = invoices.map(inv => `<tr>
<td>${esc(inv.title)}</td>
<td>${esc(inv.taxId === '' ? '—' : inv.taxId)}</td>
<td>${esc(inv.number ?? '—')}</td>
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
${tabs('/invoices', user.role)}
${banners(data)}
<div class="panel"><h2>申请发票</h2><p class="note">提交后进入管理员开具队列，开具后此处显示票号。</p>
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
    : `<table><thead><tr><th>抬头</th><th>税号</th><th>票号</th><th>状态</th><th>申请日期</th></tr></thead><tbody>${rows}</tbody></table>`}
</div>
${user.role === 'admin'
    ? `<div class="panel"><h2>开具队列（管理员）</h2>${pending.length === 0
      ? '<p class="note">队列为空。</p>'
      : `<table><thead><tr><th>用户</th><th>抬头</th><th>税号</th><th class="num">金额</th><th>操作</th></tr></thead><tbody>${adminRows}</tbody></table>`}
<p class="note">开具动作登记开具时间；正式税控开票对接后自动回填票号。</p></div>`
    : ''}`
}

/**
 * The admin back-office: order/audit/reconciliation views plus the two
 * operator actions the ladder needs (grant an L2 window; refund/void a bill).
 * Everything here is admin-only — the service enforces it again server-side.
 */
function adminBody(data: ViewData): string {
  const { report, orders, audit } = data
  const reportText = report === null ? '' : `<dl class="kv">
<dt>账单</dt><dd>${report.bills.total}（未付 ${report.bills.unpaid} · 已付 ${report.bills.paid} · 作废 ${report.bills.void} · 退款 ${report.bills.refunded}）</dd>
<dt>订单</dt><dd>${report.orders.total}（待付 ${report.orders.pending} · 已履约 ${report.orders.fulfilled} · 已取消 ${report.orders.cancelled} · 已退款 ${report.orders.refunded}）</dd>
</dl>${report.anomalies.length === 0
    ? '<p class="note">对账无异常。</p>'
    : `<table><thead><tr><th>异常类型</th><th>详情</th></tr></thead><tbody>${report.anomalies.map(anomaly =>
      `<tr><td><span class="pill warn">${esc(anomaly.kind)}</span></td><td>${esc(anomaly.detail)}</td></tr>`).join('')}</tbody></table>`}`
  const orderRows = (orders ?? []).slice(0, 50).map(order => `<tr>
<td class="num">${order.id.slice(0, 8)}</td>
<td class="num">${yuan(order.amountCents)}</td>
<td>${order.status === 'fulfilled' ? '<span class="pill ok">已履约</span>'
    : order.status === 'pending' ? '<span class="pill warn">待支付</span>'
    : order.status === 'refunded' ? '<span class="pill">已退款</span>'
    : order.status === 'cancelled' ? '<span class="pill">已取消</span>' : '<span class="pill ok">已支付</span>'}</td>
<td>${order.createdAt.toISOString().slice(0, 10)}</td>
<td>${order.status === 'paid' && order.billId !== null
      ? `<form class="inline" method="post" action="/api/billing/admin/refund"><input type="hidden" name="bill" value="${order.billId}"><button class="btn ghost">退款</button></form>`
      : order.status === 'pending' && order.billId !== null
        ? `<form class="inline" method="post" action="/api/billing/admin/void"><input type="hidden" name="bill" value="${order.billId}"><button class="btn ghost">作废</button></form>`
        : ''}</td>
</tr>`).join('')
  const auditRows = (audit ?? []).slice(0, 50).map(entry => `<tr>
<td>${entry.ts.toISOString().replace('T', ' ').slice(0, 19)}</td>
<td>${esc(entry.actor)}</td>
<td>${esc(entry.event)}</td>
<td>${esc(entry.target)}</td>
</tr>`).join('')
  return `<h1>管理</h1><p class="sub">运营面 — 对账 / 订单 / 审计 / L2 授权开通（仅管理员可见）</p>
${tabs('/admin', data.user.role)}
${banners(data)}
<div class="panel"><h2>对账</h2>${reportText}</div>
<div class="panel"><h2>订单</h2>${(orders ?? []).length === 0
    ? '<p class="note">暂无订单。</p>'
    : `<table><thead><tr><th>订单</th><th class="num">金额</th><th>状态</th><th>创建</th><th>操作</th></tr></thead><tbody>${orderRows}</tbody></table>`}
</div>
<div class="panel"><h2>L2 源码授权开通</h2>
<form class="grid" method="post" action="/api/billing/admin/grant-source">
<label>用户名<input name="username" required placeholder="alice"></label>
<label>开通至（YYYY-MM-DD）<input name="until" required placeholder="2027-06-30"></label>
<label>备注<input name="note" placeholder="合同 2026-114"></label>
<button class="btn">开通 / 续期</button>
</form><p class="note">与 <code>pnpm run source:grant</code> 等效；操作写入审计。</p></div>
<div class="panel"><h2>审计（最近 50 条）</h2>${(audit ?? []).length === 0
    ? '<p class="note">暂无审计记录。</p>'
    : `<table><thead><tr><th>时间</th><th>操作者</th><th>事件</th><th>对象</th></tr></thead><tbody>${auditRows}</tbody></table>`}
</div>`
}

// ── plugin ───────────────────────────────────────────────────────────────

export function apply(ctx: Context, config: Config): void {
  /** 备案 number carried by every user-center page footer. Config-only: the
   * bundle patch decides whether this carrier is one of the showing surfaces. */
  const icp = config.icp
  /** Ext-adapter 回调验签密钥（空 = 回调端点整体关闭，fail-closed）。 */
  const channelSecret = config.channelSecret ?? ''
  /** 已认证动作限流：状态变更端点按 (来源 IP, 账户) 固定窗口计数。 */
  const actions = new ActionRateLimiter()

  /** Resolve the cookie/bearer user, or null. */
  const authedUser = async (request: IncomingMessage): Promise<AuthedUser | null> => {
    const user = await ctx.aliothAuth.userForToken(bearerToken(request) ?? cookieToken(request))
    return user === null ? null : { id: user.id, username: user.username, namespace: user.namespace, role: user.role }
  }

  const viewData = async (user: AuthedUser, notice = '', error = '', admin = false): Promise<ViewData> => ({
    user,
    subscription: await ctx.aliothBilling.getSubscription(user.id),
    bills: await ctx.aliothBilling.bills(user.id),
    invoices: await ctx.aliothBilling.invoices(user.id),
    pending: user.role === 'admin' ? await ctx.aliothBilling.pendingInvoices(user) : [],
    source: await ctx.aliothBilling.sourceLicenseRequest(user.id),
    orders: admin && user.role === 'admin' ? await ctx.aliothBilling.allOrders(user) : null,
    report: admin && user.role === 'admin' ? await ctx.aliothBilling.reconcile(user) : null,
    audit: admin && user.role === 'admin' ? await ctx.aliothBilling.auditTail(user, 50) : null,
    notice,
    error,
  })

  const pages: Record<string, { title: string; body: (data: ViewData) => string }> = {
    '/usercenter': { title: '概览', body: overviewBody },
    '/usercenter/subscription': { title: '订阅', body: subscriptionBody },
    '/usercenter/bills': { title: '账单', body: billsBody },
    '/usercenter/invoices': { title: '发票', body: invoicesBody },
    '/usercenter/admin': { title: '管理', body: adminBody },
  }

  /** Where each POST action redirects back to (form flow). */
  const backTo: Record<string, string> = {
    subscribe: '/usercenter/subscription', cancel: '/usercenter/subscription',
    pay: '/usercenter/bills', invoice: '/usercenter/invoices', issue: '/usercenter/invoices',
    source: '/usercenter/subscription',
    'grant-source': '/usercenter/admin', refund: '/usercenter/admin', void: '/usercenter/admin',
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
      if (url.pathname === '/usercenter/admin' && user.role !== 'admin') {
        response.writeHead(302, { location: '/usercenter?error=' + encodeURIComponent('仅管理员可访问') })
        response.end()
        return
      }
      const view = pages[url.pathname]!
      page(response, 200, view.title, view.body(await viewData(user, url.searchParams.get('notice') ?? '', url.searchParams.get('error') ?? '', url.pathname === '/usercenter/admin')), icp)
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
        subscription: data.subscription, source: data.source,
        bills: data.bills, invoices: data.invoices,
      })
      return
    }

    const actionMatch = /^\/api\/billing\/(subscribe|cancel|pay|invoice|issue|source)$/.exec(url.pathname)
      ?? /^\/api\/billing\/admin\/(grant-source|refund|void)$/.exec(url.pathname)
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
      // 动作限流（每次计数，不分成败）：挡脚本刷接口，不挡正常操作。
      if (!actions.admit(`${clientKeyOf(request)}\u0000${user.id}`)) {
        sendJson(response, 429, { error: '操作过于频繁，请稍后再试' })
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
        } else if (action === 'source') {
          // 申请 L2：只落一条请求行，开通由商务对接在库上确认（无自助通道）。
          result = await ctx.aliothBilling.requestSourceLicense(user.id)
        } else if (action === 'grant-source') {
          if (user.role !== 'admin') throw new Error('仅管理员可开通授权')
          const username = typeof body.username === 'string' ? body.username.trim() : ''
          const until = typeof body.until === 'string' ? body.until.trim() : ''
          const note = typeof body.note === 'string' ? body.note : ''
          if (username === '' || !/^\d{4}-\d{2}-\d{2}$/.test(until)) {
            throw new Error('需要 username 与 YYYY-MM-DD 的 until')
          }
          const target0 = await ctx.aliothAuth.userByUsername(username)
          if (target0 === null) throw new Error(`用户不存在：${username}`)
          // Date-only means the whole day (same rule as the operator's env list).
          result = await ctx.aliothBilling.grantSourceLicense(
            target0.id, new Date(`${until}T23:59:59.999Z`), note,
          )
        } else if (action === 'refund') {
          const bill = typeof body.bill === 'string' ? body.bill : ''
          if (bill === '') throw new Error('缺少账单')
          result = await ctx.aliothBilling.refundBill(bill, user)
        } else if (action === 'void') {
          const bill = typeof body.bill === 'string' ? body.bill : ''
          if (bill === '') throw new Error('缺少账单')
          result = await ctx.aliothBilling.voidBill(bill, user)
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

    // ── C4 外部渠道结算回调（ext-adapter 契约） ────────────────────────────
    // NS:Cosmic-Tools 的 Ext-adapter（如 wechat-pay-adapter）在自身 HTTP 边缘
    // 完成与 PSP 的验签/对账后，把已结算的账单引用 POST 到这里。共享密钥
    // HMAC-SHA256(`t.raw`) + 5 分钟时间戳窗口；未配置密钥 → 整体关闭。
    const channelMatch = /^\/api\/billing\/channel\/([a-z0-9-]+)\/callback$/.exec(url.pathname)
    if (request.method === 'POST' && channelMatch !== null) {
      if (channelSecret === '') {
        sendJson(response, 503, { error: 'channel callbacks disabled (ALIOTH_BILLING_CHANNEL_SECRET unset)' })
        return
      }
      const raw = await rawBody(request)
      const check = verifyChannelSignature(request.headers['x-alioth-signature'], channelSecret, raw)
      if (!check.ok) {
        sendJson(response, 401, { error: check.reason })
        return
      }
      let payload: Record<string, unknown>
      try {
        payload = JSON.parse(raw) as Record<string, unknown>
      } catch {
        sendJson(response, 400, { error: 'invalid json body' })
        return
      }
      const billReference = typeof payload.billId === 'string' ? payload.billId : ''
      const amountCents = typeof payload.amountCents === 'number' ? payload.amountCents : Number.NaN
      const note = typeof payload.note === 'string' ? payload.note : `channel ${channelMatch[1]}`
      if (billReference === '' || !Number.isFinite(amountCents)) {
        sendJson(response, 400, { error: '需要 billId 与 amountCents' })
        return
      }
      try {
        const bill = await ctx.aliothBilling.applyChannelPayment(channelMatch[1]!, billReference, amountCents, note)
        sendJson(response, 200, { ok: true, billId: bill.id, status: bill.status })
      } catch (error) {
        sendJson(response, 409, { error: error instanceof Error ? error.message : String(error) })
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
