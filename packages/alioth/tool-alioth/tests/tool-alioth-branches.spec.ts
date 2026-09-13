/**
 * Branch-coverage spec for the five app tools (`alioth_app_list`,
 * `alioth_app_inspect`, `alioth_app_write`, `alioth_app_configure`,
 * `alioth_app_delete`). Same wiring soul as `tool-alioth.spec.ts`: a real
 * Context with the real ToolRuntime over a real temp Pre-Proc tree — only the
 * approval seam is a stand-in. Each test seeds one hand-edited artifact shape
 * and asserts what the tool does with it.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { type ToolCallView, type ToolExecutionResult } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'

import * as tool from '../src/index.ts'

const signal = new AbortController().signal

/** Contract-valid app.json; each test overrides exactly the field under test. */
const BASE_APP = {
  id: '946462018160351133',
  code: 'base-app',
  namespace: 'Alioth',
  name: 'Base App',
  version: '0.1.0',
  config: { modules: ['inventory'], blocks: ['block-list-inventory'] },
  permissions: { defaultRoles: ['admin', 'user'], adminRoles: ['admin'] },
  routing: { base: '/apps/base-app', defaultRoute: '/inventory' },
  navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory'] }],
  min_alioth_version: '10.0.0',
}

let root: string
let ctx: Context
let counter = 0

/** One app.json variant of the valid base; `overrides` win key by key. */
function appSpec(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { ...BASE_APP, ...overrides }
}

async function seedApp(namespace: string, app: string, appJson: unknown): Promise<void> {
  const dir = path.join(root, namespace, 'Apps', app)
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, 'app.json'), typeof appJson === 'string' ? appJson : JSON.stringify(appJson))
}

function call(name: string, args: unknown): Promise<ToolExecutionResult> {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`${name}-${++counter}`),
    name,
    arguments: args,
  })
}

/** The failure message of a call that must fail. */
function errorOf(result: ToolExecutionResult): string {
  if (!result.isError) throw new Error('expected the tool call to fail')
  return result.error.message
}

/** The object `value` of a call that must succeed; `{}` for a missing value. */
function valueOf(result: ToolExecutionResult): Record<string, unknown> {
  if (result.isError) throw new Error(`expected the tool call to succeed: ${result.error.message}`)
  const value = result.value
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
}

/** Object entries of an array-valued field, ignoring anything else. */
function recordsOf(value: unknown): Array<Record<string, unknown>> {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is Record<string, unknown> =>
    typeof item === 'object' && item !== null && !Array.isArray(item))
}

/** The salient raw input of a call presented as a generic card. */
function rawInputOf(view: ToolCallView | undefined): unknown {
  if (view?.card !== 'generic') throw new Error(`expected a generic card view, got ${String(view?.card)}`)
  return view.rawInput
}

async function readAppJson(namespace: string, app: string): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(path.join(root, namespace, 'Apps', app, 'app.json'), 'utf8')) as Record<string, unknown>
}

beforeAll(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-branches-'))
  await seedApp('Alioth', 'base-app', BASE_APP)
  ctx = new Context()
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(tool, { preProcRoot: root })
})

afterAll(async () => {
  await rm(root, { recursive: true, force: true })
})

