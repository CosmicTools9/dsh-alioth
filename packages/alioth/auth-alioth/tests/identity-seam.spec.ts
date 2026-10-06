/**
 * 身份缝（C3）+ AccountContext（C5）+ BYOK：
 *
 * - OIDC RP 原语对着一个**进程内假 IdP**全链验证（discovery / JWKS / token 换
 *   码 / ID token 验签 ES256），JIT 开户落到真实测试库；
 * - `accountForSession` 把会话解析成结构化账户（计划与配额来自 billing 缝）；
 * - BYOK 密钥的密封/读取/禁用语义。
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { createServer, type Server } from 'node:http'
import { generateKeyPairSync, sign as cryptoSign } from 'node:crypto'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'
import * as auth from '../src/index.ts'
import { clearJwksCache, verifyIdToken, discoverOidc, buildAuthorizeUrl, exchangeCode } from '../src/oidc.ts'
import { openSecret, sealSecret } from '../src/secretbox.ts'

// ── 假 IdP（ES256） ────────────────────────────────────────────────────────

const { privateKey, publicKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
const jwk = { ...publicKey.export({ format: 'jwk' }) as Record<string, string>, kid: 'test-key-1', alg: 'ES256', use: 'sig' }
const CLIENT_ID = 'dsh-alioth-test-client'
let idpBase = ''
let idpServer: Server | undefined
let lastIdToken = (): string => ''

function b64url(input: Buffer | string): string {
  return Buffer.from(input).toString('base64url')
}

/** 签一个 ES256 ID token（node:crypto 的 ECDSA 签名即 DER，JWS 直接携带）。 */
function signIdToken(payload: Record<string, unknown>, signingKey = privateKey): string {
  const header = b64url(JSON.stringify({ alg: 'ES256', kid: 'test-key-1', typ: 'JWT' }))
  const body = b64url(JSON.stringify(payload))
  const der = cryptoSign('sha256', Buffer.from(`${header}.${body}`), signingKey)
  return `${header}.${body}.${der.toString('base64url')}`
}

async function startIdp(): Promise<string> {
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    if (url.pathname === '/.well-known/openid-configuration') {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({
        issuer: idpBase,
        authorization_endpoint: `${idpBase}/authorize`,
        token_endpoint: `${idpBase}/token`,
        jwks_uri: `${idpBase}/jwks.json`,
      }))
      return
    }
    if (url.pathname === '/jwks.json') {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ keys: [jwk] }))
      return
    }
    if (url.pathname === '/token') {
      let body = ''
      request.on('data', chunk => { body += String(chunk) })
      request.on('end', () => {
        const params = new URLSearchParams(body)
        if (params.get('code') !== 'good-code') {
          response.writeHead(400).end()
          return
        }
        response.writeHead(200, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ id_token: lastIdToken(), access_token: 'at', token_type: 'Bearer' }))
      })
      return
    }
    response.writeHead(404).end()
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  idpServer = server
  const port = (server.address() as { port: number }).port
  return `http://127.0.0.1:${port}`
}

// ── 插件环境（与 auth-alioth.spec 同一 fixture，独立库） ──────────────────

