/**
 * auth-web-alioth — the Alioth right-Sidebar tab's data plane: the read-only app
 * status projection and the HTTP route that serves it.
 *
 * The projection is asserted against real on-disk artifacts written by the real
 * libraries (`appendClosureVerdict`, `createDeferredStore`, `saveRun`), so the
 * panel cannot drift from what the tools actually leave behind. Corrupt state is
 * a case in its own right: a panel must report it, never throw at the caller.
 */
import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { appendClosureVerdict, createDeferredStore } from '@dsh-alioth/verify-alioth'
import { saveRun } from '@dsh-alioth/skill-alioth'
import { buildAppStatus, type AppStatus } from '../src/app-status.ts'

let root: string
let appDir: string
let dataRoot: string

/** One app workspace: `Pre-Proc/{ns}/Apps/{app}` under the temp root. */
const APP = { namespace: 'U-tester', code: 'default' } as const

async function write(relative: string, contents: string): Promise<void> {
  const file = path.join(appDir, relative)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, 'utf8')
}

/** A contract-valid app.json (the required set the upstream evaluator pins). */
function validAppJson(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    id: 'app-1',
    code: 'default',
    namespace: 'U-tester',
    name: '测试应用',
    version: '1.0.0',
    status: 'developing',
    config: { modules: ['m1', 'm2'], blocks: ['b1', 'b2', 'b3'] },
    permissions: { defaultRoles: [], adminRoles: [] },
    routing: { base: '/apps/default', defaultRoute: '/apps/default/home' },
    navigation: [],
    min_alioth_version: '10.0.0',
    ...overrides,
  })
}

beforeEach(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-appstatus-'))
  appDir = path.join(root, 'Pre-Proc', APP.namespace, 'Apps', APP.code)
  dataRoot = path.join(root, 'data')
  await mkdir(appDir, { recursive: true })
})

afterEach(async () => {
  await rm(root, { recursive: true, force: true })
})

function status(model: { readonly version: string; readonly sourceRef: string } | null = null): Promise<AppStatus> {
  return buildAppStatus({ namespace: APP.namespace, code: APP.code, dir: appDir }, dataRoot, model)
}

describe('buildAppStatus artifacts', () => {
  it('reports a contract-valid app.json with its module and block counts', async () => {
    await write('app.json', validAppJson())
    await write('extensions/nav.yaml', 'form: nav\n')
    await write('extensions/brand.yaml', 'form: brand\n')
    await write('Sources/m1/module.json', '{}')
    await write('Sources/m2/module.json', '{}')
    await write('Sources/loose/readme.md', '')
    await write('prototype.html', '<html></html>')

    const result = await status()

    expect(result.ok).toBe(true)
    expect(result.app).toEqual({ namespace: APP.namespace, code: APP.code, dir: appDir })
    expect(result.artifacts.appJson).toMatchObject({
      present: true,
      valid: true,
      errors: [],
      name: '测试应用',
      status: 'developing',
      version: '1.0.0',
      modules: 2,
      blocks: 3,
    })
    expect(result.artifacts.extensions).toEqual({ files: 2, verification: 'absent' })
    expect(result.artifacts.sources.dirs).toBe(3)
    expect(result.artifacts.modulesOnDisk).toBe(2)
    expect(result.artifacts.prototype.html).toBe(true)
  })

  it('reports contract violations instead of failing the whole panel', async () => {
    // `permissions`/`routing`/`navigation` are required by the app contract.
    await write('app.json', validAppJson({ permissions: undefined, routing: undefined, navigation: undefined }))

    const result = await status()

    expect(result.artifacts.appJson.present).toBe(true)
    expect(result.artifacts.appJson.valid).toBe(false)
    expect(result.artifacts.appJson.errors.length).toBeGreaterThan(0)
  })

  it('separates an absent app.json from an unparseable one', async () => {
    const absent = await status()
    expect(absent.artifacts.appJson).toEqual({
      present: false, valid: false, errors: [], name: null, status: null, version: null, minAliothVersion: null, modules: 0, blocks: 0,
    })

    await write('app.json', '{ not json')
    const broken = await status()
    expect(broken.artifacts.appJson.present).toBe(true)
    expect(broken.artifacts.appJson.valid).toBe(false)
    expect(broken.artifacts.appJson.errors[0]).toContain('无法解析')
  })

  it('reads extension verification from the canonical report, then its degraded sibling', async () => {
    await write('extension-verify.json', JSON.stringify({ status: 'passed' }))
    expect((await status()).artifacts.extensions.verification).toBe('passed')

    // A degraded run deletes the canonical file — the sibling is the evidence.
    await rm(path.join(appDir, 'extension-verify.json'))
    await write('extension-verify.degraded.json', JSON.stringify({ status: 'degraded' }))
    expect((await status()).artifacts.extensions.verification).toBe('degraded')

    await rm(path.join(appDir, 'extension-verify.degraded.json'))
    expect((await status()).artifacts.extensions.verification).toBe('absent')
  })
})

