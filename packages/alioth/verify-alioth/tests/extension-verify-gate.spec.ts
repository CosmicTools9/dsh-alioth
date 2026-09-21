/**
 * 降级验证门的自我解除防护：canonical `extension-verify.json` 只允许存在"对当前产物成立的真实通过"。
 * 这些断言钉住三件事——降级落盘清掉陈旧 canonical、报告绑定产物指纹、以及 deferred 门只能由
 * 一次真实通过解除（旧 canonical 不会让门自我解除）。
 */
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { createDeferredStore } from '../src/deferred.ts'
import { artifactFingerprint } from '../src/closure-audit.ts'
import { verifyExtensions, writeExtensionVerify, type ExtensionVerification } from '../src/extension-verify.ts'

const ALLOWED_FORMS = ['constraints', 'rules', 'statemachines', 'workflows', 'profiles'] as const

async function appWithExtensions(files: Record<string, string>): Promise<string> {
  const appDir = await mkdtemp(path.join(tmpdir(), 'ext-verify-gate-'))
  await writeFile(path.join(appDir, 'app.json'), '{"code":"demo"}\n', 'utf8')
  await mkdir(path.join(appDir, 'extensions'), { recursive: true })
  for (const [name, content] of Object.entries(files)) {
    await writeFile(path.join(appDir, 'extensions', name), content, 'utf8')
  }
  return appDir
}

/** 一条引擎不认的声明文件（`unknown.yaml` 不在加载器的固定文件名分支里）→ 必然 degraded。 */
const UNCOVERED_FIXTURE = { 'unknown.yaml': '- entity: order\n' }
/** 加载器认可且条目带齐必需键 → wired。 */
const WIRED_FIXTURE = { 'constraints.yaml': '- entity: order\n  expression: "qty > 0"\n  message: 数量' }

async function verify(appDir: string): Promise<ExtensionVerification> {
  return verifyExtensions({ app: 'demo', namespace: 'U-bob', appDir, allowedForms: ALLOWED_FORMS })
}

describe('extension verification gate', () => {
  it('binds the report to the artifact fingerprint it judged', async () => {
    const appDir = await appWithExtensions(WIRED_FIXTURE)

    const report = await verify(appDir)

    expect(report.status).toBe('passed')
    expect(report.artifact_fingerprint).toBe(await artifactFingerprint(appDir))
  })

  it('removes a stale canonical report when a later run degrades', async () => {
    const appDir = await appWithExtensions(WIRED_FIXTURE)
    await writeExtensionVerify(appDir, await verify(appDir))
    expect(JSON.parse(await readFile(path.join(appDir, 'extension-verify.json'), 'utf8'))).toBeTruthy()

    await writeFile(path.join(appDir, 'extensions', 'unknown.yaml'), '- entity: order\n', 'utf8')
    const degraded = await verify(appDir)
    expect(degraded.status).toBe('degraded')
    await writeExtensionVerify(appDir, degraded)

    await expect(readFile(path.join(appDir, 'extension-verify.json'), 'utf8')).rejects.toThrow()
    const sidecar = JSON.parse(await readFile(path.join(appDir, 'extension-verify.degraded.json'), 'utf8')) as ExtensionVerification
    expect(sidecar.status).toBe('degraded')
  })

  it('keeps the deferred degraded gate open until a real pass rewrites the canonical report', async () => {
    const appDir = await appWithExtensions(UNCOVERED_FIXTURE)
    const store = createDeferredStore(await mkdtemp(path.join(tmpdir(), 'ext-verify-gate-store-')))
    const degraded = await verify(appDir)
    await writeExtensionVerify(appDir, degraded)
    await store.register({
      id: 'extension-verify',
      sessionId: 's1',
      app: 'demo',
      namespace: 'U-bob',
      reason: `uncovered=${degraded.uncovered}`,
      adjudication: '未覆盖面无法离线执行，需人工确认后方可上线',
      successors: ['重跑扩展验证'],
      createdTs: new Date().toISOString(),
      trigger: {
        kind: 'artifact-json-pointer',
        path: path.join(appDir, 'extension-verify.json'),
        pointer: '/status',
        equals: 'passed',
      },
    })

    // 未决门对**任何会话**的发布都可见（跨会话扫描，不是 sessionId 过滤）。
    expect(await store.unlockDue('s1')).toHaveLength(0)
    expect((await store.all()).filter(item => item.app === 'demo')).toHaveLength(1)

    const passed = await verify(await replaceWithWiredExtensions(appDir))
    await writeExtensionVerify(appDir, passed)

    expect(await store.sweep()).toHaveLength(1)
    expect(await store.open('s1')).toHaveLength(0)
    expect((await store.all()).filter(item => item.app === 'demo')).toHaveLength(0)
  })
})

/** Swap the uncovered declaration for a wired one — a real fix, so the gate may clear. */
async function replaceWithWiredExtensions(appDir: string): Promise<string> {
  await rm(path.join(appDir, 'extensions', 'unknown.yaml'), { force: true })
  await writeFile(path.join(appDir, 'extensions', 'constraints.yaml'), WIRED_FIXTURE['constraints.yaml'], 'utf8')
  return appDir
}
