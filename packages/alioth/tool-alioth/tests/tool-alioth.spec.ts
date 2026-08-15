import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'

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
