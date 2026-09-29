/**
 * `verify_runtime` 产物（运行期验证）—— 对齐上游 AppAgent `dialog_tools/verify_runtime.rs`
 * 与 `NS_APP_RUNTIME_HOSTING_SPEC.md` §9.3/§9.6/§9.7：把 agent 生成的应用交给**容器托管平面**
 * 启动并观测，落盘**canonical / degraded 互斥**的一对证据文件。
 *
 * 契约（MUST NOT 软化）：
 * - **canonical**（`runtime-verify.json`）只在**真实执行**且 `verdict ∈ ready|failed` 时写；
 *   `failed` 同样产生 canonical——故门解除 MUST 用内容谓词 `/verdict == "ready"`，
 *   **MUST NOT** 用「文件存在」判定（上游 `verify_runtime.rs` 同款理由）。
 * - **degraded**（`runtime-verify.degraded.json`）只在环境不可达等未真实执行时写，
 *   且 canonical **MUST NOT** 同时存在（同一时刻至多一个）。
 * - 证据脱敏：不落凭据值域（`token`/`password`/`secret`/`dsn`/`database_url`… 一律抹除），
 *   与上游 `trace::redact_json` + §9.3「证据 MUST NOT 含凭据值」同纪律。
 * - 本平面只判「容器可启动并就绪」，**不是 E2E**（§9.7）：MUST NOT 作为 `e2e_verify` 的替代。
 * - **全自动、无人工门**（2026-09-29 用户裁决）：环境不可达等失败 MUST 以**机器可执行的失败契约**
 *   （{@link runtimeFailure} 的 `artifact|environment|auth|capability` 分类 + 类与动作）交给自动重试墙
 *   与 agent 修复闭环消化；**MUST NOT** 登记「等人放行」的人工门，也 MUST NOT 把 degraded 当通过。
 *   `verdict === 'ready'` 的现状由 {@link runtimeVerificationSatisfied} 作**幂等判定**（已通过则不重跑）。
 * @module @dsh-alioth/verify-alioth/runtime-verify
 */

import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'

/** canonical 证据名（真实执行才产生）。 */
export const RUNTIME_VERIFY_NAME = 'runtime-verify.json'
/** 降级留痕名（环境不可达；canonical MUST NOT 同时存在）。 */
export const RUNTIME_VERIFY_DEGRADED_NAME = 'runtime-verify.degraded.json'
/** 文档 schema 版本。 */
export const RUNTIME_VERIFY_SCHEMA = 'alioth-runtime-verify/v1'

/**
 * 平面 verdict（§9.3）：`ready` / `failed` 属真实执行；`started` 是「已起未就绪」的中间态
 * （不写 canonical，不解除门）；`environment_unreachable` 属降级。
 */
export type RuntimeVerdict = 'ready' | 'failed' | 'started' | 'environment_unreachable'

/**
 * 失败分类（机器可执行，驱动自动处置）：
 * `artifact` = 产物缺陷（agent 可修）｜`environment` = 环境/引擎不可达（自动重试）｜
 * `auth` = 控制面 token 缺失或错误（配置面）｜`capability` = 容器服务未开启/非本机引擎（平台面）。
 */
export type RuntimeFailureKind = 'artifact' | 'environment' | 'auth' | 'capability'

/** 单条探针（§9.3：`via` 是观测点，容器内通过 ≠ 宿主可达）。 */
export interface RuntimeProbe {
  readonly url: string
  readonly via: 'exec' | 'host' | 'external'
  readonly status: number | null
  readonly duration_ms?: number
}

/** 收尾（`down`）结果——真实执行过就要交代是否清理干净。 */
export interface RuntimeCleanup {
  readonly ran: boolean
  readonly ok: boolean | null
  readonly detail: string
}

/** `runtime-verify.json` / `runtime-verify.degraded.json` 文档形态。 */
export interface RuntimeVerifyDoc {
  readonly schema: string
  readonly namespace: string
  readonly app: string
  readonly verdict: RuntimeVerdict
  readonly passed: boolean
  readonly degraded: boolean
  readonly container: string
  readonly engine: string
  readonly endpoint: string
  readonly remote: boolean | null
  readonly image: { readonly ref: string; readonly digest?: string } | null
  readonly ports: { readonly be?: number; readonly fe?: number; readonly sso?: number } | null
  readonly probes: readonly RuntimeProbe[]
  readonly failures: readonly string[]
  readonly cleanup: RuntimeCleanup
  /** 失败分类（成功时为 null；降级/失败时 MUST 给出，供自动处置）。 */
  readonly failure_kind: RuntimeFailureKind | null
  readonly logs_cmd: string
  readonly duration_ms: number | null
  readonly started_at: string
}

/** 判定结果：写哪个文件、能否解除降级门。 */
export interface RuntimeVerdictOutcome {
  readonly canonical: boolean
  readonly degraded: boolean
  readonly unlocksGate: boolean
}

