/**
 * 诚实评分 / fail-closed 行为：缺失产物记 0 分并留 violation；degraded ≠ passed；
 * publish 影子谓词看到 degraded。〔契约 §4/T3 必需测试〕
 * @module @dsh-alioth/verify-alioth/tests/fail-closed
 */

import { describe, expect, it, afterEach } from 'vitest'
import { readFile } from 'node:fs/promises'
import { buildEvalReport, writeEvalReport } from '../src/eval-report.ts'
import { verifyExtensions, writeExtensionVerify, type ExtensionVerification } from '../src/extension-verify.ts'
import { evaluatePublishShadow } from '../src/auto-gate.ts'
import { evaluateStageGate } from '../src/stage-gates.ts'
import { createTempApp, STANDALONE_PROTOTYPE_HTML, VALID_APP_JSON, type TempApp } from './fixture.ts'

const opened: TempApp[] = []
async function app(files: Readonly<Record<string, string>> = {}): Promise<TempApp> {
  const created = await createTempApp(files)
  opened.push(created)
  return created
}
afterEach(async () => {
  await Promise.all(opened.splice(0).map(entry => entry.cleanup()))
})

const ALLOWED_FORMS = ['constraints', 'rules', 'statemachines', 'workflows', 'profiles']

const VALID_STATEMACHINES = `- entity: Order
  state_field: status
  states:
    - name: draft
    - name: submitted
  transitions:
    - from: draft
      to: submitted
  initial_state: draft
`

describe('buildEvalReport — 诚实评分（缺失 ≠ 满分）', () => {
  it('产物全缺 → 两维各 0 分、逐维记 violation、passed=false', async () => {
    const target = await app()
    const report = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: target.appDir })

    expect(report.dimensions).toEqual({ schema_validity: 0, prototype_standalone: 0 })
    expect(report.passed).toBe(false)
    expect(report.violations.map(v => v.rule).sort()).toEqual(['prototype_standalone', 'schema_validity'])
    expect(report.violations.every(v => v.severity === 'error')).toBe(true)
    expect(report.evaluated_dimensions).toEqual(['schema_validity', 'prototype_standalone'])
    expect(report.note).toContain('未通过')
  })

  it('app.json 不可解析 → schema 维度 0 分且 violation 记明原因（不得静默满分）', async () => {
    const target = await app({ 'app.json': '{ not json', 'prototype.html': STANDALONE_PROTOTYPE_HTML })
    const report = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: target.appDir })

    expect(report.dimensions['schema_validity']).toBe(0)
    expect(report.dimensions['prototype_standalone']).toBe(1)
    expect(report.passed).toBe(false)
    expect(report.violations[0]?.message).toContain('不可解析')
  })

  it('合规产物 → 双维满分且 passed=true、violations 为空', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'prototype.html': STANDALONE_PROTOTYPE_HTML })
    const report = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: target.appDir })

    expect(report.dimensions).toEqual({ schema_validity: 1, prototype_standalone: 1 })
    expect(report.violations).toEqual([])
    expect(report.passed).toBe(true)
  })

  it('原型引用外部 CDN 或越界相对路径 → prototype 维度 0 分并逐条留 violation', async () => {
    const external = await app({
      'app.json': VALID_APP_JSON,
      'prototype.html': '<html><script src="https://cdn.example.com/vue.js"></script></html>',
    })
    const externalReport = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: external.appDir })
    expect(externalReport.dimensions['prototype_standalone']).toBe(0)
    expect(externalReport.passed).toBe(false)
    expect(externalReport.violations.some(v => v.message.includes('外部资源引用'))).toBe(true)

    const escaping = await app({
      'app.json': VALID_APP_JSON,
      'prototype.html': '<html><img src="../../../../etc/hosts" /></html>',
    })
    const escapingReport = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: escaping.appDir })
    expect(escapingReport.dimensions['prototype_standalone']).toBe(0)
    expect(escapingReport.violations.some(v => v.message.includes('越界'))).toBe(true)
  })

  it('writeEvalReport 落盘 eval-report.json 且内容与报告一致', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON, 'prototype.html': STANDALONE_PROTOTYPE_HTML })
    const report = await buildEvalReport({ app: 'tm-app', namespace: 'TestNS', appDir: target.appDir })
    const written = await writeEvalReport(target.appDir, report)

    expect(written).toBe(`${target.appDir}/eval-report.json`)
    const persisted = JSON.parse(await readFile(written, 'utf8')) as typeof report
    expect(persisted.passed).toBe(true)
    expect(persisted.evaluated_dimensions).toEqual(report.evaluated_dimensions)
  })
})