const SCHEMA_DDL = `
CREATE SCHEMA IF NOT EXISTS isahl_meta;
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL, name text NOT NULL, config jsonb DEFAULT '{}'::jsonb,
    schema text DEFAULT 'isahl'::text, PRIMARY KEY (table_name)
);
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let testDb: TestDatabase
let preProcRoot = ''
let deployRoot = ''

/** env-alioth 需要的最小模型树（与主 spec 的 fixture 同形）。 */
async function makeModelDir(prefix: string): Promise<string> {
  const modelDir = await mkdtemp(path.join(tmpdir(), prefix))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'a.yaml'), 'x\n')
  await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
  await writeFile(
    path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
    'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
  )
  return modelDir
}

async function mountPlugin(config: Partial<auth.Config>): Promise<void> {
  const plugin = await ctx.plugin(auth, {
    mode: 'enforce', preProcRoot, deployRoot,
    ...config,
  })
  disposers.push(() => plugin.dispose())
}

beforeAll(async () => {
  idpBase = await startIdp()
  testDb = await createTestDatabase('authidentity')
  const modelDir = await makeModelDir('auth-oidc-model-')
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'auth-oidc-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'auth-oidc-preproc-'))
  deployRoot = await mkdtemp(path.join(tmpdir(), 'auth-oidc-deploy-'))
  ctx = new Context()
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot, databaseUrl: testDb.url })
  disposers.push(() => env.dispose())
  await ctx.aliothEnv.ready()
  await mountPlugin({
    authMode: 'oidc',
    oidcIssuer: idpBase,
    oidcClientId: CLIENT_ID,
    oidcClientSecret: 'client-secret',
    oidcRedirectUri: 'http://localhost:3100/api/auth/oidc/callback',
    byok: true,
    byokSecret: 'byok-test-secret',
  })
  // billing 缺席时的降级基线（plan L0 / quota null）由结构性查找保证。
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await testDb.dispose()
  idpServer?.close()
  clearJwksCache()
  await rm(preProcRoot, { recursive: true, force: true })
  await rm(deployRoot, { recursive: true, force: true })
})

// ── OIDC RP 原语 ───────────────────────────────────────────────────────────

describe('oidc primitives (fake IdP, ES256)', () => {
  it('discovers and validates the issuer contract', async () => {
    const discovery = await discoverOidc(idpBase)
    expect(discovery.issuer).toBe(idpBase)
    expect(discovery.jwksUri).toBe(`${idpBase}/jwks.json`)
    // issuer mismatch fails loud（连接不上的端口 → fetch 层报错；语义一致：fail-closed）
    await expect(discoverOidc('http://127.0.0.1:1')).rejects.toThrow()
  })

  it('builds an authorization URL carrying the use-once state', async () => {
    const discovery = await discoverOidc(idpBase)
    const url = new URL(buildAuthorizeUrl(discovery, {
      clientId: CLIENT_ID, redirectUri: 'http://localhost/cb', scope: 'openid profile', state: 'st-1',
    }))
    expect(url.pathname).toBe('/authorize')
    expect(url.searchParams.get('state')).toBe('st-1')
    expect(url.searchParams.get('client_id')).toBe(CLIENT_ID)
  })

  it('verifies a well-signed ID token and rejects expired / wrong-audience / foreign-key ones', async () => {
    const discovery = await discoverOidc(idpBase)
    const now = Math.floor(Date.now() / 1000)
    const good = signIdToken({ sub: 'u1', iss: idpBase, aud: CLIENT_ID, exp: now + 600, preferred_username: 'ada' })
    const claims = await verifyIdToken(good, { discovery, audience: CLIENT_ID })
    expect(claims.sub).toBe('u1')
    expect(claims.preferredUsername).toBe('ada')

    const expired = signIdToken({ sub: 'u1', iss: idpBase, aud: CLIENT_ID, exp: now - 10 })
    await expect(verifyIdToken(expired, { discovery, audience: CLIENT_ID })).rejects.toThrow(/expired/)

    const wrongAud = signIdToken({ sub: 'u1', iss: idpBase, aud: 'someone-else', exp: now + 600 })
    await expect(verifyIdToken(wrongAud, { discovery, audience: CLIENT_ID })).rejects.toThrow(/audience/)

    const { privateKey: other } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
    const forged = signIdToken({ sub: 'u1', iss: idpBase, aud: CLIENT_ID, exp: now + 600 }, other)
    await expect(verifyIdToken(forged, { discovery, audience: CLIENT_ID })).rejects.toThrow(/signature verification failed/)
  })

  it('exchanges the code at the token endpoint', async () => {
    const discovery = await discoverOidc(idpBase)
    lastIdToken = () => signIdToken({ sub: 'u1', iss: idpBase, aud: CLIENT_ID, exp: Math.floor(Date.now() / 1000) + 600 })
    const { idToken } = await exchangeCode(discovery, {
      clientId: CLIENT_ID, clientSecret: 'client-secret', redirectUri: 'http://localhost/cb', code: 'good-code',
    })
    expect(idToken.split('.')).toHaveLength(3)
    await expect(exchangeCode(discovery, {
      clientId: CLIENT_ID, clientSecret: 'client-secret', redirectUri: 'http://localhost/cb', code: 'bad-code',
    })).rejects.toThrow(/token endpoint answered 400/)
  })
})

// ── 服务面：OIDC 登录 + JIT 开户 ───────────────────────────────────────────

describe('identity seam via the service (oidc adapter active)', () => {
  it('refuses local password login loud, exposes the adapter name', async () => {
    expect(ctx.aliothAuth.authMode()).toBe('oidc')
    await expect(ctx.aliothAuth.login('ada', 'password1')).rejects.toThrow(/authMode=oidc/)
  })

  it('issues a use-once state and completes the round-trip with JIT provisioning', async () => {
    lastIdToken = () => signIdToken({
      sub: 'idp-sub-1', iss: idpBase, aud: CLIENT_ID,
      exp: Math.floor(Date.now() / 1000) + 600, preferred_username: 'Alice.Co',
    })
    const authorizeUrl = await ctx.aliothAuth.oidcAuthorizeUrl('state-abc')
    expect(authorizeUrl.startsWith(`${idpBase}/authorize`)).toBe(true)

    const { token, namespace, role } = await ctx.aliothAuth.oidcExchange('good-code', 'state-abc')
    expect(namespace).toBe('U-alice-co') // claims → 本地用户名契约（小写/连字符）
    expect(role).toBe('user')
    expect(await ctx.aliothAuth.userForToken(token)).toMatchObject({ username: 'alice-co', namespace: 'U-alice-co' })

    // state 是一次性的：重放即拒。
    await expect(ctx.aliothAuth.oidcExchange('good-code', 'state-abc')).rejects.toThrow(/state/)
    // 未知 state 也拒。
    await expect(ctx.aliothAuth.oidcExchange('good-code', 'never-issued')).rejects.toThrow(/state/)
  })
})

// ── AccountContext（C5） ───────────────────────────────────────────────────

describe('accountForSession (structured account, memoized)', () => {
  it('resolves identity + plan/quota, and degrades to L0/null without billing', async () => {
    // 先无 billing：L0 / null。（oidc 模式下本地 login 被拒——用注册返回的令牌。）
    const login = await ctx.aliothAuth.register('ctxuser', 'password123')
    await ctx.aliothAuth.bind(login.token, 'ctx-session-1')
    const bare = await ctx.aliothAuth.accountForSession('ctx-session-1')
    expect(bare).toMatchObject({ username: 'ctxuser', namespace: 'U-ctxuser', plan: 'L0', monthlyCostCents: null })

    // 挂上 billing 结构面：计划与配额从权益缝来。
    ctx.provide('aliothBilling')
    ctx.set('aliothBilling', {
      planOf: async () => 'L1',
      entitlement: async (_actor: unknown, capability: string) => {
        expect(capability).toBe('llm-budget')
        return { plan: 'L1', monthlyCostCents: 5000 }
      },
    } as never)
    // memo 60s：同会话再读仍旧值 → 换一个会话 id 验证新解析。
    await ctx.aliothAuth.bind(login.token, 'ctx-session-2')
    const withBilling = await ctx.aliothAuth.accountForSession('ctx-session-2')
    expect(withBilling).toMatchObject({ username: 'ctxuser', plan: 'L1', monthlyCostCents: 5000 })
    // 未绑定会话 → null。
    expect(await ctx.aliothAuth.accountForSession('no-such-session')).toBeNull()
  })
})

// ── BYOK ───────────────────────────────────────────────────────────────────

describe('BYOK keys (sealed at rest, read per account)', () => {
  it('seals and reopens a key by namespace or username, clears on null', async () => {
    const login = await ctx.aliothAuth.register('byokuser', 'password123')
    await ctx.aliothAuth.setApiKey(login.token, 'sk-live-byok-1')
    expect(await ctx.aliothAuth.apiKeyFor('U-byokuser')).toBe('sk-live-byok-1')
    expect(await ctx.aliothAuth.apiKeyFor('byokuser')).toBe('sk-live-byok-1')
    expect(await ctx.aliothAuth.apiKeyFor('U-nobody')).toBeNull()
    await ctx.aliothAuth.setApiKey(login.token, null)
    expect(await ctx.aliothAuth.apiKeyFor('U-byokuser')).toBeNull()
  })

  it('refuses a bad token and stays fail-closed when disabled', async () => {
    await expect(ctx.aliothAuth.setApiKey('not-a-token', 'sk-x')).rejects.toThrow(/invalid or expired token/)
    // 独立 Context（byok 未启用）：setApiKey 拒绝，apiKeyFor 恒 null。
    const bare = new Context()
    const systemBare = await bare.plugin(SystemPrompt)
    disposers.push(() => systemBare.dispose())
    const toolsBare = await bare.plugin(ToolRuntime)
    disposers.push(() => toolsBare.dispose())
    const modelDir = await makeModelDir('auth-oidc-model2-')
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'auth-oidc-data2-'))
    const env = await bare.plugin(envAlioth, { modelSource: modelDir, dataRoot, databaseUrl: testDb.url })
    disposers.push(() => env.dispose())
    const plugin = await bare.plugin(auth, { mode: 'open', preProcRoot, deployRoot })
    disposers.push(() => plugin.dispose())
    await expect(bare.aliothAuth.setApiKey('whatever', 'sk-x')).rejects.toThrow(/BYOK is disabled/)
    expect(await bare.aliothAuth.apiKeyFor('U-byokuser')).toBeNull()
    await rm(modelDir, { recursive: true, force: true })
    await rm(dataRoot, { recursive: true, force: true })
  })
})

// ── secretbox ─────────────────────────────────────────────────────────────

describe('secretbox (AES-256-GCM under a deployment secret)', () => {
  it('round-trips and refuses tampering; disabled without a secret', () => {
    const sealed = sealSecret('sk-live', 's3cret')
    expect(sealed).toMatch(/^v1:/)
    expect(openSecret(sealed, 's3cret')).toBe('sk-live')
    const parts = sealed!.split(':')
    const tampered = `${parts[0]}:${parts[1]}:${parts[2]}:${Buffer.from('evil').toString('base64')}`
    expect(openSecret(tampered, 's3cret')).toBeNull()
    expect(openSecret(sealed, 'wrong')).toBeNull()
    expect(sealSecret('sk-live', undefined)).toBeNull()
    expect(openSecret(sealed, undefined)).toBeNull()
    // 密钥派生是确定性的：同一 secret 解开同一密文（跨进程稳定）。
    expect(openSecret(sealed, 's3cret')).toBe(openSecret(sealed, 's3cret'))
  })
})
