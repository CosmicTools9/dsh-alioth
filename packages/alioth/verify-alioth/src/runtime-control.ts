/**
 * 容器托管平面的**控制面客户端**——对齐 `NS_APP_RUNTIME_HOSTING_SPEC.md` §11
 * （控制平面唯一宿主 = Meta 后端）与 §9.6（仓外 harness 只依赖可执行脚本 + 引擎端点）。
 *
 * 合同要点（MUST）：
 * - 机器面动词：`GET /containers`、`POST /containers {ns,label?,tier?,pkg?,image?}`、
 *   `GET /containers/{key}`、`GET /containers/{key}/logs?tail=N`、`DELETE /containers/{key}`、
 *   `GET /api/meta/runtime/routes`；认证 = `Authorization: Bearer <token>`。
 * - 状态码：`200 → ok`、`401 → unauthorized`（token 缺/错）、`409 → capability`
 *   （`container_service_unavailable` 能力快照 / `container_service_remote_readonly`）、
 *   `422 → unprocessable`、`503 → unavailable`、`504 → timeout`。
 * - token 载体解析与 Meta 侧同序（唯一语义）：显式配置 ▶ `ALIOTH_RUNTIME_CONTROL_TOKEN`
 *   （**显式置空 = 关闭文件载体**）▶ `ALIOTH_RUNTIME_CONTROL_TOKEN_FILE` ▶ 默认
 *   `$HOME/.alioth/runtime-control.token`；文件 MUST `0600` 且 token ≥32 字符，否则 fail-closed。
 * - 本模块只做**消费与动词转发**，MUST NOT 复制平面的容器逻辑/探针实现（§11 执行面纪律）。
 * @module @dsh-alioth/verify-alioth/runtime-control
 */

import { readFile, stat } from 'node:fs/promises'
import http from 'node:http'
import https from 'node:https'
import { homedir } from 'node:os'
import path from 'node:path'

/** 控制面端点配置。`url` = 控制面入口（如 `https://192.168.10.251:2048`）。 */
export interface RuntimeControlConfig {
  readonly url: string
  /** `Host` 头覆写（nginx 按 `server_name` 分流时必须给对；缺省用 URL 的 host）。 */
  readonly host?: string
  /** 显式 token（优先于环境与文件载体）。 */
  readonly token?: string
  /** 显式 token 文件（优先于默认文件；仍受 `0600`/长度约束）。 */
  readonly tokenFile?: string
  /** 关掉 TLS 校验（自签/名称不匹配时的**显式**选择；缺省 false）。 */
  readonly insecureTls?: boolean
  /** 单次请求超时（毫秒，缺省 30000）。 */
  readonly timeoutMs?: number
}

/** 合同状态码归一（§11）。 */
export type ControlOutcome =
  | 'ok'
  | 'unauthorized'
  | 'capability'
  | 'unprocessable'
  | 'unavailable'
  | 'timeout'
  | 'error'

/** 一次控制面调用结果（不抛异常；调用方按 outcome 决策）。 */
export interface ControlResponse {
  readonly outcome: ControlOutcome
  readonly status: number | null
  readonly body: unknown
  readonly text: string
  readonly detail: string
}

/** token 解析结果（**MUST NOT** 记录 token 值本身，只记来源）。 */
export interface TokenResolution {
  readonly token: string
  readonly source: 'config' | 'env' | 'env-file' | 'default-file'
}

/** token 最短长度（与 Meta 侧一致）。 */
export const RUNTIME_TOKEN_MIN_LENGTH = 32

/** 默认 token 文件（相对家目录）。 */
const DEFAULT_TOKEN_FILE = path.join('.alioth', 'runtime-control.token')

/** 单次 HTTP 请求的最小依赖面（可注入，供测试断言请求构造）。 */
export interface RuntimeControlRequest {
  readonly method: string
  readonly url: URL
  readonly hostHeader: string
  readonly headers: Readonly<Record<string, string>>
  readonly body?: string
  readonly timeoutMs: number
  readonly insecureTls: boolean
}

/**
 * 读取并校验 token 载体。
 * @param env - 环境变量集合（缺省 `process.env`）。
 * @param options - `token` / `tokenFile` 显式覆盖。
 * @param readText - 读文件实现（测试注入）。
 * @param modeOf - 取文件权限位实现（测试注入）。
 * @returns token 与其来源。
 */
