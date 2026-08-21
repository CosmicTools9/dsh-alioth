import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'
import { type Agent } from '@deepseek-ai/dsh-agent'
import { Session, SessionId } from '@deepseek-ai/dsh-session'

import * as tool from '../src/index.ts'

const signal = new AbortController().signal

/** Self-contained valid Alioth app.json (hand-written test data). */
const VALID_APP = {
  id: '946462018160351133',
  code: 'ai-i-need-a',
  namespace: 'Alioth',
  name: 'ai-i-need-a',
  version: '0.1.0',
  config: {
    modules: ['inventory', 'demand'],
    blocks: ['block-list-inventory'],
  },
  permissions: {
    defaultRoles: ['admin', 'user'],
    publicPaths: ['/login'],
    adminRoles: ['admin'],
  },
  routing: { base: '/apps/ai-i-need-a', defaultRoute: '/inventory' },
  navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'demand'] }],
  min_alioth_version: '10.0.0',
}

let root: string
let ctx: Context
let counter = 0

function callInspect(args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: CallId(`call-${++counter}`),
    name: 'alioth_app_inspect',
    arguments: args,
  })
}

function callList(args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: CallId(`list-${++counter}`),
    name: 'alioth_app_list',
    arguments: args,
  })
}

async function writeApp(namespace: string, app: string, content: unknown): Promise<void> {
  const dir = path.join(root, namespace, 'Apps', app)
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, 'app.json'), JSON.stringify(content))
}

beforeAll(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-'))
  await writeApp('Alioth', 'ai-i-need-a', VALID_APP)
  const brokenDir = path.join(root, 'Alioth', 'Apps', 'broken')
  await mkdir(brokenDir, { recursive: true })
  await writeFile(path.join(brokenDir, 'app.json'), '{not json')
  await writeApp('Alioth', 'incomplete', { id: 'x', code: 'incomplete' })
  ctx = new Context()
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(tool, { preProcRoot: root })
})

afterAll(async () => {
  await rm(root, { recursive: true, force: true })
})

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function propType(value: unknown): string | undefined {
  return isRecord(value) && typeof value.type === 'string' ? value.type : undefined
}

function missingOf(value: unknown): string[] {
  return isRecord(value) && Array.isArray(value.missing)
    ? value.missing.filter((item): item is string => typeof item === 'string')
    : []
}