describe('verifyExtensions — degraded ≠ passed', () => {
  it('声明齐备且形态在 allowedForms 内 → passed，且仅落 canonical 报告', async () => {
    const target = await app({ 'extensions/statemachines.yaml': VALID_STATEMACHINES })
    const result = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: target.appDir,
      allowedForms: ALLOWED_FORMS,
    })

    expect(result.status).toBe('passed')
    expect(result.covered).toBe(1)
    expect(result.uncovered).toBe(0)
    expect(result.declarations[0]?.status).toBe('wired')

    const written = await writeExtensionVerify(target.appDir, result)
    expect(written).toBe(`${target.appDir}/extension-verify.json`)
    await expect(readFile(written, 'utf8')).resolves.toContain('"passed"')
  })

  it('条目缺必需键 → uncovered 且整体 degraded，报告落 .degraded 位（不得写 canonical 冒充通过）', async () => {
    const target = await app({
      'extensions/rules.yaml': `- entity: Order\n  name: discount\n  condition: amount > 0\n  action: discount_rate = 0.1\n`,
    })
    const result = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: target.appDir,
      allowedForms: ALLOWED_FORMS,
    })

    expect(result.status).toBe('degraded')
    expect(result.uncovered).toBe(1)
    expect(result.declarations[0]?.evidence).toContain('trigger')

    const written = await writeExtensionVerify(target.appDir, result)
    expect(written).toBe(`${target.appDir}/extension-verify.degraded.json`)
    await expect(readFile(`${target.appDir}/extension-verify.json`, 'utf8')).rejects.toThrow()
  })

  it('加载器不认的文件名 / 未注入的形态 / 顶层形状不符 → 一律 uncovered（degraded）', async () => {
    const unknownName = await app({ 'extensions/notes.yaml': '- x: 1\n' })
    const unknownResult = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: unknownName.appDir,
      allowedForms: ALLOWED_FORMS,
    })
    expect(unknownResult.status).toBe('degraded')
    expect(unknownResult.declarations[0]?.form).toBe('unknown')

    const notInjected = await app({ 'extensions/workflows.yaml': `- name: wf\n  trigger: { entity: Order, event: create }\n  steps: []\n` })
    const notInjectedResult = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: notInjected.appDir,
      allowedForms: ['statemachines'],
    })
    expect(notInjectedResult.status).toBe('degraded')
    expect(notInjectedResult.declarations[0]?.evidence).toContain('allowedForms')

    const wrongShape = await app({ 'extensions/constraints.yaml': 'entity: Order\n' })
    const wrongShapeResult = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: wrongShape.appDir,
      allowedForms: ALLOWED_FORMS,
    })
    expect(wrongShapeResult.status).toBe('degraded')
    expect(wrongShapeResult.declarations[0]?.evidence).toContain('顶层形状非数组')
  })

  it('无 extensions/ 目录 → 无声明可验，不产生假 degraded', async () => {
    const target = await app()
    const result = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: target.appDir,
      allowedForms: ALLOWED_FORMS,
    })

    expect(result).toMatchObject({ status: 'passed', covered: 0, uncovered: 0 })
    expect(result.note).toContain('目录不存在')
  })
})

describe('publish 影子谓词 — degraded 不进通过面', () => {
  const base = {
    artifactsComplete: true,
    qualityPassed: true,
    closureApproved: true,
    noOpenDeferred: true,
  }

  it('五谓词全真 → willAutoApprove=true', () => {
    const shadow = evaluatePublishShadow({ ...base, extensionStatus: 'passed' })
    expect(shadow.willAutoApprove).toBe(true)
    expect(shadow.predicates['extension_verify_passed']).toBe(true)
  })

  it('扩展 degraded → 该谓词 false 且整体不放行', () => {
    const shadow = evaluatePublishShadow({ ...base, extensionStatus: 'degraded' })
    expect(shadow.predicates['extension_verify_passed']).toBe(false)
    expect(shadow.willAutoApprove).toBe(false)
  })

  it('任一谓词假 → 不放行（fail-closed）', () => {
    const shadow = evaluatePublishShadow({ ...base, extensionStatus: 'passed', noOpenDeferred: false })
    expect(shadow.predicates['no_open_deferred']).toBe(false)
    expect(shadow.willAutoApprove).toBe(false)
  })
})