export async function resolveRuntimeControlToken(
  env: Record<string, string | undefined> = process.env,
  options: { readonly token?: string; readonly tokenFile?: string } = {},
  readText: (file: string) => Promise<string> = file => readFile(file, 'utf8'),
  modeOf: (file: string) => Promise<number> = async (file) => {
    const info = await stat(file)
    return info.mode & 0o777
  },
): Promise<TokenResolution> {
  const explicit = options.token
  if (explicit !== undefined) {
    if (explicit.length < RUNTIME_TOKEN_MIN_LENGTH) {
      throw new Error(`runtime control token 过短（<${RUNTIME_TOKEN_MIN_LENGTH} 字符）：配置的 token 被拒绝`)
    }
    return { token: explicit, source: 'config' }
  }
  const fromEnv = env['ALIOTH_RUNTIME_CONTROL_TOKEN']
  if (fromEnv !== undefined) {
    // 显式置空 = 关闭文件载体（与 Meta 侧同语义）：不再回落文件，直接判无 token。
    if (fromEnv.length < RUNTIME_TOKEN_MIN_LENGTH) {
      throw new Error('runtime control token 缺失或过短：ALIOTH_RUNTIME_CONTROL_TOKEN 被显式设置但无效（文件载体已按语义关闭）')
    }
    return { token: fromEnv, source: 'env' }
  }
  const explicitFile = options.tokenFile ?? env['ALIOTH_RUNTIME_CONTROL_TOKEN_FILE']
  const file = explicitFile !== undefined && explicitFile !== ''
    ? explicitFile
    : path.join(homedir(), DEFAULT_TOKEN_FILE)
  const source: TokenResolution['source'] = explicitFile !== undefined && explicitFile !== '' ? 'env-file' : 'default-file'
  let raw: string
  try {
    raw = (await readText(file)).trim()
  } catch (error) {
    throw new Error(`runtime control token 不可读（${source}: ${file}）：${error instanceof Error ? error.message : String(error)}`)
  }
  if (raw.length < RUNTIME_TOKEN_MIN_LENGTH) {
    throw new Error(`runtime control token 过短（${source}: ${file}，<${RUNTIME_TOKEN_MIN_LENGTH} 字符）：fail-closed`)
  }
  const mode = await modeOf(file)
  if (mode !== 0o600) {
    throw new Error(`runtime control token 文件权限必须 0600（${source}: ${file}，实际 ${mode.toString(8)}）：fail-closed`)
  }
  return { token: raw, source }
}

/** 合同状态码 → outcome（§11）。 */
export function outcomeForStatus(status: number): ControlOutcome {
  if (status === 200) return 'ok'
  if (status === 401) return 'unauthorized'
  if (status === 409) return 'capability'
  if (status === 422) return 'unprocessable'
  if (status === 503) return 'unavailable'
  if (status === 504) return 'timeout'
  return 'error'
}

/** 能力快照的可判读投影（`409` 与消费面响应里的 `management_available`）。 */
export interface ControlCapability {
  readonly code: string | null
  readonly managementAvailable: boolean | null
  readonly engineEndpoint: string | null
  readonly detail: string
}

/**
 * 从响应体提取能力/管理可用性（缺项一律 `null`，不得猜测）。
 * @param body - 已解析的响应体（任意形态）。
 * @returns 投影（无可判读字段时各值为 null）。
 */
export function parseControlCapability(body: unknown): ControlCapability {
  const rec = typeof body === 'object' && body !== null ? body as Record<string, unknown> : {}
  const code = typeof rec['code'] === 'string' ? rec['code'] : null
  const management = rec['management_available']
  const engine = rec['engine_endpoint'] ?? rec['endpoint']
  const detail = typeof rec['detail'] === 'string'
    ? rec['detail']
    : typeof rec['message'] === 'string' ? rec['message'] : ''
  return {
    code,
    managementAvailable: typeof management === 'boolean' ? management : null,
    engineEndpoint: typeof engine === 'string' ? engine : null,
    detail,
  }
}

