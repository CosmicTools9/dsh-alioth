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

/** A valid Alioth app.json mirroring `Pre-Proc/Alioth/Apps/ai-i-need-a/app.json`. */
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