describe('dsh-alioth tool-alioth', () => {
  it('registers alioth_app_inspect with namespace/app string parameters', async () => {
    const schema = ctx.tools.schemas().find(s => s.name === 'alioth_app_inspect')
    expect(schema).toBeDefined()
    const params = schema!.parameters
    const props = isRecord(params) && isRecord(params.properties) ? params.properties : {}
    expect(Object.keys(props).sort()).toEqual(['app', 'namespace'])
    expect(propType(props.namespace)).toBe('string')
    expect(propType(props.app)).toBe('string')
  })

  it('returns a structured summary for a valid app.json', async () => {
    const result = await callInspect({ namespace: 'Alioth', app: 'ai-i-need-a' })
    expect(result.isError).toBe(false)
    if (result.isError) throw new Error('expected alioth_app_inspect success')
    expect(result.value).toMatchObject({
      code: 'ai-i-need-a',
      namespace: 'Alioth',
      version: '0.1.0',
      minAliothVersion: '10.0.0',
      modules: ['inventory', 'demand'],
      blocks: ['block-list-inventory'],
      routing: { base: '/apps/ai-i-need-a', defaultRoute: '/inventory' },
      navigationGroups: ['系统管理'],
      roles: { defaultRoles: ['admin', 'user'], adminRoles: ['admin'] },
      missing: [],
    })
  })

  it('fails loud when the app.json is missing', async () => {
    const result = await callInspect({ namespace: 'Alioth', app: 'nope' })
    if (!result.isError) throw new Error('expected alioth_app_inspect failure')
    expect(result.error.message).toContain('no app.json at')
  })

  it('fails loud on invalid JSON', async () => {
    const result = await callInspect({ namespace: 'Alioth', app: 'broken' })
    if (!result.isError) throw new Error('expected alioth_app_inspect failure')
    expect(result.error.message).toContain('invalid JSON')
  })

  it('reports missing required fields instead of rejecting the artifact', async () => {
    const result = await callInspect({ namespace: 'Alioth', app: 'incomplete' })
    expect(result.isError).toBe(false)
    if (result.isError) throw new Error('expected alioth_app_inspect success')
    expect([...missingOf(result.value)].sort()).toEqual(['config', 'name', 'namespace', 'version'])
  })

  it('rejects namespace/app values that could escape the Pre-Proc root', async () => {
    const escaped = await callInspect({ namespace: 'Alioth', app: '../..' })
    if (!escaped.isError) throw new Error('expected alioth_app_inspect failure')
    expect(escaped.error.message).toContain('invalid app code')
    const slash = await callInspect({ namespace: 'Alioth', app: 'a/b' })
    if (!slash.isError) throw new Error('expected alioth_app_inspect failure')
    expect(slash.error.message).toContain('invalid app code')
    const badNamespace = await callInspect({ namespace: 'alioth', app: 'ai-i-need-a' })
    if (!badNamespace.isError) throw new Error('expected alioth_app_inspect failure')
    expect(badNamespace.error.message).toContain('invalid namespace')
  })

  it('registers alioth_app_list with an optional namespace parameter', async () => {
    const schema = ctx.tools.schemas().find(s => s.name === 'alioth_app_list')
    expect(schema).toBeDefined()
    const params = schema!.parameters
    const props = isRecord(params) && isRecord(params.properties) ? params.properties : {}
    expect(Object.keys(props).sort()).toEqual(['namespace'])
  })

  it('lists every namespace and its apps, flagging invalid artifacts', async () => {
    const result = await callList({})
    expect(result.isError).toBe(false)
    if (result.isError) throw new Error('expected alioth_app_list success')
    expect(result.value).toMatchObject({
      namespaces: [{
        namespace: 'Alioth',
        apps: [
          { code: 'ai-i-need-a', name: 'ai-i-need-a', version: '0.1.0', modules: ['inventory', 'demand'], valid: true, missing: [] },
          { code: 'broken', name: '', valid: false },
          { code: 'incomplete', name: '', valid: false },
        ],
      }],
    })
  })

  it('filters by namespace and tolerates unknown namespaces', async () => {
    const filtered = await callList({ namespace: 'Alioth' })
    if (filtered.isError) throw new Error(`expected alioth_app_list success: ${filtered.error.message}`)
    const namespaces = (filtered.value as { namespaces: Array<{ apps: Array<{ code: string }> }> }).namespaces
    expect(namespaces).toHaveLength(1)
    expect(namespaces[0]!.apps.map(app => app.code)).toEqual(['ai-i-need-a', 'broken', 'incomplete'])

    const empty = await callList({ namespace: 'Nope' })
    if (empty.isError) throw new Error(`expected alioth_app_list success: ${empty.error.message}`)
    expect(empty.value).toEqual({ namespaces: [{ namespace: 'Nope', apps: [] }] })
  })

  it('rejects an invalid namespace filter', async () => {
    const result = await callList({ namespace: 'alioth' })
    if (!result.isError) throw new Error('expected alioth_app_list failure')
    expect(result.error.message).toContain('invalid namespace')
  })
})

/** A parent Agent backed by a real Session — the approval seam needs `agent.session`. */
function fakeAgent(): Agent & { session: Session } {
  const session = Session.create(SessionId('parent-1'))
  return { id: SessionId('parent-1'), session } as unknown as Agent & { session: Session }
}

// ── alioth_app_write ─────────────────────────────────────────────────────

let writeCounter = 0

function callWrite(args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: CallId(`write-${++writeCounter}`),
    name: 'alioth_app_write',
    arguments: args,
  })
}