/** 发起一次请求（node 内建 http/https；超时按合同口径归 `timeout`）。 */
function performRequest(request: RuntimeControlRequest): Promise<{ status: number | null; text: string; error?: string }> {
  return new Promise(resolve => {
    const mod = request.url.protocol === 'https:' ? https : http
    const headers: Record<string, string> = { ...request.headers, host: request.hostHeader }
    if (request.body !== undefined) headers['content-type'] = 'application/json'
    const req = mod.request({
      protocol: request.url.protocol,
      hostname: request.url.hostname,
      port: request.url.port,
      path: `${request.url.pathname}${request.url.search}`,
      method: request.method,
      headers,
      ...(request.url.protocol === 'https:' ? { rejectUnauthorized: !request.insecureTls } : {}),
    }, res => {
      let text = ''
      res.setEncoding('utf8')
      res.on('data', chunk => { text += chunk })
      res.on('end', () => resolve({ status: res.statusCode ?? null, text }))
    })
    req.setTimeout(request.timeoutMs, () => {
      req.destroy(new Error(`control plane 超时（${request.timeoutMs}ms）`))
    })
    req.on('error', error => resolve({ status: null, text: '', error: error.message }))
    if (request.body !== undefined) req.write(request.body)
    req.end()
  })
}

/** 控制面客户端面。 */
export interface RuntimeControlClient {
  readonly list: () => Promise<ControlResponse>
  readonly start: (body: StartContainerBody) => Promise<ControlResponse>
  readonly status: (key: string) => Promise<ControlResponse>
  readonly logs: (key: string, tail?: number) => Promise<ControlResponse>
  readonly stop: (key: string) => Promise<ControlResponse>
  readonly routes: () => Promise<ControlResponse>
}

/** `POST /containers` 入参（§11 合同；`ns` 必需）。 */
export interface StartContainerBody {
  readonly ns: string
  readonly label?: string
  readonly tier?: string
  readonly pkg?: string
  readonly image?: string
}

/** 解析 JSON 体（失败则保留原文，不得静默当作空）。 */
function parseBody(text: string): unknown {
  if (text === '') return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

/**
 * 建客户端。
 * @param config - 端点/认证/TLS 配置。
 * @param deps - 注入点（测试用：替换传输与 token 解析）。
 * @returns 动词面（每个动词返回 {@link ControlResponse}，不抛异常）。
 */
export function createRuntimeControl(
  config: RuntimeControlConfig,
  deps: {
    readonly request?: (request: RuntimeControlRequest) => Promise<{ status: number | null; text: string; error?: string }>
    readonly resolveToken?: () => Promise<TokenResolution>
  } = {},
): RuntimeControlClient {
  const request = deps.request ?? performRequest
  const timeoutMs = config.timeoutMs ?? 30_000
  const base = new URL(config.url)
  const token = deps.resolveToken
    ?? (() => resolveRuntimeControlToken(process.env, {
      ...(config.token === undefined ? {} : { token: config.token }),
      ...(config.tokenFile === undefined ? {} : { tokenFile: config.tokenFile }),
    }))

  const call = async (method: string, pathname: string, body?: unknown): Promise<ControlResponse> => {
    let authorization: string
    try {
      authorization = `Bearer ${(await token()).token}`
    } catch (error) {
      return {
        outcome: 'unauthorized',
        status: null,
        body: null,
        text: '',
        detail: error instanceof Error ? error.message : String(error),
      }
    }
    const url = new URL(pathname, base)
    const result = await request({
      method,
      url,
      hostHeader: config.host ?? base.host,
      headers: { authorization, accept: 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      timeoutMs,
      insecureTls: config.insecureTls ?? false,
    })
    if (result.status === null) {
      const detail = result.error ?? '请求失败'
      const isTimeout = /超时|timeout/i.test(detail)
      return { outcome: isTimeout ? 'timeout' : 'error', status: null, body: null, text: '', detail }
    }
    const parsed = parseBody(result.text)
    const capability = parseControlCapability(parsed)
    const detail = capability.detail !== ''
      ? capability.detail
      : `${method} ${pathname} → ${result.status}`
    return { outcome: outcomeForStatus(result.status), status: result.status, body: parsed, text: result.text, detail }
  }

  return {
    list: () => call('GET', '/containers'),
    start: body => call('POST', '/containers', body),
    status: key => call('GET', `/containers/${encodeURIComponent(key)}`),
    logs: (key, tail) => call('GET', `/containers/${encodeURIComponent(key)}/logs${tail === undefined ? '' : `?tail=${tail}`}`),
    stop: key => call('DELETE', `/containers/${encodeURIComponent(key)}`),
    routes: () => call('GET', '/api/meta/runtime/routes'),
  }
}
