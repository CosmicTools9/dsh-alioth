/**
 * 控制面客户端契约：token 载体解析与 Meta 侧同序（显式 ▶ env ▶ 文件，显式置空关文件载体；
 * 文件 0600 且 ≥32 字符否则 fail-closed）、状态码归一（§11）、动词与请求构造（Host 覆写 + Bearer）。
 * 真 HTTP 往返在本地 server 上跑（不只 mock），确保传输层可用。
 */
import { createServer } from 'node:http'
import { describe, expect, it } from 'vitest'
import {
  createRuntimeControl,
  outcomeForStatus,
  parseControlCapability,
  resolveRuntimeControlToken,
  RUNTIME_TOKEN_MIN_LENGTH,
  type RuntimeControlRequest,
} from '../src/runtime-control.ts'

const TOKEN = 't'.repeat(RUNTIME_TOKEN_MIN_LENGTH)

describe('resolveRuntimeControlToken', () => {
  it('prefers the explicit config token and rejects a short one', async () => {
    await expect(resolveRuntimeControlToken({}, { token: TOKEN })).resolves.toEqual({ token: TOKEN, source: 'config' })
    await expect(resolveRuntimeControlToken({}, { token: 'short' })).rejects.toThrow(/过短/)
  })

  it('uses the env token, and an explicitly set invalid env value disables the file carrier', async () => {
    await expect(resolveRuntimeControlToken({ ALIOTH_RUNTIME_CONTROL_TOKEN: TOKEN }))
      .resolves.toEqual({ token: TOKEN, source: 'env' })
    const readText = async (): Promise<string> => TOKEN
    await expect(resolveRuntimeControlToken({ ALIOTH_RUNTIME_CONTROL_TOKEN: '' }, {}, readText))
      .rejects.toThrow(/显式设置但无效/)
  })

  it('reads an explicit token file and enforces 0600 + length', async () => {
    const readText = async (): Promise<string> => `${TOKEN}\n`
    const ok = await resolveRuntimeControlToken({}, { tokenFile: '/tmp/tok' }, readText, async () => 0o600)
    expect(ok).toEqual({ token: TOKEN, source: 'env-file' })

    await expect(resolveRuntimeControlToken({}, { tokenFile: '/tmp/tok' }, readText, async () => 0o644))
      .rejects.toThrow(/0600/)
    await expect(resolveRuntimeControlToken({}, { tokenFile: '/tmp/tok' }, async () => 'tiny', async () => 0o600))
      .rejects.toThrow(/过短/)
  })

  it('falls back to the default file under $HOME and reports unreadable carriers', async () => {
    await expect(resolveRuntimeControlToken({}, {}, async () => { throw new Error('ENOENT') }, async () => 0o600))
      .rejects.toThrow(/\.alioth\/runtime-control\.token/)
  })
})

describe('outcomeForStatus', () => {
  it('maps every contract status and treats unknown codes as errors', () => {
    expect(outcomeForStatus(200)).toBe('ok')
    expect(outcomeForStatus(401)).toBe('unauthorized')
    expect(outcomeForStatus(409)).toBe('capability')
    expect(outcomeForStatus(422)).toBe('unprocessable')
    expect(outcomeForStatus(503)).toBe('unavailable')
    expect(outcomeForStatus(504)).toBe('timeout')
    expect(outcomeForStatus(418)).toBe('error')
  })
})

describe('parseControlCapability', () => {
  it('projects the capability snapshot and never guesses missing values', () => {
    expect(parseControlCapability({
      code: 'container_service_remote_readonly',
      engine_endpoint: 'ssh://host',
      management_available: false,
      detail: '非本机引擎',
    })).toEqual({
      code: 'container_service_remote_readonly',
      managementAvailable: false,
      engineEndpoint: 'ssh://host',
      detail: '非本机引擎',
    })
    expect(parseControlCapability('nope')).toEqual({ code: null, managementAvailable: null, engineEndpoint: null, detail: '' })
  })
})