describe('dsh-alioth alioth_app_write (bypass)', () => {
  it('writes a validated artifact tree readable back by inspect', async () => {
    const result = await callWrite({
      namespace: 'Alioth',
      code: 'fresh-app',
      name: 'Fresh App',
      modules: [
        { id: 'inventory', name: '库存' },
        { id: 'demand', name: '需求' },
      ],
      blocks: ['block-list-inventory'],
    })
    if (result.isError) throw new Error(`expected alioth_app_write success: ${result.error.message}`)
    expect(result.value).toMatchObject({
      namespace: 'Alioth',
      code: 'fresh-app',
      moduleIds: ['inventory', 'demand'],
      files: expect.arrayContaining(['app.json', 'modules/inventory/module.json', 'extensions/constraints.yaml']),
    })
    // The real proof: the tree exists on disk and inspect reads it back.
    await expect(readFile(path.join(root, 'Alioth', 'Apps', 'fresh-app', 'app.json'), 'utf8')).resolves.toContain('"code": "fresh-app"')

    const inspect = await callInspect({ namespace: 'Alioth', app: 'fresh-app' })
    if (inspect.isError) throw new Error(`expected alioth_app_inspect success: ${inspect.error.message}`)
    expect(inspect.value).toMatchObject({
      code: 'fresh-app',
      modules: ['inventory', 'demand'],
      blocks: ['block-list-inventory'],
      navigationGroups: ['系统管理'],
      missing: [],
    })
  })

  it('refuses to overwrite an existing app', async () => {
    const result = await callWrite({ namespace: 'Alioth', code: 'fresh-app', name: 'Again', modules: [] })
    if (!result.isError) throw new Error('expected alioth_app_write failure')
    expect(result.error.message).toContain('already exists')
  })

  it('rejects invalid module ids before writing', async () => {
    const result = await callWrite({
      namespace: 'Alioth',
      code: 'bad-mod',
      name: 'Bad',
      modules: [{ id: '../evil', name: 'E' }],
    })
    if (!result.isError) throw new Error('expected alioth_app_write failure')
    expect(result.error.message).toContain('invalid module id')
  })
})