describe('buildAppStatus pipeline', () => {
  it('reports the persisted run position and the last completed step', async () => {
    await saveRun(path.join(dataRoot, 'workflows'), { namespace: APP.namespace, app: APP.code }, {
      adapter: { tracks: [] } as never,
      position: { trackIndex: 1, stepIndex: 2 },
      completed: ['app-creation', 'semantic-audit'],
    })

    const result = await status()

    expect(result.pipeline.run).toEqual({
      present: true, trackIndex: 1, stepIndex: 2, completed: 2, lastCompleted: 'semantic-audit',
    })
  })

  it('reports no run without creating one (a read must never start a run)', async () => {
    const result = await status()

    expect(result.pipeline.run).toEqual({ present: false })
    // The run file the panel read must not exist: `loadRun` would have written one.
    await expect(readFile(path.join(dataRoot, 'workflows', APP.namespace, APP.code, 'run-state.json')))
      .rejects.toThrow(/ENOENT/)
  })

  it('reports a corrupt run file as data', async () => {
    await write('x', '')
    await mkdir(path.join(dataRoot, 'workflows', APP.namespace, APP.code), { recursive: true })
    await writeFile(path.join(dataRoot, 'workflows', APP.namespace, APP.code, 'run-state.json'), '{ not json')

    const result = await status()

    expect(result.pipeline.run.present).toBe(true)
    expect(result.pipeline.run).toHaveProperty('error')
  })

  it('shows pending gates for this app plus unattributed ones, hiding other apps\'', async () => {
    const store = createDeferredStore(dataRoot)
    const base = {
      sessionId: 'session-1',
      adjudication: '人工确认',
      trigger: { kind: 'artifact-exists' as const, path: 'app.json' },
      successors: [],
      createdTs: '2026-09-24T00:00:00.000Z',
    }
    await store.register({ ...base, id: 'g-mine', app: APP.code, namespace: APP.namespace, reason: '扩展未装配' })
    await store.register({ ...base, id: 'g-any', reason: '未归属的降级门' })
    await store.register({ ...base, id: 'g-other', app: 'other-app', namespace: APP.namespace, reason: '别的应用' })

    const result = await status()

    expect(result.pipeline.deferred.open).toBe(2)
    // Order is the store's business; membership and provenance are ours.
    expect(result.pipeline.deferred.items.map(item => item.id).sort()).toEqual(['g-any', 'g-mine'])
    expect(result.pipeline.deferred.items.find(item => item.id === 'g-mine'))
      .toMatchObject({ app: APP.code, reason: '扩展未装配' })
    expect(result.pipeline.deferred.items.find(item => item.id === 'g-any')).toMatchObject({ app: null })
  })

  it('reports the latest closure verdict written by the real audit writer', async () => {
    await appendClosureVerdict(appDir, {
      app: APP.code,
      namespace: APP.namespace,
      verdict: 'rejected',
      fingerprint: 'sha256:abc',
      findings: [],
      evidence: [],
    })
    await appendClosureVerdict(appDir, {
      app: APP.code,
      namespace: APP.namespace,
      verdict: 'approved',
      fingerprint: 'sha256:abc',
      findings: [],
      evidence: [],
    })

    const result = await status()

    expect(result.pipeline.closure).toMatchObject({ present: true, verdict: 'approved', seq: 2 })
    expect(result.pipeline.closure).toHaveProperty('at')
  })
})

describe('buildAppStatus model dependency', () => {
  it('names the deployment model and resolves the app\'s declared minimum against it', async () => {
    await write('app.json', validAppJson({ min_alioth_version: '10.0.0' }))
    const result = await status({ version: '10.0.34', sourceRef: 'local' })

    expect(result.model).toEqual({ version: '10.0.34', sourceRef: 'local' })
    expect(result.artifacts.appJson.minAliothVersion).toBe('10.0.0')
    expect(result.dependency).toEqual({ declared: '10.0.0', model: '10.0.34', satisfied: true })
  })

  it('flags a declared minimum the deployment does not provide', async () => {
    await write('app.json', validAppJson({ min_alioth_version: '10.1.0' }))
    const result = await status({ version: '10.0.34', sourceRef: 'local' })

    expect(result.dependency).toEqual({ declared: '10.1.0', model: '10.0.34', satisfied: false })
  })

  it('reports an unknown deployment model instead of guessing one', async () => {
    await write('app.json', validAppJson())
    const result = await status(null)

    expect(result.model).toBeNull()
    // Undecidable is reported as unsatisfied: the panel never claims a dependency is met
    // when one side of the comparison is missing.
    expect(result.dependency).toEqual({ declared: '10.0.0', model: '', satisfied: false })
  })

  it('treats a non-release declared value as undecidable', async () => {
    await write('app.json', validAppJson({ min_alioth_version: 'v10-latest' }))
    const result = await status({ version: '10.0.34', sourceRef: 'local' })

    expect(result.dependency.satisfied).toBe(false)
  })

  it('normalises a v-prefixed publication tag to the form artifacts carry', async () => {
    await write('app.json', validAppJson())
    const result = await status({ version: 'v10.0.34', sourceRef: 'tag:v10.0.34' })

    expect(result.model).toEqual({ version: '10.0.34', sourceRef: 'tag:v10.0.34' })
    expect(result.dependency.model).toBe('10.0.34')
  })
})
