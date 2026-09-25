/**
 * `alioth_verify` / `alioth_closure`——模型面验证工具（薄封装 `@dsh-alioth/verify-alioth`，零 LLM）。
 *
 * 契约要点：
 * - `alioth_verify extensions` 的 `allowedForms` 来自 Gateway loader 真形态表
 *   （`./gateway-extensions.ts`，来源 `extension.rs:836-910`），MUST NOT 臆造形态名；
 *   `status='degraded'` 是诚实结论，MUST NOT 被表述为通过。
 * - `alioth_verify stage` 走 7 阶段实质判据；**缺失产物 = 不通过**，以带 `[rule:<id>]` 前缀的
 *   修复契约抛错（复用 `skill-alioth` 的唯一错误信封），不做「静默放行」。
 * - `alioth_closure` 裁决 append-only 落 `AppAgentTraces/closure-audit/{seq}.json`，指纹锁死
 *   「审计后偷改产物」；连续 rejected 达阈值由库升为 `escalate`。
 * @module @dsh-alioth/tool-alioth-verify/verify-tools
 */

import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { formatRepairError, repairContractFor } from '@dsh-alioth/skill-alioth'
import {
  appendClosureVerdict,
  artifactFingerprint,
  buildEvalReport,
  evaluateStageGate,
  pipelineProgressJson,
  scanStageProgress,
  EXTENSION_FORMS,
  latestMatchingVerdict,
  readClosureVerdicts,
  verifyExtensions,
  writeEvalReport,
  writeExtensionVerify,
  type ClosureFinding,
  type DeferredStore,
  type StageId,
} from '@dsh-alioth/verify-alioth'
import { extensionsGateItem, extensionsGateScope } from './extensions-gate.ts'
import { asJsonOutput } from './json-output.ts'
import { appDirOf, assertNamespaceApp, namespaceRootOf, requireString } from './paths.ts'

const VERIFY_ACTIONS = ['artifacts', 'extensions', 'stage', 'progress'] as const
const CLOSURE_ACTIONS = ['verdict', 'status'] as const

/** 7 阶段 id（与 `verify-alioth/stage-gates.ts` 的 `StageId` 联合一一对应）。 */
const STAGE_IDS: readonly StageId[] = [
  'appagent-ready',
  'module-design',
  'block-extract',
  'block-refinement',
  'ontology-mapping',
  'factor-dev',
  'quality',
]

/** 解析 closure 发现集（形态非法即 throw——裁决是审计证据，形态错误不得静默丢弃）。 */
function parseFindings(value: unknown): readonly ClosureFinding[] {
  if (value === undefined) return []
  if (!Array.isArray(value)) throw new Error('alioth_closure: findings 必须是数组')
  return value.map((entry, index) => {
    if (typeof entry !== 'object' || entry === null || Array.isArray(entry)) {
      throw new Error(`alioth_closure: findings[${index}] 必须是对象`)
    }
    const record = entry as Record<string, unknown>
    const dimension = record['dimension']
    const verdict = record['verdict']
    const detail = record['detail']
    if (typeof dimension !== 'string' || dimension.trim() === '') {
      throw new Error(`alioth_closure: findings[${index}].dimension 必填`)
    }
    if (verdict !== 'pass' && verdict !== 'fail' && verdict !== 'unknown') {
      throw new Error(`alioth_closure: findings[${index}].verdict 必须是 pass|fail|unknown`)
    }
    if (typeof detail !== 'string') {
      throw new Error(`alioth_closure: findings[${index}].detail 必填（字符串）`)
    }
    return { dimension, verdict, detail }
  })
}

/** 解析证据字符串数组。 */
function parseEvidence(value: unknown): readonly string[] {
  if (value === undefined) return []
  if (!Array.isArray(value) || value.some(entry => typeof entry !== 'string')) {
    throw new Error('alioth_closure: evidence 必须是字符串数组')
  }
  return value as readonly string[]
}