describe('dsh-alioth alioth_app_list branch behaviour', () => {
  it('falls back to the directory name when app.json carries no code string', async () => {
    const { code: _code, ...withoutCode } = BASE_APP
    await seedApp('Alioth', 'nocode', { ...withoutCode, name: 'No Code' })
    const result = await call('alioth_app_list', { namespace: 'Alioth' })
    const apps = recordsOf(recordsOf(valueOf(result).namespaces)[0]?.apps)
    expect(apps.find(app => app.code === 'nocode')).toMatchObject({ code: 'nocode', name: 'No Code', valid: false })
  })

  it('sorts namespaces and their apps alphabetically across the whole tree', async () => {
    await seedApp('Zeta', 'z-app', appSpec({ code: 'z-app', namespace: 'Zeta', name: 'Z' }))
    await seedApp('Alpha', 'b-app', appSpec({ code: 'b-app', namespace: 'Alpha', name: 'B' }))
    await seedApp('Alpha', 'a-app', appSpec({ code: 'a-app', namespace: 'Alpha', name: 'A' }))
    const namespaces = recordsOf(valueOf(await call('alioth_app_list', {})).namespaces)
    // Seeded creation order was Alioth, Zeta, Alpha — the listing must sort.
    expect(namespaces.map(ns => ns.namespace)).toEqual(['Alioth', 'Alpha', 'Zeta'])
    expect(recordsOf(namespaces.find(ns => ns.namespace === 'Alpha')?.apps).map(app => app.code))
      .toEqual(['a-app', 'b-app'])
    expect(recordsOf(namespaces.find(ns => ns.namespace === 'Zeta')?.apps).map(app => app.code))
      .toEqual(['z-app'])
  })

  it('presents an unfiltered list call and a namespace-filtered one differently', () => {
    const definition = ctx.tools.get('alioth_app_list')
    const unfiltered = definition?.presentCall?.({})
    expect(unfiltered).toMatchObject({ card: 'generic', title: 'List Alioth apps', kind: 'other' })
    expect(rawInputOf(unfiltered)).toEqual({})
    const filtered = definition?.presentCall?.({ namespace: 'Alioth' })
    expect(filtered).toMatchObject({ title: 'List Alioth apps in Alioth' })
    expect(rawInputOf(filtered)).toEqual({ namespace: 'Alioth' })
  })
})

describe('dsh-alioth alioth_app_inspect branch behaviour', () => {
  it('rejects an app.json whose JSON is not an object', async () => {
    await seedApp('Alioth', 'json-array', '[{"id": "x"}]')
    expect(errorOf(await call('alioth_app_inspect', { namespace: 'Alioth', app: 'json-array' })))
      .toContain('must be a JSON object')
  })

  it('reports a read failure that is not a missing file', async () => {
    // `app.json` exists but is a directory: readFile fails with EISDIR, not ENOENT.
    await mkdir(path.join(root, 'Alioth', 'Apps', 'app-json-dir', 'app.json'), { recursive: true })
    expect(errorOf(await call('alioth_app_inspect', { namespace: 'Alioth', app: 'app-json-dir' })))
      .toContain('failed to read')
  })

  it('labels navigation entries that carry no usable group name', async () => {
    await seedApp('Alioth', 'odd-nav', appSpec({ navigation: ['nope', { icon: 'Inbox' }, { group: 7 }] }))
    expect(valueOf(await call('alioth_app_inspect', { namespace: 'Alioth', app: 'odd-nav' })).navigationGroups)
      .toEqual(['<unnamed>', '<unnamed>', '<unnamed>'])
  })

  it('reports an empty code when the app.json code field is absent', async () => {
    expect(valueOf(await call('alioth_app_inspect', { namespace: 'Alioth', app: 'nocode' })))
      .toMatchObject({ code: '', name: 'No Code', missing: ['code'] })
  })
})

