/**
 * 登录/注册限流：按 `(来源 IP, 用户名)` 的滑动窗口失败计数（内存态、有界）。
 * 在线 B/S 的最低限速缝——不是 WAF，但能把暴力猜口令从「每秒无限次」压到
 * 「窗口内至多 maxFailures 次尝试」。键内含用户名，所以攻击者封不到别人。
 * @module @dsh-alioth/auth-web-alioth/throttle
 */

export interface LoginThrottleOptions {
  /** 失败窗口（毫秒）；默认 5 分钟。 */
  readonly windowMs?: number
  /** 窗口内允许的失败次数上限；默认 10。 */
  readonly maxFailures?: number
  /** 记录上限（有界，防内存无界）；默认 10_000 个键。 */
  readonly maxKeys?: number
}

export class LoginThrottle {
  private readonly failures = new Map<string, readonly number[]>()
  private readonly windowMs: number
  private readonly maxFailures: number
  private readonly maxKeys: number

  constructor(options: LoginThrottleOptions = {}) {
    this.windowMs = options.windowMs ?? 5 * 60 * 1000
    this.maxFailures = options.maxFailures ?? 10
    this.maxKeys = options.maxKeys ?? 10_000
  }

  /** 该键当前是否被限流（窗口内失败计数已满）。 */
  blocked(key: string): boolean {
    const now = Date.now()
    const recent = (this.failures.get(key) ?? []).filter(time => now - time < this.windowMs)
    return recent.length >= this.maxFailures
  }

  /** 记一次失败；@returns 记录后的窗口内失败次数。 */
  failure(key: string): number {
    const now = Date.now()
    const recent = (this.failures.get(key) ?? []).filter(time => now - time < this.windowMs)
    recent.push(now)
    if (this.failures.size >= this.maxKeys && !this.failures.has(key)) {
      // 有界：丢最旧的一键（Map 保持插入序）。
      const oldest = this.failures.keys().next().value
      if (oldest !== undefined) this.failures.delete(oldest)
    }
    this.failures.set(key, recent)
    return recent.length
  }

  /** 成功登录后清零该键（正常用户输错一两次不该被记账惩罚太久）。 */
  reset(key: string): void {
    this.failures.delete(key)
  }

  /** 窗口内剩余可尝试次数（限流文案用）。 */
  remaining(key: string): number {
    const now = Date.now()
    const recent = (this.failures.get(key) ?? []).filter(time => now - time < this.windowMs)
    return Math.max(0, this.maxFailures - recent.length)
  }
}

/** 请求来源 IP 的尽力解析（反代取 x-forwarded-for 首跳；无 → 'unknown'）。 */
export function clientKeyOf(request: { headers: Record<string, unknown>; socket?: { remoteAddress?: string | undefined } }): string {
  const forwarded = request.headers['x-forwarded-for']
  if (typeof forwarded === 'string' && forwarded.trim() !== '') {
    return forwarded.split(',')[0]!.trim()
  }
  return request.socket?.remoteAddress ?? 'unknown'
}