describe('createRuntimeControl', () => {
  const stub = (responses: Array<{ status: number | null; text: string; error?: string }>) => {
    const seen: RuntimeControlRequest[] = []
    const client = createRuntimeControl(
      { url: 'https://198.51.100.7:2048', host: 'isahl.com' },
      {
        resolveToken: async () => ({ token: TOKEN, source: 'config' }),
        request: async (request) => {
          seen.push(request)
          return responses.shift() ?? { status: 500, text: '' }
        },
      },
    )
    return { client, seen }
  }

  it('sends Bearer auth with the Host override and JSON body, and maps the reply', async () => {
    const { client, seen } = stub([{ status: 200, text: '{"containers":[]}' }])
    const res = await client.start({ ns: 'WZ', image: 'alioth-ns-wz:abcd1234' })
    expect(res.outcome).toBe('ok')
    expect(res.body).toEqual({ containers: [] })
    expect(seen[0]?.method).toBe('POST')
    expect(seen[0]?.hostHeader).toBe('isahl.com')
    expect(seen[0]?.headers['authorization']).toBe(`Bearer ${TOKEN}`)
    expect(seen[0]?.body).toBe('{"ns":"WZ","image":"alioth-ns-wz:abcd1234"}')
    expect(seen[0]?.url.pathname).toBe('/containers')
  })

  it('classifies 401/409/422/503/504 per contract and keeps the capability body', async () => {
    const { client } = stub([
      { status: 401, text: '{"detail":"缺少或错误的控制面 token"}' },
      { status: 409, text: '{"code":"container_service_unavailable","management_available":false,"detail":"容器服务未开启"}' },
      { status: 422, text: '' },
      { status: 503, text: '' },
      { status: 504, text: '' },
    ])
    expect((await client.list()).outcome).toBe('unauthorized')
    const capability = await client.list()
    expect(capability.outcome).toBe('capability')
    expect(parseControlCapability(capability.body).managementAvailable).toBe(false)
    expect((await client.status('k')).outcome).toBe('unprocessable')
    expect((await client.logs('k')).outcome).toBe('unavailable')
    expect((await client.stop('k')).outcome).toBe('timeout')
  })

  it('reports a missing token as unauthorized without touching the network, and classifies transport failures', async () => {
    const requests: RuntimeControlRequest[] = []
    const client = createRuntimeControl(
      { url: 'https://198.51.100.7:2048' },
      {
        resolveToken: async () => { throw new Error('runtime control token 不可读') },
        request: async (request) => { requests.push(request); return { status: 200, text: '' } },
      },
    )
    const unauthorized = await client.list()
    expect(unauthorized.outcome).toBe('unauthorized')
    expect(requests).toHaveLength(0)

    // 一直不响应的对端 ⇒ 客户端自己的超时归 `timeout`（合同 504 同族）。
    const hanging = createServer(() => { /* 故意不响应 */ })
    await new Promise<void>(resolve => { hanging.listen(0, '127.0.0.1', resolve) })
    try {
      const address = hanging.address()
      const port = typeof address === 'object' && address !== null ? address.port : 0
      const timingOut = createRuntimeControl({ url: `http://127.0.0.1:${port}`, token: TOKEN, timeoutMs: 60 })
      expect((await timingOut.list()).outcome).toBe('timeout')
    } finally {
      await new Promise<void>(resolve => { hanging.close(() => resolve()) })
    }

    // 连接被拒是另一种失败（不得冒充超时）。
    const refused = createRuntimeControl({ url: 'http://127.0.0.1:9', token: TOKEN, timeoutMs: 200 })
    expect((await refused.list()).outcome).toBe('error')
  })

  it('passes the tail query through and uses DELETE for stop', async () => {
    const { client, seen } = stub([{ status: 200, text: 'ok' }, { status: 200, text: 'ok' }])
    await client.logs('k1', 40)
    await client.stop('k1')
    expect(seen[0]?.url.pathname).toBe('/containers/k1/logs')
    expect(seen[0]?.url.search).toBe('?tail=40')
    expect(seen[1]?.method).toBe('DELETE')
  })

  it('constructs an https request (including the explicit insecure-TLS opt-in) and reports the transport failure', async () => {
    // 只覆盖 https 分支的构造与失败分类：不搭真 TLS 服务（自签证书会引入平台依赖）。
    const client = createRuntimeControl({ url: 'https://127.0.0.1:9', token: TOKEN, insecureTls: true, timeoutMs: 500 })
    const res = await client.list()
    expect(res.outcome).toBe('error')
    expect(res.status).toBeNull()
  })

  it('round-trips against a real HTTP server (transport layer, not a mock)', async () => {
    const server = createServer((req, res) => {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ seenHost: req.headers.host, code: 0 }))
    })
    await new Promise<void>(resolve => { server.listen(0, '127.0.0.1', resolve) })
    try {
      const address = server.address()
      const port = typeof address === 'object' && address !== null ? address.port : 0
      const client = createRuntimeControl(
        { url: `http://127.0.0.1:${port}`, host: 'isahl.com', token: TOKEN },
      )
      const res = await client.list()
      expect(res.outcome).toBe('ok')
      expect(res.body).toEqual({ seenHost: 'isahl.com', code: 0 })
    } finally {
      await new Promise<void>(resolve => { server.close(() => resolve()) })
    }
  })
})