/**
 * verdict → 产物落点与门解除判据（唯一实现，调用方 MUST NOT 自行复刻）。
 * @param verdict - 平面 verdict。
 * @returns 该写 canonical / degraded，以及是否满足门解除条件。
 */
export function runtimeVerdictOutcome(verdict: RuntimeVerdict): RuntimeVerdictOutcome {
  const canonical = verdict === 'ready' || verdict === 'failed'
  return {
    canonical,
    degraded: !canonical,
    unlocksGate: verdict === 'ready',
  }
}

/** 凭据值域的键名黑名单（大小写不敏感，逐层抹除）。 */
const REDACT_KEY_RE = /token|secret|password|passwd|credential|database_url|dsn|connection_string/i
/** 抹除后的占位（保留键名，便于判读「此处曾有值」）。 */
const REDACTED = '[redacted]'

/**
 * 逐层抹除凭据值域（§9.3 证据卫生；上游 `trace::redact_json` 同纪律）。
 * @param value - 任意 JSON 值。
 * @returns 抹除后的副本；数组原序保留。
 */
export function redactRuntimeEvidence(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(item => redactRuntimeEvidence(item))
  if (typeof value !== 'object' || value === null) return value
  const out: Record<string, unknown> = {}
  for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
    out[key] = REDACT_KEY_RE.test(key) ? REDACTED : redactRuntimeEvidence(item)
  }
  return out
}

/** `buildRuntimeVerify` 入参（`verdict`/`namespace`/`app` 必需，其余可缺）。 */
export interface RuntimeVerifyInput {
  readonly namespace: string
  readonly app: string
  readonly verdict: RuntimeVerdict
  readonly container?: string
  readonly engine?: string
  readonly endpoint?: string
  readonly remote?: boolean | null
  readonly image?: { readonly ref: string; readonly digest?: string } | null
  readonly ports?: { readonly be?: number; readonly fe?: number; readonly sso?: number } | null
  readonly probes?: readonly RuntimeProbe[]
  readonly failures?: readonly string[]
  readonly cleanup?: RuntimeCleanup
  readonly failure_kind?: RuntimeFailureKind | null
  readonly logs_cmd?: string
  readonly duration_ms?: number | null
  readonly started_at?: string
}

/** 未执行时的收尾口径（不伪称跑过）。 */
const CLEANUP_NOT_RUN: RuntimeCleanup = { ran: false, ok: null, detail: '未启动容器（环境不可达）' }

/**
 * 组装证据文档（纯函数：不落盘、不探测）。
 * `passed` 只对 `ready` 为真；`degraded` 只对未真实执行的 verdict 为真（degraded ≠ passed）。
 * @param input - 字段集合（见 {@link RuntimeVerifyInput}）。
 * @returns 文档（已脱敏）。
 */
export function buildRuntimeVerify(input: RuntimeVerifyInput): RuntimeVerifyDoc {
  const { canonical, degraded } = runtimeVerdictOutcome(input.verdict)
  const cleanup = input.cleanup ?? CLEANUP_NOT_RUN
  const doc: RuntimeVerifyDoc = {
    schema: RUNTIME_VERIFY_SCHEMA,
    namespace: input.namespace,
    app: input.app,
    verdict: input.verdict,
    passed: canonical && input.verdict === 'ready',
    degraded,
    container: input.container ?? '',
    engine: input.engine ?? '',
    endpoint: input.endpoint ?? '',
    remote: input.remote ?? null,
    image: input.image ?? null,
    ports: input.ports ?? null,
    probes: redactRuntimeEvidence(input.probes ?? []) as readonly RuntimeProbe[],
    failures: input.failures ?? [],
    cleanup: input.cleanup === undefined ? { ...CLEANUP_NOT_RUN } : redactRuntimeEvidence(cleanup) as RuntimeCleanup,
    // 缺省分类：通过 → null；产物缺陷(真实运行失败) → artifact；其余（不可达/未就绪） → environment。
    failure_kind: input.failure_kind
      ?? (input.verdict === 'ready' ? null : input.verdict === 'failed' ? 'artifact' : 'environment'),
    logs_cmd: input.logs_cmd === undefined ? '' : String(redactRuntimeEvidence(input.logs_cmd)),
    duration_ms: input.duration_ms ?? null,
    started_at: input.started_at ?? new Date().toISOString(),
  }
  return doc
}

/** 写盘结果：落地的文件路径（至多一个）与是否满足门解除条件。 */
export interface RuntimeVerifyWritten {
  readonly canonical?: string
  readonly degraded?: string
  readonly unlocksGate: boolean
}