/** 解析可选 modules 列表（`module-design` 判据的逐模块清单）。 */
function parseModules(value: unknown): readonly string[] {
  if (value === undefined) return []
  if (!Array.isArray(value) || value.some(entry => typeof entry !== 'string')) {
    throw new Error('alioth_verify: modules 必须是字符串数组')
  }
  return value as readonly string[]
}

/** 注册 `alioth_verify` 与 `alioth_closure`。 */
export function registerVerifyTools(
  ctx: Context,
  options: { readonly preProcRoot: string; readonly deferred: DeferredStore },
): void {
  const preProcRoot = options.preProcRoot

  ctx.tools.register(defineTool({
    name: 'alioth_verify',
    description:
      'Programmatic verification over an App\'s artifacts (no LLM; never narrate a verdict from memory). Actions: '
      + '"artifacts" — build the eval report (rule dimensions: app.json schema validity + prototype standalone-ness) '
      + 'and persist `eval-report.json` (quality stage artifact); '
      + '"extensions" — check every `extensions/*.yaml` declaration against the Gateway loader contract '
      + '(unknown filename / wrong top-level shape / missing required keys = uncovered = `degraded`, which is NOT a pass); '
      + 'a degraded result REGISTERS a per-App human gate (deferred, cleared only when the canonical '
      + '`extension-verify.json` says status=passed), so a stale canonical pass cannot wave a degraded run through; '
      + '"stage" — evaluate one of the 7 pipeline stages against its declared artifacts; a missing artifact fails '
      + 'with a `[rule:<id>]` repair contract instead of passing silently. '
      + '"progress" — the HONEST 7-stage progress projection over the App tree (`pipeline_progress` segment shape): '
      + 'a stage with no declared artifact on disk is `pending` (never reported completed), a stage with artifacts that '
      + 'fail its gate is reported in `failures` (blocking publish; never silently waved through). '
      + '`write=false` skips persistence (dry read); defaults to writing the canonical evidence file.',
    parameters: {
      action: { type: 'string', required: true, description: `One of: ${VERIFY_ACTIONS.join(', ')}.` },
      namespace: { type: 'string', description: 'Workspace namespace (Pre-Proc/{namespace}); required for every action.' },
      app: { type: 'string', description: 'App code (Apps/{app} under the namespace); required for every action.' },
      stage: { type: 'string', description: `stage only: one of ${STAGE_IDS.join(', ')}.` },
      modules: { type: 'array', items: { type: 'string' }, description: 'stage/progress: module ids that must each have module.json.' },
      write: { type: 'boolean', description: 'artifacts/extensions: persist the canonical evidence file (default true).' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          status: { type: 'string' },
          passed: { type: 'boolean' },
          report: { type: 'json' },
          reportPath: { type: 'string' },
          allowedForms: { type: 'array', items: { type: 'string' } },
          gateScope: { type: 'string' },
          gateRegistered: { type: 'boolean' },
          gatesCleared: { type: 'number' },
          stage: { type: 'string' },
          ok: { type: 'boolean' },
          evidence: { type: 'string' },
          artifacts: { type: 'array', items: { type: 'string' } },
          currentStage: { type: 'string' },
          allCompleted: { type: 'boolean' },
          stages: { type: 'json' },
          failures: { type: 'json' },
          progress: { type: 'json' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'stage'
          ? `stage ${String(value.stage)}: ${value.ok === true ? 'ok' : 'FAIL'} — ${String(value.evidence)}`
          : `${String(value.action)}: status=${String(value.status)}`
            + `${value.reportPath === undefined || value.reportPath === '' ? '' : ` → ${String(value.reportPath)}`}`,
      }],
    },
    async execute(args) {
      const a = args as Record<string, unknown>
      const action = typeof a.action === 'string' ? a.action : ''
      if (!(VERIFY_ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_verify: invalid action ${JSON.stringify(a.action)} (expected ${VERIFY_ACTIONS.join(', ')})`)
      }
      const namespace = requireString(a, 'namespace', 'alioth_verify')
      const app = requireString(a, 'app', 'alioth_verify')
      assertNamespaceApp(namespace, app, 'alioth_verify')
      const appDir = appDirOf(preProcRoot, namespace, app)
      const persist = a.write !== false

      if (action === 'artifacts') {
        const report = await buildEvalReport({ app, namespace, appDir })
        const reportPath = persist ? await writeEvalReport(appDir, report) : ''
        return { action, status: report.passed ? 'passed' : 'failed', passed: report.passed, report: asJsonOutput(report), reportPath }
      }

      if (action === 'extensions') {
        const report = await verifyExtensions({
          app,
          namespace,
          appDir,
          allowedForms: EXTENSION_FORMS,
        })
        const reportPath = persist ? await writeExtensionVerify(appDir, report) : ''
        const gateScope = extensionsGateScope(namespace, app)
        // 人工门只在**落盘运行**上维护：门的解除条件是 canonical 文件为 passed，而只有落盘运行
        // 会让库删掉陈旧 canonical（`write:false` 的干跑不改动任何证据，登记一条立刻可解除的门没有意义）。
        let gateRegistered = false
        let gatesCleared = 0
        if (persist && report.status === 'degraded') {
          await options.deferred.register(extensionsGateItem({
            appDir,
            namespace,
            app,
            uncovered: report.declarations
              .filter(declaration => declaration.status === 'uncovered')
              .map(declaration => `${declaration.file}#${declaration.id}: ${declaration.evidence}`),
          }))
          gateRegistered = true
        } else if (persist) {
          gatesCleared = (await options.deferred.unlockDue(gateScope)).length
        }
        return {
          action,
          status: report.status,
          passed: report.status === 'passed',
          report: asJsonOutput(report),
          reportPath,
          allowedForms: [...EXTENSION_FORMS],
          gateScope,
          gateRegistered,
          gatesCleared,
        }
      }

      if (action === 'progress') {
        const scan = await scanStageProgress({
          appDir,
          preProcRoot: namespaceRootOf(preProcRoot, namespace),
          namespace,
          app,
          modules: parseModules(a.modules),
        })
        // 读侧投影：failures **如实报告**而不 throw（只读面必须能描述一棵坏树；
        // 阻断归 publish 前置与 orchestrator 的 pipelineAdvance）。
        return {
          action,
          currentStage: scan.currentStage ?? '',
          allCompleted: scan.allCompleted,
          stages: scan.stages.map(stage => ({
            id: stage.id,
            label: stage.label,
            status: stage.status,
            gate: stage.gate,
            hasHumanGate: stage.hasHumanGate,
            ...(stage.humanGatePrompt === undefined ? {} : { humanGatePrompt: stage.humanGatePrompt }),
            ...(stage.completedAt === undefined ? {} : { completedAt: stage.completedAt }),
          })),
          failures: scan.failures.map(failure => ({ id: failure.id, evidence: failure.evidence })),
          progress: pipelineProgressJson(scan, { stageSource: namespaceRootOf(preProcRoot, namespace) }),
        }
      }

      const stage = typeof a.stage === 'string' ? a.stage : ''
      if (!(STAGE_IDS as readonly string[]).includes(stage)) {
        throw new Error(`alioth_verify: invalid stage ${JSON.stringify(a.stage)} (expected ${STAGE_IDS.join(', ')})`)
      }
      const outcome = await evaluateStageGate(stage as StageId, {
        appDir,
        preProcRoot: namespaceRootOf(preProcRoot, namespace),
        namespace,
        app,
        modules: parseModules(a.modules),
      })
      if (!outcome.ok) {
        throw new Error(formatRepairError(repairContractFor('gate-output-missing', `stage:${outcome.stage}`, outcome.evidence)))
      }
      return {
        action,
        stage: outcome.stage,
        ok: outcome.ok,
        evidence: outcome.evidence,
        artifacts: [...outcome.artifacts],
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Verify ${String((args as Record<string, unknown>).action ?? '')} ${String((args as Record<string, unknown>).namespace ?? '')}/${String((args as Record<string, unknown>).app ?? '')}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_closure',
    description:
      'Independent closure audit (generation and acceptance are separate: this tool never generates the artifact it judges). '
      + 'Actions: "verdict" — append an audit verdict for the App\'s CURRENT artifact fingerprint '
      + '(`AppAgentTraces/closure-audit/{seq}.json`, append-only); three consecutive rejections escalate to a human; '
      + '"status" — read the audit trail and report whether a verdict matches the current fingerprint '
      + '(a fingerprint change after the audit voids the old verdict, so publish preconditions must re-check). '
      + 'The fingerprint covers app.json bytes + sorted extensions/*.yaml bytes.',
    parameters: {
      action: { type: 'string', required: true, description: `One of: ${CLOSURE_ACTIONS.join(', ')}.` },
      namespace: { type: 'string', description: 'Workspace namespace (Pre-Proc/{namespace}); required.' },
      app: { type: 'string', description: 'App code; required.' },
      verdict: { type: 'string', description: 'verdict only: "approved" or "rejected".' },
      findings: {
        type: 'array',
        items: { type: 'json' },
        description: 'verdict only: findings as [{ dimension, verdict: "pass"|"fail"|"unknown", detail }].',
      },
      evidence: { type: 'array', items: { type: 'string' }, description: 'verdict only: evidence pointers (paths, trace ids).' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          seq: { type: 'number' },
          verdict: { type: 'string' },
          escalated: { type: 'boolean' },
          fingerprint: { type: 'string' },
          total: { type: 'number' },
          matched: { type: 'boolean' },
          matching: { type: 'json' },
          latestSeq: { type: 'number' },
          latestVerdict: { type: 'string' },
          recordPath: { type: 'string' },
          trail: { type: 'json' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'verdict'
          ? `closure verdict #${String(value.seq)}: ${String(value.verdict)}${value.escalated === true ? ' (escalated to human)' : ''}`
          : `closure status: ${String(value.total)} record(s), matched=${String(value.matched)}`,
      }],
    },
    async execute(args) {
      const a = args as Record<string, unknown>
      const action = typeof a.action === 'string' ? a.action : ''
      if (!(CLOSURE_ACTIONS as readonly string[]).includes(action)) {
        throw new Error(`alioth_closure: invalid action ${JSON.stringify(a.action)} (expected ${CLOSURE_ACTIONS.join(', ')})`)
      }
      const namespace = requireString(a, 'namespace', 'alioth_closure')
      const app = requireString(a, 'app', 'alioth_closure')
      assertNamespaceApp(namespace, app, 'alioth_closure')
      const appDir = appDirOf(preProcRoot, namespace, app)

      if (action === 'status') {
        const fingerprint = await artifactFingerprint(appDir)
        const verdicts = await readClosureVerdicts(appDir)
        const matching = await latestMatchingVerdict(appDir, fingerprint)
        const latest = verdicts.at(-1)
        return {
          action,
          fingerprint,
          total: verdicts.length,
          matched: matching !== null,
          matching: asJsonOutput(matching),
          latestSeq: latest?.seq ?? 0,
          latestVerdict: latest?.verdict ?? '',
          trail: verdicts.map(record => ({
            seq: record.seq,
            verdict: record.verdict,
            fingerprint: record.fingerprint,
            ts: record.ts,
          })),
        }
      }

      const verdict = a.verdict
      if (verdict !== 'approved' && verdict !== 'rejected') {
        throw new Error(`alioth_closure: action "verdict" requires verdict "approved"|"rejected" (got ${JSON.stringify(a.verdict)})`)
      }
      const fingerprint = await artifactFingerprint(appDir)
      const record = await appendClosureVerdict(appDir, {
        app,
        namespace,
        verdict,
        fingerprint,
        findings: parseFindings(a.findings),
        evidence: parseEvidence(a.evidence),
      })
      return {
        action,
        seq: record.seq,
        verdict: record.verdict,
        escalated: record.verdict === 'escalate',
        fingerprint: record.fingerprint,
        recordPath: path.join(appDir, 'AppAgentTraces', 'closure-audit', `${record.seq}.json`),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Closure ${String((args as Record<string, unknown>).action ?? '')} ${String((args as Record<string, unknown>).namespace ?? '')}/${String((args as Record<string, unknown>).app ?? '')}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
