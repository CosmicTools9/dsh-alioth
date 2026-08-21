import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as landingAlioth from '@dsh-alioth/landing-alioth'
import * as authWeb from '../src/index.ts'

const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TYPE isahl_meta.field_category AS ENUM ('scalar', 'reference', 'computed', 'auto');
CREATE TYPE isahl_meta.field_data_type AS ENUM ('text', 'decimal', 'bigint');
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL,
    name text NOT NULL,
    type isahl_meta.collection_type,
    config jsonb DEFAULT '{}'::jsonb,
    data_source text,
    schema text DEFAULT 'isahl'::text,
    biz_description text,
    PRIMARY KEY (table_name)
);
CREATE TABLE isahl_meta.meta_fields (
    fk_collection text NOT NULL REFERENCES isahl_meta.meta_collections(table_name) ON DELETE CASCADE,
    name text NOT NULL,
    category isahl_meta.field_category,
    data_type isahl_meta.field_data_type,
    is_required boolean DEFAULT false,
    default_value text,
    config jsonb DEFAULT '{}'::jsonb,
    title text NOT NULL DEFAULT ''::text,
    PRIMARY KEY (fk_collection, name)
);
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let port: number

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'authweb-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'authweb-data-'))
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

  ctx = new Context()
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
  disposers.push(() => env.dispose())
  await ctx.aliothEnv.ready()

  // Real harness webServer (port 0 → kernel-assigned) mounted BEFORE the
  // carriers so their deferred ctx.inject(['webServer']) callbacks find it.
  const webServerPlugin = await ctx.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  disposers.push(() => webServerPlugin.dispose())
  const landing = await ctx.plugin(landingAlioth, {})
  disposers.push(() => landing.dispose())
  const auth = await ctx.plugin(authAlioth, {
    mode: 'enforce',
    preProcRoot: path.join(dataRoot, 'pre-proc'),
    deployRoot: path.join(dataRoot, 'deploy'),
  })
  disposers.push(() => auth.dispose())

  port = 3987 + Math.floor(Math.random() * 500)
  const carrier = await ctx.plugin(authWeb, { port })
  disposers.push(() => carrier.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('B/S HTTP surface (real server)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('serves the landing page at GET / (via the aliothLanding service)', async () => {
    const response = await fetch(`${base()}/`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('Alioth AppCreator')
    expect(html).toContain('app-creation')
    expect(html).toContain('e2e-verification')
  })

  it('serves the login page (GET /login)', async () => {
    const response = await fetch(`${base()}/login`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<form')
    expect(html).toContain('/api/auth/login')
  })

  it('links register page back to /login', async () => {
    const response = await fetch(`${base()}/register`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('href="/login"')
  })

  it('registers via browser form submission (urlencoded) → styled success page + cookies', async () => {
    const form = new URLSearchParams({ username: 'carol', password: 'password-789' })
    const response = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: form.toString(),
    })
    expect(response.status).toBe(201)
    expect(response.headers.get('content-type')).toContain('text/html')
    const html = await response.text()
    expect(html).toContain('U-carol')
    expect(html).toMatch(/class="token">[0-9a-f]{64}</)
    const cookies = response.headers.getSetCookie()
    expect(cookies.some(c => c.startsWith('alioth_session=') && c.includes('HttpOnly'))).toBe(true)
    expect(cookies.some(c => c.startsWith('alioth_user=carol'))).toBe(true)
  })

  it('rejects a browser form login with bad credentials → styled 401 page', async () => {
    const form = new URLSearchParams({ username: 'carol', password: 'wrong-pass-000' })
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: form.toString(),
    })
    expect(response.status).toBe(401)
    const html = await response.text()
    expect(html).toContain('用户名或密码错误')
    expect(html).toContain('/api/auth/login') // form re-rendered for retry
  })

  it('logs in via JSON API and reads /me', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    expect(login.status).toBe(200)
    const session = await login.json() as { token: string }
    const me = await fetch(`${base()}/api/auth/me`, {
      headers: { authorization: `Bearer ${session.token}` },
    })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', namespace: 'U-carol', role: 'admin' })
  })

  it('rejects invalid credentials over HTTP', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'wrong-password' }),
    })
    expect(response.status).toBe(400)
    expect((await response.json() as { error: string }).error).toContain('invalid credentials')
  })

  it('logs out over HTTP and the token stops working', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    await fetch(`${base()}/api/auth/logout`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}` },
    })
    const me = await fetch(`${base()}/api/auth/me`, { headers: { authorization: `Bearer ${token}` } })
    expect(me.status).toBe(401)
  })
})

describe('web gate (real harness WebServer)', () => {
  const webBase = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  it('serves the landing route mounted by landing-alioth on the GUI origin', async () => {
    const response = await fetch(`${webBase()}/landing`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('Alioth AppCreator')
    expect(html).toContain('app-creation')
  })

  it('serves login/register on the GUI origin', async () => {
    const login = await fetch(`${webBase()}/login`)
    expect(login.status).toBe(200)
    expect(await login.text()).toContain('/api/auth/login')

    const register = await fetch(`${webBase()}/register`)
    expect(register.status).toBe(200)
    expect(await register.text()).toContain('href="/login"')
  })

  it('sets session + marker cookies on login and accepts the cookie on /me', async () => {
    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    expect(login.status).toBe(200)
    const cookies = login.headers.getSetCookie()
    const session = cookies.find(c => c.startsWith('alioth_session='))
    expect(session).toBeDefined()
    expect(session).toContain('HttpOnly')
    expect(cookies.some(c => c.startsWith('alioth_user=carol'))).toBe(true)

    const me = await fetch(`${webBase()}/api/auth/me`, { headers: { cookie: session!.split(';')[0]! } })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', namespace: 'U-carol' })
  })

  it('binds agent sessions via /api/auth/bind; rejects bad tokens', async () => {
    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const bind = await fetch(`${webBase()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token, sessionId: 'web-session-1' }),
    })
    expect(bind.status).toBe(204)
    expect(await ctx.aliothAuth.userForSessionId('web-session-1')).toMatchObject({ namespace: 'U-carol' })

    const bad = await fetch(`${webBase()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: 'deadbeef'.repeat(8), sessionId: 'web-session-2' }),
    })
    expect(bad.status).toBe(401)
    expect(await ctx.aliothAuth.userForSessionId('web-session-2')).toBeNull()
  })

  it('injects the gate script through the index tap (target from aliothLanding)', () => {
    const out = ctx.webServer.applyIndexTaps('<html><head></head><body></body></html>')
    expect(out).toContain('alioth_user')
    expect(out).toContain("location.replace('/landing')")
    expect(out).toContain('/api/sessions.create')
    // The injected script must be syntactically valid JS — a broken gate
    // silently never redirects (this regressed once on string-concat seams).
    const match = out.match(/<script>([\s\S]*?)<\/script>/)
    if (match === null) throw new Error('gate script not injected')
    expect(() => new Function(match[1]!)).not.toThrow()
  })
})

describe('workspace surface (工作区 unlimited / 应用 standard)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('requires authentication for /api/workspace', async () => {
    const response = await fetch(`${base()}/api/workspace`)
    expect(response.status).toBe(401)
  })

  it('returns the user workspace list with AliothStudio paths (standard mode)', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const response = await fetch(`${base()}/api/workspace`, {
      headers: { authorization: `Bearer ${token}` },
    })
    expect(response.status).toBe(200)
    const body = await response.json() as {
      mode: 'standard' | 'unlimited'
      workspaces: Array<{ namespace: string; preProcPath: string; deployPath: string; apps: Array<{ code: string; name: string }> }>
    }
    expect(body.mode).toBe('standard')
    expect(body.workspaces).toHaveLength(1)
    expect(body.workspaces[0]).toMatchObject({
      namespace: 'U-carol',
      apps: [],
    })
    expect(body.workspaces[0]!.preProcPath).toContain('pre-proc')
    expect(body.workspaces[0]!.deployPath).toContain('deploy')
  })

  it('serves the workspace page (standard renders 应用 without workspace chrome)', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const page = await fetch(`${base()}/workspace`, {
      headers: { authorization: `Bearer ${token}` },
    })
    expect(page.status).toBe(200)
    const html = await page.text()
    expect(html).toContain('<h1>应用</h1>')
    expect(html).toContain('U-carol')
    // standard hides the custom-workspace chrome: no Pre-Proc/Deploy paths
    expect(html).not.toContain('Pre-Proc/U-carol/')
    expect(html).not.toContain('Deploy/U-carol/')
  })

  it('redirects unauthenticated visitors from /workspace to /login', async () => {
    const response = await fetch(`${base()}/workspace`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('renders 工作区 with paths when the deployment is unlimited', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const saved = process.env.ALIOTH_WORKSPACE_MODE
    try {
      process.env.ALIOTH_WORKSPACE_MODE = 'unlimited'
      const page = await fetch(`${base()}/workspace`, {
        headers: { authorization: `Bearer ${token}` },
      })
      expect(page.status).toBe(200)
      const html = await page.text()
      expect(html).toContain('<h1>工作区</h1>')
      expect(html).toContain('U-carol')
      expect(html).toContain('Pre-Proc/U-carol/')
      expect(html).toContain('Deploy/U-carol/')
    } finally {
      if (saved === undefined) { delete process.env.ALIOTH_WORKSPACE_MODE } else { process.env.ALIOTH_WORKSPACE_MODE = saved }
    }
  })

  it('includes the resolved workspace mode in /api/auth/me (client chip entry)', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const me = await fetch(`${base()}/api/auth/me`, { headers: { authorization: `Bearer ${token}` } })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', workspaceMode: 'standard' })
  })
})

describe('client face artifact', () => {
  it('ships a valid client module (shell.overlay user chip)', async () => {
    // Hand-authored closure-factory (no build step) — guard its contract:
    // correct module id, react as the only platform require, inject+apply
    // exports, registration into shell.overlay with a stable entry id.
    const source = await readFile(new URL('../lib/client.js', import.meta.url), 'utf8')
    let registration: { id: string; factory: (require: (name: string) => unknown) => Record<string, unknown> } | undefined
    const fakeWindow = { __ModuleLoader__: { load: (r: typeof registration) => { registration = r } } }
    new Function('window', source)(fakeWindow)
    expect(registration?.id).toBe('@dsh-alioth/auth-web-alioth')

    const reactStub = {
      createElement: (...args: unknown[]) => args,
      useState: (value: unknown) => [value, () => {}],
      useEffect: () => {},
    }
    const exports = registration!.factory((name: string) => {
      if (name !== 'react') throw new Error(`unexpected require: ${name}`)
      return reactStub
    })
    expect(exports.inject).toEqual(['slots'])
    expect(typeof exports.apply).toBe('function')

    let injectedKey: string | undefined
    let captured: { options: { name: string; id: string }; component: unknown } | undefined
    const ctxStub = {
      effect: (fn: () => unknown) => { fn() },
      slots: {
        // Registration defers through slots.inject (shell.overlay is declared
        // by ui-layout after plugin apply; direct register races it).
        inject: (key: string, callback: () => unknown) => {
          injectedKey = key
          callback()
          return () => {}
        },
        register: (options: { name: string; id: string }, component: unknown) => {
          captured = { options, component }
          return () => {}
        },
      },
    }
    ;(exports.apply as (c: unknown) => void)(ctxStub)
    expect(injectedKey).toBe('shell.overlay')
    expect(captured?.options).toEqual({ name: 'shell.overlay', id: 'alioth-user-chip' })
    expect(typeof captured?.component).toBe('function')
  })
})