describe('阶段判据 — 缺失/未通过一律 fail-closed', () => {
  it('quality 门判产物契约面（schema_validity）；原型面由 per-App 人工门承压', async () => {
    // 口径必须与 orchestrator 的 E2E/发布前置一致：一次 create 不能自相矛盾。
    const probe = async (body: string): Promise<{ ok: boolean; evidence: string; artifacts: readonly string[] }> => {
      const fixture = await app({ 'eval-report.json': body })
      return await evaluateStageGate('quality', {
        appDir: fixture.appDir,
        preProcRoot: fixture.preProcRoot,
        namespace: 'TestNS',
        app: 'tm-app',
      })
    }

    const missing = await app()
    const missingOutcome = await evaluateStageGate('quality', {
      appDir: missing.appDir,
      preProcRoot: missing.preProcRoot,
      namespace: 'TestNS',
      app: 'tm-app',
    })
    expect(missingOutcome.ok).toBe(false)
    expect(missingOutcome.evidence).toContain('缺失')

    // 契约面为 0 ⇒ 不过（缺失/不合格按 0 分计）
    const zero = await probe('{"passed":false,"dimensions":{"schema_validity":0,"prototype_standalone":0}}\n')
    expect(zero.ok).toBe(false)
    expect(zero.evidence).toContain('schema_validity')

    // 契约面达 1、原型面未达 ⇒ 本门通过（原型义务在人工门上，publish 前置 2 会阻断）
    const contractOnly = await probe('{"passed":false,"dimensions":{"schema_validity":1,"prototype_standalone":0}}\n')
    expect(contractOnly.ok).toBe(true)
    expect(contractOnly.evidence).toContain('prototype_standalone=0')
    expect(contractOnly.evidence).toContain('人工门')
    expect(contractOnly.artifacts).toEqual(['Apps/tm-app/eval-report.json'])

    const full = await probe('{"passed":true,"dimensions":{"schema_validity":1,"prototype_standalone":1}}\n')
    expect(full.ok).toBe(true)
    expect(full.evidence).toContain('passed=true')
  })

  it('factor-dev / ontology-mapping / block-refinement 判据按产物实质判定', async () => {
    const target = await app({ 'app.json': VALID_APP_JSON })
    await target.writePreProc('local/ontology-output.json', '{"mappings":[]}\n')
    await target.writePreProc('Sources/Apps/Services/order-service/service.json', '{"code":"order-service"}\n')
    await target.writePreProc('Sources/Apps/Blocks/order-list/block.json', '{"block":"ORD","flows":[{"id":"browse"}]}\n')

    const input = {
      appDir: target.appDir,
      preProcRoot: target.preProcRoot,
      namespace: 'TestNS',
      app: 'tm-app',
    }
    expect(await evaluateStageGate('appagent-ready', input)).toMatchObject({ ok: true })
    expect((await evaluateStageGate('factor-dev', input)).ok).toBe(true)
    expect((await evaluateStageGate('ontology-mapping', input)).ok).toBe(true)
    expect((await evaluateStageGate('block-extract', input)).ok).toBe(true)
    expect((await evaluateStageGate('block-refinement', input)).ok).toBe(true)

    const undeclared = await app()
    await undeclared.writePreProc('Sources/Apps/Blocks/order-list/block.json', '{"block":"ORD"}\n')
    const undeclaredOutcome = await evaluateStageGate('block-refinement', {
      appDir: undeclared.appDir,
      preProcRoot: undeclared.preProcRoot,
      namespace: 'TestNS',
      app: 'tm-app',
    })
    expect(undeclaredOutcome.ok).toBe(false)
  })

  it('module-design：声明 modules 时逐个须齐，缺一即 false', async () => {
    const target = await app()
    await target.writePreProc('Sources/Apps/Modules/orders/module.json', '{"id":"orders"}\n')
    const input = { appDir: target.appDir, preProcRoot: target.preProcRoot, namespace: 'TestNS', app: 'tm-app' }

    expect((await evaluateStageGate('module-design', { ...input, modules: ['orders'] })).ok).toBe(true)
    expect((await evaluateStageGate('module-design', { ...input, modules: ['orders', 'billing'] })).ok).toBe(false)
  })
})

describe('ExtensionVerification 形态自洽', () => {
  it('covered/uncovered 与 declarations 计数一致', async () => {
    const target = await app({
      'extensions/statemachines.yaml': VALID_STATEMACHINES,
      'extensions/bogus.yaml': '- a: 1\n',
    })
    const result: ExtensionVerification = await verifyExtensions({
      app: 'tm-app',
      namespace: 'TestNS',
      appDir: target.appDir,
      allowedForms: ALLOWED_FORMS,
    })

    expect(result.covered + result.uncovered).toBe(result.declarations.length)
    expect(result.status).toBe('degraded')
  })
})