describe('dsh-alioth alioth_app_write branch behaviour', () => {
  it('rejects a namespace that is not capitalized before touching the filesystem', async () => {
    expect(errorOf(await call('alioth_app_write', { namespace: 'alioth', code: 'x-app', name: 'X', modules: [] })))
      .toContain('invalid namespace')
  })

  it('rejects an app code containing a separator', async () => {
    expect(errorOf(await call('alioth_app_write', { namespace: 'Alioth', code: 'x/app', name: 'X', modules: [] })))
      .toContain('invalid app code')
  })

  it('honours version, navigation, roles and routing overrides instead of the defaults', async () => {
    const result = await call('alioth_app_write', {
      namespace: 'Alioth',
      code: 'wired-app',
      name: 'Wired',
      modules: [{ id: 'm1', name: 'M1' }],
      version: '2.3.4',
      navigation: [{ group: '库存', icon: 'Inbox', modules: ['m1'] }],
      defaultRoles: ['user'],
      adminRoles: ['ops'],
      base: '/apps/wired-base',
      defaultRoute: '/home',
    })
    expect(valueOf(result)).toMatchObject({ code: 'wired-app' })
    expect(await readAppJson('Alioth', 'wired-app')).toMatchObject({
      version: '2.3.4',
      permissions: { defaultRoles: ['user'], adminRoles: ['ops'] },
      routing: { base: '/apps/wired-base', defaultRoute: '/home' },
      navigation: [{ group: '库存', icon: 'Inbox', modules: ['m1'] }],
    })
    // The owning app's version flows into the generated module artifact.
    const moduleJson = JSON.parse(
      await readFile(path.join(root, 'Alioth', 'Apps', 'wired-app', 'modules', 'm1', 'module.json'), 'utf8'))
    expect(moduleJson).toMatchObject({ id: 'm1', namespace: 'Alioth', version: '2.3.4' })
  })

  it('refuses to persist a generated app.json that fails the app contract', async () => {
    const message = errorOf(await call('alioth_app_write', {
      namespace: 'Alioth', code: 'bad-version', name: 'Bad', modules: [], version: '2.3',
    }))
    expect(message).toContain('generated app.json fails the app contract')
    expect(existsSync(path.join(root, 'Alioth', 'Apps', 'bad-version'))).toBe(false)
  })

  it('refuses to persist a generated module.json that fails the module contract', async () => {
    const message = errorOf(await call('alioth_app_write', {
      namespace: 'Alioth', code: 'bad-module', name: 'Bad', modules: [{ id: 'm1', name: '' }],
    }))
    expect(message).toContain('generated module.json for m1 fails the module contract')
    expect(existsSync(path.join(root, 'Alioth', 'Apps', 'bad-module'))).toBe(false)
  })
})

describe('dsh-alioth alioth_app_write approval seam branch behaviour', () => {
  it('defaults to bypass when a deployment registers apply() without approvalMode', async () => {
    const bareRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-apply-'))
    const bare = new Context()
    const system = await bare.plugin(SystemPrompt)
    const tools = await bare.plugin(ToolRuntime)
    try {
      tool.apply(bare, { preProcRoot: bareRoot })
      const result = await bare.tools.execute({
        signal,
        callId: ToolCallId('apply-default-bypass'),
        name: 'alioth_app_write',
        arguments: { namespace: 'Alioth', code: 'apply-app', name: 'Apply', modules: [{ id: 'm1', name: 'M1' }] },
      })
      expect(result.isError).toBe(false)
      expect(existsSync(path.join(bareRoot, 'Alioth', 'Apps', 'apply-app', 'app.json'))).toBe(true)
    } finally {
      await system.dispose()
      await tools.dispose()
      await rm(bareRoot, { recursive: true, force: true })
    }
  })

  it('fails loud when approvalMode=required but the call carries no agent to ask', async () => {
    const approvalCtx = new Context()
    const system = await approvalCtx.plugin(SystemPrompt)
    const tools = await approvalCtx.plugin(ToolRuntime)
    approvalCtx.provide('approval')
    approvalCtx.set('approval', { request: async () => 'allowed-once' } as never)
    const fiber = await approvalCtx.plugin(tool, { preProcRoot: root, approvalMode: 'required' })
    try {
      const result = await approvalCtx.tools.execute({
        signal,
        callId: ToolCallId('write-without-agent'),
        name: 'alioth_app_write',
        arguments: { namespace: 'Alioth', code: 'agentless', name: 'Agentless', modules: [{ id: 'm1', name: 'M1' }] },
      })
      expect(errorOf(result)).toContain('no agent to route approval')
      expect(existsSync(path.join(root, 'Alioth', 'Apps', 'agentless'))).toBe(false)
    } finally {
      await fiber.dispose()
      await system.dispose()
      await tools.dispose()
    }
  })
})