describe('dsh-alioth alioth_app_write (approvalMode=required)', () => {
  let approvalCtx: Context
  let approvalRoot: string
  const disposers: Array<() => Promise<void>> = []

  async function bootWithApproval(answer: 'allowed-once' | 'rejected'): Promise<Context> {
    const ctx = new Context()
    const system = await ctx.plugin(SystemPrompt)
    disposers.push(() => system.dispose())
    const tools = await ctx.plugin(ToolRuntime)
    disposers.push(() => tools.dispose())
    ctx.provide('approval')
    ctx.set('approval', {
      request: async () => answer,
    } as never)
    const fiber = await ctx.plugin(tool, { preProcRoot: approvalRoot, approvalMode: 'required' })
    disposers.push(() => fiber.dispose())
    return ctx
  }

  beforeAll(async () => {
    approvalRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-approval-'))
  })

  afterAll(async () => {
    for (const dispose of disposers.reverse()) {
      await dispose().catch(() => {})
    }
    await rm(approvalRoot, { recursive: true, force: true })
  })

  it('fails loud when no ApprovalService is composed', async () => {
    const bare = new Context()
    const system = await bare.plugin(SystemPrompt)
    const tools = await bare.plugin(ToolRuntime)
    try {
      const fiber = await bare.plugin(tool, { preProcRoot: approvalRoot, approvalMode: 'required' })
      const result = await bare.tools.execute({
        signal,
        callId: CallId('approval-missing'),
        name: 'alioth_app_write',
        arguments: { namespace: 'Alioth', code: 'no-approval', name: 'X', modules: [] },
      })
      if (!result.isError) throw new Error('expected alioth_app_write failure')
      expect(result.error.message).toContain('no ApprovalService')
      await fiber.dispose()
    } finally {
      await system.dispose()
      await tools.dispose()
    }
  })

  it('writes when approval grants allowed-once', async () => {
    approvalCtx = await bootWithApproval('allowed-once')
    const result = await approvalCtx.tools.execute({
      signal,
      callId: CallId('approval-grant'),
      name: 'alioth_app_write',
      arguments: { namespace: 'Alioth', code: 'granted-app', name: 'Granted', modules: [{ id: 'm1', name: 'M1' }] },
      agent: fakeAgent() as never,
    })
    if (result.isError) throw new Error(`expected alioth_app_write success: ${result.error.message}`)
    expect(result.value).toMatchObject({ code: 'granted-app' })
  })

  it('writes brand/goal/non_scope through app_write parameters', async () => {
    const result = await ctx.tools.execute({
      signal,
      callId: CallId('write-branded'),
      name: 'alioth_app_write',
      arguments: {
        namespace: 'Alioth', code: 'branded-app', name: 'Branded',
        modules: [{ id: 'm1', name: 'M1' }],
        brand: { primary: '#1677ff', logo: '/assets/logo.png' },
        goal: 'manage inventory',
        nonScope: ['no accounting'],
      },
    })
    if (result.isError) throw new Error(`expected app_write success: ${result.error.message}`)
    const appJson = JSON.parse(await readFile(path.join(root, 'Alioth', 'Apps', 'branded-app', 'app.json'), 'utf8'))
    expect(appJson.brand).toEqual({ primary: '#1677ff', logo: '/assets/logo.png' })
    expect(appJson.goal).toBe('manage inventory')
    expect(appJson.non_scope).toEqual(['no accounting'])
  })

  it('alioth_app_configure merges enrichment fields into an existing app', async () => {
    const result = await ctx.tools.execute({
      signal,
      callId: CallId('configure-app'),
      name: 'alioth_app_configure',
      arguments: {
        namespace: 'Alioth', app: 'ai-i-need-a',
        brand: { primary: '#1677ff' },
        goal: 'business app',
        navigation: [{ group: '库存', icon: 'Inbox', modules: ['inventory'] }],
      },
    })
    if (result.isError) throw new Error(`expected configure success: ${result.error.message}`)
    expect(result.value).toMatchObject({ updated: expect.arrayContaining(['brand.primary', 'goal', 'navigation']) })
    const appJson = JSON.parse(await readFile(path.join(root, 'Alioth', 'Apps', 'ai-i-need-a', 'app.json'), 'utf8'))
    expect(appJson.brand.primary).toBe('#1677ff')
    expect(appJson.goal).toBe('business app')
    expect(appJson.navigation).toEqual([{ group: '库存', icon: 'Inbox', modules: ['inventory'] }])
    // untouched fields survive
    expect(appJson.routing).toEqual({ base: '/apps/ai-i-need-a', defaultRoute: '/inventory' })
  })

  it('alioth_app_configure is idempotent and refuses unknown config', async () => {
    const noop = await ctx.tools.execute({
      signal, callId: CallId('configure-noop'),
      name: 'alioth_app_configure',
      arguments: { namespace: 'Alioth', app: 'ai-i-need-a' },
    })
    if (noop.isError) throw new Error(`expected noop success: ${noop.error.message}`)
    expect(noop.value).toMatchObject({ updated: [] })

    const invalid = await ctx.tools.execute({
      signal, callId: CallId('configure-invalid'),
      name: 'alioth_app_configure',
      arguments: { namespace: 'Alioth', app: 'ai-i-need-a', defaultRoles: 'admin' },
    })
    if (!invalid.isError) throw new Error('expected configure failure on bad types')
  })

  it('alioth_app_configure fails loud when the app does not exist', async () => {
    const result = await ctx.tools.execute({
      signal, callId: CallId('configure-missing'),
      name: 'alioth_app_configure',
      arguments: { namespace: 'Alioth', app: 'no-such-app', goal: 'x' },
    })
    if (!result.isError) throw new Error('expected configure failure')
    expect(result.error.message).toContain('no app.json')
  })

  it('denies the write when approval rejects', async () => {
    approvalCtx = await bootWithApproval('rejected')
    const result = await approvalCtx.tools.execute({
      signal,
      callId: CallId('approval-deny'),
      name: 'alioth_app_write',
      arguments: { namespace: 'Alioth', code: 'denied-app', name: 'Denied', modules: [{ id: 'm1', name: 'M1' }] },
      agent: fakeAgent() as never,
    })
    if (!result.isError) throw new Error('expected alioth_app_write failure')
    expect(result.error.message).toContain('denied by approval')
  })
})