/** 原子写（临时名 → rename），失败不留半截文件。 */
async function writeJsonAtomic(target: string, doc: unknown): Promise<void> {
  const tmp = `${target}.tmp-${process.pid}`
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`, 'utf8')
  await rename(tmp, target)
}

/**
 * 落盘证据：**canonical / degraded 互斥**——写一侧即删除另一侧（上游同款不变量）。
 * @param appDir - 应用产物目录（`Pre-Proc/{ns}/Apps/{app}`）。
 * @param doc - {@link buildRuntimeVerify} 的产物。
 * @returns 落地路径与门解除结论。
 */
export async function writeRuntimeVerify(appDir: string, doc: RuntimeVerifyDoc): Promise<RuntimeVerifyWritten> {
  const { canonical, degraded, unlocksGate } = runtimeVerdictOutcome(doc.verdict)
  await mkdir(appDir, { recursive: true })
  const canonicalPath = path.join(appDir, RUNTIME_VERIFY_NAME)
  const degradedPath = path.join(appDir, RUNTIME_VERIFY_DEGRADED_NAME)
  if (canonical) {
    await writeJsonAtomic(canonicalPath, doc)
    await rm(degradedPath, { force: true })
    return { canonical: canonicalPath, unlocksGate }
  }
  if (degraded && doc.verdict === 'environment_unreachable') {
    await writeJsonAtomic(degradedPath, doc)
    await rm(canonicalPath, { force: true })
    return { degraded: degradedPath, unlocksGate }
  }
  // `started` 之类的中间态：不落 canonical（未真实通过），保留既有降级留痕不动。
  return { unlocksGate }
}

/** 读回的现状（用于判读与门扫描）。 */
export interface RuntimeVerifyState {
  readonly canonical: RuntimeVerifyDoc | null
  readonly degraded: RuntimeVerifyDoc | null
}

/**
 * 读取现有证据（不可解析视同缺失——缺失 ≠ 通过）。
 * @param appDir - 应用产物目录。
 * @returns canonical / degraded 文档（各自可为 null）。
 */
export async function readRuntimeVerify(appDir: string): Promise<RuntimeVerifyState> {
  const read = async (name: string): Promise<RuntimeVerifyDoc | null> => {
    try {
      const text = await readFile(path.join(appDir, name), 'utf8')
      const parsed: unknown = JSON.parse(text)
      if (typeof parsed !== 'object' || parsed === null) return null
      return parsed as RuntimeVerifyDoc
    } catch {
      return null
    }
  }
  return { canonical: await read(RUNTIME_VERIFY_NAME), degraded: await read(RUNTIME_VERIFY_DEGRADED_NAME) }
}

/**
 * **幂等判定**（全自动链的复用点）：canonical 存在且 `/verdict === "ready"` ⇒ 已通过，不必重跑。
 * 这不是「门解除」——没有人工门；「文件存在」也**不算**通过（`failed` 同样产生 canonical）。
 * @param appDir - 应用产物目录。
 * @returns 是否已有一次真实的 ready 记录。
 */
export async function runtimeVerificationSatisfied(appDir: string): Promise<boolean> {
  const { canonical } = await readRuntimeVerify(appDir)
  return canonical?.verdict === 'ready'
}

/** 机器可执行的失败处置（类沿用仓库 `RepairClass` 三态；格式化归 skill-alioth 的 repair 契约）。 */
export interface RuntimeFailure {
  readonly ruleId: string
  readonly class: 'fixable' | 'retryable' | 'not-fixable'
  readonly action: string
  readonly detail: string
}

/**
 * 把一次运行期验证的结论翻成**自动处置契约**（不触发任何人工流程）。
 * @param doc - 证据文档（或其 verdict/failure_kind）。
 * @returns 失败契约；`ready` 返回 null（通过即无事可做）。
 */
export function runtimeFailure(doc: {
  readonly verdict: RuntimeVerdict
  readonly failure_kind?: RuntimeFailureKind | null
  readonly failures?: readonly string[]
  readonly container?: string
}): RuntimeFailure | null {
  if (doc.verdict === 'ready') return null
  const detail = (doc.failures ?? []).join('; ')
  switch (doc.failure_kind) {
    case 'artifact':
      return {
        ruleId: 'runtime-artifact-failed',
        class: 'fixable',
        action: '按 failures/probes 修产物（app.json/模块/扩展/服务骨架）后重跑验证',
        detail,
      }
    case 'auth':
      return {
        ruleId: 'runtime-control-unauthorized',
        class: 'not-fixable',
        action: '补齐/轮换控制面 token（ALIOTH_RUNTIME_CONTROL_TOKEN 或其文件载体，0600 且 ≥32 字符）后重跑',
        detail,
      }
    case 'capability':
      return {
        ruleId: 'runtime-container-service-unavailable',
        class: 'not-fixable',
        action: '平台侧开启容器服务（仅 Linux、本机引擎）或改指本机控制面后重跑',
        detail,
      }
    default:
      return {
        ruleId: 'runtime-environment-unreachable',
        class: 'retryable',
        action: '自动重试（重试墙按调用签名收敛）；持续不可达时核控制面端点/网络/TLS 后重跑',
        detail,
      }
  }
}