describe('dsh-alioth alioth_app_configure merge branch behaviour', () => {
  it('merges a brand key into an app that already declares a brand object', async () => {
    await seedApp('Alioth', 'cfg-brand', appSpec({ code: 'cfg-brand', brand: { primary: '#111111' } }))
    const result = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-brand', brand: { logo: '/assets/logo.png' },
    })
    expect(valueOf(result).updated).toEqual(['brand.logo'])
    expect((await readAppJson('Alioth', 'cfg-brand')).brand).toEqual({ primary: '#111111', logo: '/assets/logo.png' })
  })

  it('leaves the app untouched when the brand object carries no values', async () => {
    await seedApp('Alioth', 'cfg-empty-brand', appSpec({ code: 'cfg-empty-brand' }))
    const result = await call('alioth_app_configure', { namespace: 'Alioth', app: 'cfg-empty-brand', brand: {} })
    expect(valueOf(result).updated).toEqual([])
    expect('brand' in await readAppJson('Alioth', 'cfg-empty-brand')).toBe(false)
  })

  it('creates the permissions block from nothing when both role lists are given', async () => {
    const { permissions: _permissions, routing: _routing, ...withoutRoleRouting } = BASE_APP
    await seedApp('Alioth', 'cfg-roles-new', { ...withoutRoleRouting, code: 'cfg-roles-new' })
    const result = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-roles-new',
      defaultRoles: ['user'], adminRoles: ['ops'],
      base: '/apps/cfg-roles-new', defaultRoute: '/inventory',
    })
    expect(valueOf(result).updated).toEqual(
      expect.arrayContaining(['permissions.defaultRoles', 'permissions.adminRoles', 'routing.base', 'routing.defaultRoute']))
    expect(await readAppJson('Alioth', 'cfg-roles-new')).toMatchObject({
      permissions: { defaultRoles: ['user'], adminRoles: ['ops'] },
      routing: { base: '/apps/cfg-roles-new', defaultRoute: '/inventory' },
    })
  })

  it('replaces one role list while keeping the other', async () => {
    await seedApp('Alioth', 'cfg-roles', appSpec({ code: 'cfg-roles' }))
    const first = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-roles', defaultRoles: ['viewer'],
    })
    expect(valueOf(first).updated).toEqual(['permissions.defaultRoles'])
    expect((await readAppJson('Alioth', 'cfg-roles')).permissions)
      .toEqual({ defaultRoles: ['viewer'], adminRoles: ['admin'] })

    const second = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-roles', adminRoles: ['sre'],
    })
    expect(valueOf(second).updated).toEqual(['permissions.adminRoles'])
    expect((await readAppJson('Alioth', 'cfg-roles')).permissions)
      .toEqual({ defaultRoles: ['viewer'], adminRoles: ['sre'] })
  })

  it('replaces one routing key at a time and keeps its sibling', async () => {
    await seedApp('Alioth', 'cfg-routing', appSpec({ code: 'cfg-routing' }))
    const base = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-routing', base: '/apps/cfg-routing-v2',
    })
    expect(valueOf(base).updated).toEqual(['routing.base'])
    expect((await readAppJson('Alioth', 'cfg-routing')).routing)
      .toEqual({ base: '/apps/cfg-routing-v2', defaultRoute: '/inventory' })

    const route = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-routing', defaultRoute: '/reports',
    })
    expect(valueOf(route).updated).toEqual(['routing.defaultRoute'])
    expect((await readAppJson('Alioth', 'cfg-routing')).routing)
      .toEqual({ base: '/apps/cfg-routing-v2', defaultRoute: '/reports' })
  })

  it('writes non-scope statements', async () => {
    await seedApp('Alioth', 'cfg-nonscope', appSpec({ code: 'cfg-nonscope' }))
    const result = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-nonscope', nonScope: ['no accounting', 'no payroll'],
    })
    expect(valueOf(result).updated).toEqual(['non_scope'])
    expect((await readAppJson('Alioth', 'cfg-nonscope')).non_scope).toEqual(['no accounting', 'no payroll'])
  })
})