describe('dsh-alioth app growth + discovery', () => {
  it('writes an app with a description through app_write', async () => {
    const result = await callWrite({
      namespace: 'Alioth', code: 'desc-app', name: 'Desc', modules: [{ id: 'm1', name: 'M1' }],
      description: 'one-line description',
    })
    if (result.isError) throw new Error(`expected app_write success: ${result.error.message}`)
    const appJson = JSON.parse(await readFile(path.join(root, 'Alioth', 'Apps', 'desc-app', 'app.json'), 'utf8'))
    expect(appJson.description).toBe('one-line description')
  })

  it('alioth_app_configure adds new modules with artifacts and replaces blocks', async () => {
    const created = await callWrite({
      namespace: 'Alioth', code: 'grow-app', name: 'Grow', modules: [{ id: 'alpha', name: 'A' }],
    })
    if (created.isError) throw new Error(`expected app_write success: ${created.error.message}`)

    const result = await ctx.tools.execute({
      signal,
      callId: CallId('configure-grow'),
      name: 'alioth_app_configure',
      arguments: {
        namespace: 'Alioth', app: 'grow-app',
        modules: [{ id: 'beta', name: 'B', description: 'b-desc' }, { id: 'alpha', name: 'A2' }],
        blocks: ['block-new'],
      },
    })
    if (result.isError) throw new Error(`expected configure success: ${result.error.message}`)
    const updated = (result.value as { updated: string[] }).updated
    expect(updated).toEqual(expect.arrayContaining(['modules.beta', 'navigation', 'config.blocks']))
    expect(updated).not.toContain('modules.alpha')

    // New module got a contract-valid module.json and a Sources dir.
    const moduleJson = JSON.parse(
      await readFile(path.join(root, 'Alioth', 'Apps', 'grow-app', 'modules', 'beta', 'module.json'), 'utf8'))
    expect(moduleJson).toMatchObject({ id: 'beta', namespace: 'Alioth', version: '0.1.0', description: 'b-desc' })
    const sourcesStat = await import('node:fs/promises').then(fs => fs.stat(path.join(root, 'Alioth', 'Apps', 'grow-app', 'Sources', 'Modules', 'beta')))
    expect(sourcesStat.isDirectory()).toBe(true)

    // App.json grew: modules, blocks replaced, navigation keeps every module reachable.
    const appJson = JSON.parse(await readFile(path.join(root, 'Alioth', 'Apps', 'grow-app', 'app.json'), 'utf8'))
    expect(appJson.config).toMatchObject({ modules: ['alpha', 'beta'], blocks: ['block-new'] })
    expect(appJson.navigation).toEqual([{ group: '系统管理', icon: 'Settings', modules: ['alpha', 'beta'] }])

    const inspect = await callInspect({ namespace: 'Alioth', app: 'grow-app' })
    if (inspect.isError) throw new Error(`expected inspect success: ${inspect.error.message}`)
    expect(inspect.value).toMatchObject({ modules: ['alpha', 'beta'], missing: [] })
  })

  it('alioth_app_configure module growth is idempotent', async () => {
    const again = await ctx.tools.execute({
      signal,
      callId: CallId('configure-grow-again'),
      name: 'alioth_app_configure',
      arguments: {
        namespace: 'Alioth', app: 'grow-app',
        modules: [{ id: 'beta', name: 'B' }],
      },
    })
    if (again.isError) throw new Error(`expected configure success: ${again.error.message}`)
    expect((again.value as { updated: string[] }).updated).toEqual([])
  })

  it('alioth_app_configure rejects invalid module ids before writing', async () => {
    const result = await ctx.tools.execute({
      signal,
      callId: CallId('configure-bad-module'),
      name: 'alioth_app_configure',
      arguments: {
        namespace: 'Alioth', app: 'grow-app',
        modules: [{ id: '../evil', name: 'E' }],
      },
    })
    if (!result.isError) throw new Error('expected configure failure')
    expect(result.error.message).toContain('invalid module id')
  })

  it('alioth_app_list sees grown apps', async () => {
    const result = await callList({ namespace: 'Alioth' })
    if (result.isError) throw new Error(`expected alioth_app_list success: ${result.error.message}`)
    const namespaces = (result.value as { namespaces: Array<{ apps: Array<{ code: string }> }> }).namespaces
    const codes = namespaces[0]!.apps.map(app => app.code)
    expect(codes).toEqual(expect.arrayContaining(['grow-app', 'desc-app', 'ai-i-need-a']))
  })

  it('alioth_app_configure sets lifecycle status', async () => {
    const result = await ctx.tools.execute({
      signal,
      callId: CallId('configure-status'),
      name: 'alioth_app_configure',
      arguments: { namespace: 'Alioth', app: 'grow-app', status: 'archived' },
    })
    if (result.isError) throw new Error(`expected configure success: ${result.error.message}`)
    expect((result.value as { updated: string[] }).updated).toEqual(['status'])
    const appJson = JSON.parse(await readFile(path.join(root, 'Alioth', 'Apps', 'grow-app', 'app.json'), 'utf8'))
    expect(appJson.status).toBe('archived')
  })

  it('alioth_app_inspect round-trips every field the other tools can set', async () => {
    const created = await callWrite({
      namespace: 'Alioth', code: 'roundtrip', name: 'Roundtrip', modules: [{ id: 'm1', name: 'M1' }],
      description: 'roundtrip description',
      brand: { primary: '#1677ff', logo: '/logo.png' },
      goal: 'roundtrip goal',
      nonScope: ['no side quests'],
    })
    if (created.isError) throw new Error(`expected app_write success: ${created.error.message}`)
    const configured = await ctx.tools.execute({
      signal,
      callId: CallId('roundtrip-status'),
      name: 'alioth_app_configure',
      arguments: { namespace: 'Alioth', app: 'roundtrip', status: 'developing' },
    })
    if (configured.isError) throw new Error(`expected configure success: ${configured.error.message}`)

    const inspected = await callInspect({ namespace: 'Alioth', app: 'roundtrip' })
    if (inspected.isError) throw new Error(`expected inspect success: ${inspected.error.message}`)
    expect(inspected.value).toMatchObject({
      code: 'roundtrip',
      status: 'developing',
      description: 'roundtrip description',
      goal: 'roundtrip goal',
      nonScope: ['no side quests'],
      brand: { primary: '#1677ff', logo: '/logo.png' },
      missing: [],
    })
  })
})

