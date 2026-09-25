import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { generateExtensions, type ExtensionPlanInput } from '@dsh-alioth/gen-alioth'
import { EXTENSION_FORMS, verifyExtensions } from '@dsh-alioth/verify-alioth'

/**
 * 生成 ↔ 加载器契约的一致性（上游 `compose_app` 的产物必须真的能被引擎装配）。
 *
 * 本用例存在的理由：生成器与验证器是两套独立实现（gen-alioth 生成、verify-alioth 按 vendored
 * Gateway `ExtensionLoader` 的固定文件名/顶层形状/必需键验证）。只有把生成结果**喂给验证器**，
 * 才证明「计划驱动的扩展」不是自说自话——字段名、枚举拼写、顶层形状任何一处漂移都会在这里红。
 */
const opened: string[] = []
afterEach(async () => {
  await Promise.all(opened.splice(0).map(dir => rm(dir, { recursive: true, force: true })))
})

/** App dir with the plan-derived extensions (plus a minimal app.json for the fingerprint). */
async function appDirWith(extensions: Readonly<Record<string, string>>): Promise<string> {
  const root = await mkdtemp(path.join(tmpdir(), 'conformance-'))
  opened.push(root)
  const appDir = path.join(root, 'Apps', 'demo')
  await mkdir(path.join(appDir, 'extensions'), { recursive: true })
  await writeFile(path.join(appDir, 'app.json'), '{"code":"demo"}\n')
  for (const [file, body] of Object.entries(extensions)) {
    await writeFile(path.join(appDir, 'extensions', file), body)
  }
  return appDir
}

const verify = (appDir: string) => verifyExtensions({ app: 'demo', namespace: 'U-conformance', appDir, allowedForms: EXTENSION_FORMS })

const PLAN: ExtensionPlanInput = {
  constraints: [{ entity: 'zc_id_unit', expression: 'qty > 0', message: '数量须为正', level: 'error' }],
  businessRules: [{ entity: 'zc_id_unit', ruleName: 'discount', trigger: 'onCreate', condition: 'qty > 100', action: 'discount_rate = 0.15' }],
  workflowSteps: ['reserve'],
  modules: ['inventory'],
  ontologyModelJson: JSON.stringify({
    transaction_lifecycle: {
      name: 'zc_id_bill',
      phases: [{ id: 'p1', name: 'Created' }, { id: 'p2', name: 'Confirmed' }],
      transitions: [{ trigger_event: 'confirm', from_phase: 'p1', to_phase: 'p2', guard_conditions: [] }],
    },
  }),
}

describe('plan-derived extensions are loader-conformant', () => {
  it('every generated declaration is covered by the Gateway loader contract', async () => {
    const appDir = await appDirWith(generateExtensions('demo', PLAN))
    const report = await verify(appDir)

    expect(report.status).toBe('passed')
    expect(report.uncovered).toBe(0)
    // 五个形态各自至少一条声明进入运行时（constraints 1 / rules 1 / statemachines 1 / workflows 1 / profiles 1）。
    expect(report.declarations.map(declaration => declaration.file).sort()).toEqual([
      'constraints.yaml', 'profiles.yaml', 'rules.yaml', 'statemachines.yaml', 'workflows.yaml',
    ])
    expect(report.declarations.every(declaration => declaration.status === 'wired')).toBe(true)
    expect(report.covered).toBe(5)
  })

  it('an empty plan declares nothing (honest skeleton, not a fake pass)', async () => {
    const appDir = await appDirWith(generateExtensions('demo'))
    const report = await verify(appDir)
    expect(report.status).toBe('passed')
    expect(report.covered).toBe(0)
    expect(report.uncovered).toBe(0)
  })

  it('would catch the loader-shape mistake it avoids (profiles.yaml as a sequence)', async () => {
    // 反例：profiles.yaml 顶层是序列 ⇒ 加载器 `ProfilesWrapper` 反序列化失败 ⇒ 声明从未生效。
    const appDir = await appDirWith({ 'profiles.yaml': '- name: default\n' })
    const report = await verify(appDir)
    expect(report.status).toBe('degraded')
    expect(report.uncovered).toBe(1)
  })
})
