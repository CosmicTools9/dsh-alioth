/**
 * 可选守卫服务面（T4 `@dsh-alioth/guard-alioth`，服务名 `aliothGuard`）的**结构化**读取。
 *
 * 本包不依赖 `guard-alioth`（守卫是可选装配）：只按服务面契约鸭子类型读取
 * `whitelistSource()` / `degradations()` / `usage()`。服务缺失 = 事实不可得 →
 * 调用方 MUST 置 `unknown(reason)`，MUST NOT 编造降级证据或用量。
 *
 * 上游对应面：`capabilities.rs`（六组广告）+ `add-appagent-degradation-evidence`
 * （白名单生效来源必须可见：`file` / `code_default` + 可区分原因）。
 * @module @dsh-alioth/tool-alioth-verify/guard-source
 */

import type { Context } from '@deepseek-ai/cordis'
import type { UsageSummary } from '@dsh-alioth/verify-alioth'

/** 门禁程序白名单的生效来源报告（镜像 `guard-alioth/src/whitelist.ts` 的公开形态）。 */
export interface GuardWhitelistSource {
  readonly source: 'file' | 'code_default'
  readonly reason: string
  readonly programs: readonly string[]
}

/** 一条显式降级证据（镜像 `guard-alioth/src/index.ts` 的 `Degradation`）。 */
interface GuardDegradation {
  readonly sessionId: string | null
  readonly ruleId: string
  readonly reason: string
  readonly time: number
}

/** `ctx.aliothGuard` 的消费面（只声明本包读到的方法）。 */
export interface GuardServiceLike {
  whitelistSource(): Promise<GuardWhitelistSource>
  degradations(): readonly GuardDegradation[]
  usage?(sessionId: string): UsageSummary
}

/** 读取可选的守卫服务；未装配 → `undefined`（调用方必须显式降级，不得编造）。 */
export function guardOf(ctx: Context): GuardServiceLike | undefined {
  const guard = ctx.get('aliothGuard') as GuardServiceLike | undefined
  return guard === undefined ? undefined : guard
}

/** 守卫未装配的显式原因（多处引用同一文案，避免口径漂移）。 */
export const GUARD_ABSENT_REASON =
  'aliothGuard 服务未装配（ctx.get("aliothGuard") 为空）：守卫降级证据与门禁白名单来源不可得，MUST NOT 编造'
