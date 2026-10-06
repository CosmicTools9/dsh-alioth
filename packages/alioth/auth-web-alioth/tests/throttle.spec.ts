/**
 * 登录限流（throttle）单元语义：窗口计数、封禁、清零、有界表。
 */
import { describe, expect, it } from 'vitest'
import { LoginThrottle, clientKeyOf } from '../src/throttle.ts'

describe('LoginThrottle', () => {
  it('allows up to maxFailures, then blocks within the window', () => {
    const throttle = new LoginThrottle({ maxFailures: 3, windowMs: 60_000 })
    const key = 'ip1\u0000ada'
    expect(throttle.blocked(key)).toBe(false)
    expect(throttle.failure(key)).toBe(1)
    expect(throttle.failure(key)).toBe(2)
    expect(throttle.blocked(key)).toBe(false)
    expect(throttle.failure(key)).toBe(3)
    expect(throttle.blocked(key)).toBe(true)
    // 其余键不受牵连（键内含用户名，封不到别人）。
    expect(throttle.blocked('ip1\u0000eve')).toBe(false)
    expect(throttle.remaining(key)).toBe(0)
  })

  it('forgets failures outside the window', async () => {
    const throttle = new LoginThrottle({ maxFailures: 1, windowMs: 50 })
    const key = 'ip2\u0000ada'
    throttle.failure(key)
    expect(throttle.blocked(key)).toBe(true)
    await new Promise<void>(resolve => setTimeout(resolve, 60))
    expect(throttle.blocked(key)).toBe(false)
  })

  it('reset clears the count on a successful login', () => {
    const throttle = new LoginThrottle({ maxFailures: 2, windowMs: 60_000 })
    const key = 'ip3\u0000ada'
    throttle.failure(key)
    throttle.reset(key)
    expect(throttle.blocked(key)).toBe(false)
    expect(throttle.failure(key)).toBe(1)
  })

  it('keeps the failure table bounded', () => {
    const throttle = new LoginThrottle({ maxKeys: 10 })
    for (let index = 0; index < 50; index += 1) {
      throttle.failure(`ip${index}\u0000ada`)
    }
    // 内部表有界（10 键）：最早的键被逐出，最新的还在计数。
    expect(throttle.remaining('ip0\u0000ada')).toBe(10)
    expect(throttle.remaining('ip49\u0000ada')).toBe(9)
  })
})

describe('clientKeyOf', () => {
  it('takes the first x-forwarded-for hop and falls back to the socket', () => {
    expect(clientKeyOf({ headers: { 'x-forwarded-for': '203.0.113.7, 10.0.0.1' } })).toBe('203.0.113.7')
    expect(clientKeyOf({ headers: {}, socket: { remoteAddress: '127.0.0.1' } })).toBe('127.0.0.1')
    expect(clientKeyOf({ headers: {} })).toBe('unknown')
  })
})