describe('dsh-alioth alioth_app_configure growth branch behaviour', () => {
  it('gives a navigation-less app the default group over every module', async () => {
    const { navigation: _navigation, ...withoutNav } = BASE_APP
    await seedApp('Alioth', 'cfg-no-nav', { ...withoutNav, code: 'cfg-no-nav' })
    const result = await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-no-nav', modules: [{ id: 'beta', name: 'Beta' }],
    })
    expect(valueOf(result).updated).toEqual(expect.arrayContaining(['modules.beta', 'navigation']))
    expect(await readAppJson('Alioth', 'cfg-no-nav')).toMatchObject({
      config: { modules: ['inventory', 'beta'] },
      navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'beta'] }],
    })
  })

  it('replaces a navigation that is not an array with the default group', async () => {
    await seedApp('Alioth', 'cfg-object-nav', appSpec({ code: 'cfg-object-nav', navigation: {} }))
    await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-object-nav', modules: [{ id: 'beta', name: 'Beta' }],
    })
    expect((await readAppJson('Alioth', 'cfg-object-nav')).navigation)
      .toEqual([{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'beta'] }])
  })

  it('appends a new module to the first group when no 系统管理 group exists', async () => {
    await seedApp('Alioth', 'cfg-first-group', appSpec({
      code: 'cfg-first-group',
      navigation: [{ group: '库存', icon: 'Inbox', modules: ['inventory'] }],
    }))
    await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-first-group', modules: [{ id: 'beta', name: 'Beta' }],
    })
    expect((await readAppJson('Alioth', 'cfg-first-group')).navigation)
      .toEqual([{ group: '库存', icon: 'Inbox', modules: ['inventory', 'beta'] }])
  })

  it('does not duplicate a module already listed in the navigation group', async () => {
    await seedApp('Alioth', 'cfg-dup-nav', appSpec({
      code: 'cfg-dup-nav',
      navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'beta'] }],
    }))
    await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-dup-nav', modules: [{ id: 'beta', name: 'Beta' }],
    })
    expect(await readAppJson('Alioth', 'cfg-dup-nav')).toMatchObject({
      config: { modules: ['inventory', 'beta'] },
      navigation: [{ group: '系统管理', icon: 'Settings', modules: ['inventory', 'beta'] }],
    })
  })

  it('generates module artifacts with default owner fields when app.json lacks version and namespace', async () => {
    const { version: _version, namespace: _namespace, ...withoutOwnerFields } = BASE_APP
    await seedApp('Alioth', 'cfg-owner-fallback', { ...withoutOwnerFields, code: 'cfg-owner-fallback' })
    const message = errorOf(await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-owner-fallback', modules: [{ id: 'm1', name: 'M1' }],
    }))
    // The module artifact was grown with the fallbacks, then the app contract
    // gate refused to persist the app.json that still lacks owner fields.
    expect(message).toContain('fails the app contract')
    const moduleJson = JSON.parse(
      await readFile(path.join(root, 'Alioth', 'Apps', 'cfg-owner-fallback', 'modules', 'm1', 'module.json'), 'utf8'))
    expect(moduleJson).toMatchObject({ id: 'm1', namespace: 'Alioth', version: '0.1.0' })
    expect('version' in await readAppJson('Alioth', 'cfg-owner-fallback')).toBe(false)
  })

  it('refuses to grow modules when the app.json has no config object', async () => {
    const { config: _config, ...withoutConfig } = BASE_APP
    await seedApp('Alioth', 'cfg-no-config-modules', { ...withoutConfig, code: 'cfg-no-config-modules' })
    expect(errorOf(await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-no-config-modules', modules: [{ id: 'beta', name: 'Beta' }],
    }))).toContain('fails the app contract')
  })

  it('refuses to replace blocks when the app.json has no config object', async () => {
    const { config: _config, ...withoutConfig } = BASE_APP
    await seedApp('Alioth', 'cfg-no-config-blocks', { ...withoutConfig, code: 'cfg-no-config-blocks' })
    expect(errorOf(await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-no-config-blocks', blocks: ['block-x'],
    }))).toContain('fails the app contract')
  })

  it('refuses to grow a module whose generated contract fails', async () => {
    await seedApp('Alioth', 'cfg-bad-version', appSpec({ code: 'cfg-bad-version', version: '1.0' }))
    const message = errorOf(await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-bad-version', modules: [{ id: 'm1', name: 'M1' }],
    }))
    expect(message).toContain('generated module.json for m1 fails the module contract')
    expect(existsSync(path.join(root, 'Alioth', 'Apps', 'cfg-bad-version', 'modules'))).toBe(false)
  })

  it('refuses to grow an app whose navigation entries are not group objects', async () => {
    await seedApp('Alioth', 'cfg-junk-nav', appSpec({ code: 'cfg-junk-nav', navigation: ['junk'] }))
    expect(errorOf(await call('alioth_app_configure', {
      namespace: 'Alioth', app: 'cfg-junk-nav', modules: [{ id: 'beta', name: 'Beta' }],
    }))).toContain('fails the app contract')
    expect((await readAppJson('Alioth', 'cfg-junk-nav')).navigation).toEqual(['junk'])
  })
})