describe('dsh-alioth alioth_app_delete', () => {
  let deleteCounter = 0

  function callDelete(args: unknown) {
    return ctx.tools.execute({
      signal,
      callId: CallId(`delete-${++deleteCounter}`),
      name: 'alioth_app_delete',
      arguments: args,
    })
  }

  it('refuses without confirm and keeps the tree intact', async () => {
    const created = await callWrite({
      namespace: 'Alioth', code: 'delete-me', name: 'Delete Me', modules: [{ id: 'm1', name: 'M1' }],
    })
    if (created.isError) throw new Error(`expected app_write success: ${created.error.message}`)

    const refused = await callDelete({ namespace: 'Alioth', app: 'delete-me' })
    if (!refused.isError) throw new Error('expected alioth_app_delete failure without confirm')
    expect(refused.error.message).toContain('confirm: true')
    const refusedFalse = await callDelete({ namespace: 'Alioth', app: 'delete-me', confirm: false })
    if (!refusedFalse.isError) throw new Error('expected alioth_app_delete failure on confirm: false')
    expect(refusedFalse.error.message).toContain('confirm: true')

    await expect(readFile(path.join(root, 'Alioth', 'Apps', 'delete-me', 'app.json'), 'utf8')).resolves.toContain('"code": "delete-me"')
  })

  it('removes the whole app tree when confirmed', async () => {
    const result = await callDelete({ namespace: 'Alioth', app: 'delete-me', confirm: true })
    if (result.isError) throw new Error(`expected alioth_app_delete success: ${result.error.message}`)
    expect(result.value).toMatchObject({
      namespace: 'Alioth',
      app: 'delete-me',
      files: expect.arrayContaining(['app.json', 'modules/m1/module.json', 'extensions/constraints.yaml']),
    })
    const stat = await import('node:fs/promises').then(fs => fs.stat(path.join(root, 'Alioth', 'Apps', 'delete-me')))
      .then(() => true, () => false)
    expect(stat).toBe(false)

    const listed = await callList({ namespace: 'Alioth' })
    if (listed.isError) throw new Error(`expected alioth_app_list success: ${listed.error.message}`)
    const codes = (listed.value as { namespaces: Array<{ apps: Array<{ code: string }> }> }).namespaces[0]!.apps.map(app => app.code)
    expect(codes).not.toContain('delete-me')
  })

  it('fails loud for a missing app and rejects escape attempts', async () => {
    const missing = await callDelete({ namespace: 'Alioth', app: 'delete-me', confirm: true })
    if (!missing.isError) throw new Error('expected alioth_app_delete failure')
    expect(missing.error.message).toContain('no app.json at')

    const escaped = await callDelete({ namespace: 'Alioth', app: '../..', confirm: true })
    if (!escaped.isError) throw new Error('expected alioth_app_delete failure')
    expect(escaped.error.message).toContain('invalid app code')
  })

  describe('approvalMode=required', () => {
    const disposers: Array<() => Promise<void>> = []

    async function bootDeleteApproval(answer: 'allowed-once' | 'rejected'): Promise<Context> {
      const approvalCtx = new Context()
      const system = await approvalCtx.plugin(SystemPrompt)
      disposers.push(() => system.dispose())
      const tools = await approvalCtx.plugin(ToolRuntime)
      disposers.push(() => tools.dispose())
      approvalCtx.provide('approval')
      approvalCtx.set('approval', {
        request: async () => answer,
      } as never)
      const fiber = await approvalCtx.plugin(tool, { preProcRoot: root, approvalMode: 'required' })
      disposers.push(() => fiber.dispose())
      return approvalCtx
    }

    it('denies the delete when approval rejects, tree intact', async () => {
      const created = await callWrite({
        namespace: 'Alioth', code: 'delete-approval', name: 'Approval', modules: [{ id: 'm1', name: 'M1' }],
      })
      if (created.isError) throw new Error(`expected app_write success: ${created.error.message}`)
      const approvalCtx = await bootDeleteApproval('rejected')
      const result = await approvalCtx.tools.execute({
        signal,
        callId: CallId('delete-approval-deny'),
        name: 'alioth_app_delete',
        arguments: { namespace: 'Alioth', app: 'delete-approval', confirm: true },
        agent: fakeAgent() as never,
      })
      if (!result.isError) throw new Error('expected alioth_app_delete failure')
      expect(result.error.message).toContain('denied by approval')
      await expect(readFile(path.join(root, 'Alioth', 'Apps', 'delete-approval', 'app.json'), 'utf8')).resolves.toContain('"code": "delete-approval"')
    })

    it('deletes when approval grants allowed-once', async () => {
      const approvalCtx = await bootDeleteApproval('allowed-once')
      const result = await approvalCtx.tools.execute({
        signal,
        callId: CallId('delete-approval-grant'),
        name: 'alioth_app_delete',
        arguments: { namespace: 'Alioth', app: 'delete-approval', confirm: true },
        agent: fakeAgent() as never,
      })
      if (result.isError) throw new Error(`expected alioth_app_delete success: ${result.error.message}`)
      expect(result.value).toMatchObject({ app: 'delete-approval' })
      const stat = await import('node:fs/promises').then(fs => fs.stat(path.join(root, 'Alioth', 'Apps', 'delete-approval')))
        .then(() => true, () => false)
      expect(stat).toBe(false)
    })

    afterAll(async () => {
      for (const dispose of disposers.reverse()) {
        await dispose().catch(() => {})
      }
    })
  })
})
