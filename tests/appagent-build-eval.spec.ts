/**
 * The consumer-side build-eval runner reads mechanical evidence off disk and turns it
 * into per-dimension scores. These tests pin the behaviours that decide whether a run
 * is trustworthy: evidence present → full credit, artifact absent or unparsable →
 * zero (never a default), and partial artifact trees → a fraction, with `scoreRun`
 * marking the whole round degraded so a missing evidence face cannot pass as a build.
 */
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { scoreRun, type EvalCase, type EvalCaseSet } from '@dsh-alioth/verify-alioth'
import { collectCaseEvidence, collectEvidence } from '../scripts/appagent-build-eval.ts'

const CASE_SPEC: EvalCase = {
  id: 'c1',
  namespace: 'U-alice',
  app: 'demo',
  goal: 'demo app',
  weight: 1,
  dimensions: { extension_verify: 0.35, e2e: 0.25, closure_audit: 0.2, eval_report_rules: 0.1, artifacts: 0.1 },
}

const CASE_SET: EvalCaseSet = {
  schema: 'appagent-build-eval/v1',
  threshold: 0.8,
  tolerance: 0.1,
  cases: [CASE_SPEC],
}

async function writeAppTree(input: {
  readonly preProcRoot: string
  readonly extensionVerify?: string
  readonly e2e?: string
  readonly closure?: string
  readonly evalReport?: string
  readonly appJson?: string | null
  readonly extensions?: readonly string[]
}): Promise<string> {
  const appDir = path.join(input.preProcRoot, CASE_SPEC.namespace, 'Apps', CASE_SPEC.app)
  await mkdir(appDir, { recursive: true })
  if (input.appJson !== null) {
    await writeFile(path.join(appDir, 'app.json'), input.appJson ?? '{"code":"demo"}\n', 'utf8')
  }
  const extensions = input.extensions ?? ['orders.yaml']
  await mkdir(path.join(appDir, 'extensions'), { recursive: true })
  for (const entry of extensions) {
    await writeFile(path.join(appDir, 'extensions', entry), '- entity: order\n', 'utf8')
  }
  if (input.extensionVerify !== undefined) {
    await writeFile(path.join(appDir, 'extension-verify.json'), input.extensionVerify, 'utf8')
  }
  if (input.e2e !== undefined) {
    await writeFile(path.join(appDir, 'e2e-report.json'), input.e2e, 'utf8')
  }
  if (input.closure !== undefined) {
    const dir = path.join(appDir, 'AppAgentTraces', 'closure-audit')
    await mkdir(dir, { recursive: true })
    await writeFile(path.join(dir, '0001.json'), input.closure, 'utf8')
  }
  if (input.evalReport !== undefined) {
    await writeFile(path.join(appDir, 'eval-report.json'), input.evalReport, 'utf8')
  }
  return appDir
}

describe('build-eval evidence collection', () => {
  it('scores every dimension from the artifacts our tools wrote', async () => {
    const preProcRoot = await mkdtemp(path.join(tmpdir(), 'build-eval-full-'))
    await writeAppTree({
      preProcRoot,
      extensionVerify: '{"status":"passed"}\n',
      e2e: '{"passed":true}\n',
      closure: '{"seq":1,"verdict":"approved"}\n',
      evalReport: '{"passed":true,"dimensions":{"schema_validity":1,"prototype_standalone":0.5}}\n',
    })

    const evidence = await collectEvidence({ caseSpec: CASE_SPEC, preProcRoot })

    expect(evidence.extension_verify).toBe(1)
    expect(evidence.e2e).toBe(1)
    expect(evidence.closure_audit).toBe(1)
    expect(evidence.eval_report_rules).toBeCloseTo(0.75, 6)
    expect(evidence.artifacts).toBe(1)

    const scored = scoreRun({ caseSet: CASE_SET, evidence: { c1: evidence } })
    expect(scored.degraded).toBe(false)
    expect(scored.score).toBeCloseTo(0.35 + 0.25 + 0.2 + 0.1 * 0.75 + 0.1, 6)
  })

  it('treats absent or unparsable evidence as missing, never as a passing default', async () => {
    const preProcRoot = await mkdtemp(path.join(tmpdir(), 'build-eval-missing-'))
    await writeAppTree({
      preProcRoot,
      extensionVerify: '{"status":"degraded"}\n',
      e2e: 'not json at all\n',
    })

    const evidence = await collectEvidence({ caseSpec: CASE_SPEC, preProcRoot })

    expect(evidence.extension_verify).toBe(0)
    expect(evidence.e2e).toBeUndefined()
    expect(evidence.closure_audit).toBeUndefined()
    expect(evidence.eval_report_rules).toBeUndefined()

    const scored = scoreRun({ caseSet: CASE_SET, evidence: { c1: evidence } })
    expect(scored.degraded).toBe(true)
    expect(scored.score).toBeLessThan(CASE_SET.threshold)
  })

  it('reports a partial artifact tree as a fraction and ignores a non-approved verdict', async () => {
    const preProcRoot = await mkdtemp(path.join(tmpdir(), 'build-eval-partial-'))
    await writeAppTree({
      preProcRoot,
      closure: '{"seq":1,"verdict":"escalate"}\n',
      appJson: null,
      extensions: ['a.yaml', 'b.yaml', 'c.yaml'],
    })

    const evidence = await collectEvidence({ caseSpec: CASE_SPEC, preProcRoot })

    expect(evidence.closure_audit).toBe(0)
    // One app.json expected plus three declared extensions; all three files exist → 3/4.
    expect(evidence.artifacts).toBeCloseTo(0.75, 6)
  })

  it('collects evidence for every case in the set', async () => {
    const preProcRoot = await mkdtemp(path.join(tmpdir(), 'build-eval-set-'))
    await writeAppTree({ preProcRoot, extensionVerify: '{"status":"passed"}\n' })
    const second: EvalCase = { ...CASE_SPEC, id: 'c2', app: 'other' }

    const collected = await collectCaseEvidence({
      caseSet: { ...CASE_SET, cases: [CASE_SPEC, second] },
      preProcRoot,
    })

    expect(Object.keys(collected).sort()).toEqual(['c1', 'c2'])
    expect(collected.c2?.extension_verify).toBeUndefined()
  })
})