describe('dsh-alioth alioth_app_delete branch behaviour', () => {
  it('rejects a namespace that is not capitalized before resolving the tree', async () => {
    expect(errorOf(await call('alioth_app_delete', { namespace: 'alioth', app: 'base-app', confirm: true })))
      .toContain('invalid namespace')
    expect(existsSync(path.join(root, 'Alioth', 'Apps', 'base-app'))).toBe(true)
  })
})

describe('dsh-alioth Pre-Proc containment guard', () => {
  // `path.resolve` collapses `//`, so a Pre-Proc root that *is* the filesystem
  // root makes every resolved app path fail the containment prefix check.
  let fsRootCtx: Context
  const disposers: Array<() => Promise<void>> = []

  beforeAll(async () => {
    fsRootCtx = new Context()
    const system = await fsRootCtx.plugin(SystemPrompt)
    disposers.push(() => system.dispose())
    const tools = await fsRootCtx.plugin(ToolRuntime)
    disposers.push(() => tools.dispose())
    const fiber = await fsRootCtx.plugin(tool, { preProcRoot: '/', approvalMode: 'bypass' })
    disposers.push(() => fiber.dispose())
  })

  afterAll(async () => {
    for (const dispose of disposers.reverse()) await dispose().catch(() => {})
  })

  function fsCall(name: string, args: unknown): Promise<ToolExecutionResult> {
    return fsRootCtx.tools.execute({
      signal,
      callId: ToolCallId(`fs-root-${name}-${++counter}`),
      name,
      arguments: args,
    })
  }

  it('refuses to inspect a path resolved outside the Pre-Proc root', async () => {
    expect(errorOf(await fsCall('alioth_app_inspect', { namespace: 'Alioth', app: 'base-app' })))
      .toContain('path escapes preProcRoot')
  })

  it('refuses to write a path resolved outside the Pre-Proc root', async () => {
    expect(errorOf(await fsCall('alioth_app_write', {
      namespace: 'Alioth', code: 'escapee', name: 'Escapee', modules: [],
    }))).toContain('path escapes preProcRoot')
  })

  it('refuses to configure a path resolved outside the Pre-Proc root', async () => {
    expect(errorOf(await fsCall('alioth_app_configure', { namespace: 'Alioth', app: 'base-app', goal: 'x' })))
      .toContain('path escapes preProcRoot')
  })

  it('refuses to delete a path resolved outside the Pre-Proc root', async () => {
    expect(errorOf(await fsCall('alioth_app_delete', { namespace: 'Alioth', app: 'base-app', confirm: true })))
      .toContain('path escapes preProcRoot')
  })
})

describe('dsh-alioth app tool presentation', () => {
  it('titles each app tool call with its namespace and app target', () => {
    expect(ctx.tools.get('alioth_app_inspect')?.presentCall?.({ namespace: 'Alioth', app: 'base-app' }))
      .toMatchObject({ title: 'Inspect Alioth app Alioth/base-app', kind: 'other' })
    expect(ctx.tools.get('alioth_app_write')?.presentCall?.({
      namespace: 'Alioth', code: 'pc-app', name: 'PC', modules: [{ id: 'm1', name: 'M1' }],
    })).toMatchObject({ title: 'Write Alioth app Alioth/pc-app', kind: 'other' })
    expect(ctx.tools.get('alioth_app_configure')?.presentCall?.({ namespace: 'Alioth', app: 'base-app' }))
      .toMatchObject({ title: 'Configure Alioth app Alioth/base-app', kind: 'other' })
    const deleteView = ctx.tools.get('alioth_app_delete')?.presentCall?.({ namespace: 'Alioth', app: 'base-app' })
    expect(deleteView).toMatchObject({ title: 'Delete Alioth app Alioth/base-app', kind: 'other' })
    expect(rawInputOf(deleteView)).toEqual({ namespace: 'Alioth', app: 'base-app' })
  })
})
