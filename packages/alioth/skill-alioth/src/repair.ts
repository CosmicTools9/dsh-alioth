/**
 * 修复契约：把「门禁/工具失败 = 一段终端输出」升级为「失败 = 规则码 + 修复类 + 下一步动作」。
 *
 * 契约来源：上游 `Meta/backend/app-agent/src/repair.rs`（`RepairClass` / `FailureKind` /
 * `RepairContract::new` / `render` / `rule_id_from_error` 与规则表 `classify`）。上游原实现
 * 只回 `format!("gate '{}' exit {:?} != {}", ...)`——LLM 拿到的是原始终端文本，既无分类也
 * 无行动指引；同一根因在不同轮次的输出漂移还会污染重试预算的错误签名（修复墙判定失效）。
 *
 * 本条对齐要点：
 * - 承载形式 = 工具结果 `error` 单通道字符串，故契约以固定前缀 `[rule:<id>]` 编入错误文本；
 *   {@link ruleIdFromError} 是**唯一解析点**，`retry_budget` 据此取得稳定签名。
 * - 规则键 = 结构化失败特征（{@link FailureKind}，调用点给出）+ 程序/工具名，**不以自由文本
 *   匹配作为唯一判据**；每条规则 MUST 带非空 `suggestedAction`。
 * - 证据进错误信息前截断到 {@link EVIDENCE_HEAD_LIMIT} 字符：契约保持单行可读，全文仍在
 *   trace 中可查。
 * @module @dsh-alioth/skill-alioth/repair
 */

/** 证据文本进入错误信息的截断上限（字符）。 */
export const EVIDENCE_HEAD_LIMIT = 400

/** 契约前缀（唯一解析点见 {@link ruleIdFromError}）。 */
export const RULE_PREFIX = '[rule:'

/** 修复类——决定重试策略。 */
export type RepairClass = 'fixable' | 'retryable' | 'not-fixable'

/**
 * 结构化失败特征（调用点给出，不从自由文本推测）。与上游 `FailureKind` 的对应关系：
 * `gate-exit`↔`NonZeroExit`、`gate-timeout`↔`Timeout`、`gate-missing-program`↔
 * `NotWhitelisted`/`IoError`、`gate-output-missing`↔`OutputGlobMiss`、
 * `gate-json-predicate`↔`JsonPredicateFail`、`plan-write-outside-scope`↔
 * `PlanWriteOutsideScope`；`tool-denied`/`write-outside-sandbox`/`unknown` 是 harness 侧
 * 新增（上游对应 `tool_registry` 沙箱拒绝与未分类兜底）。
 */
export type FailureKind =
  | 'gate-exit'
  | 'gate-timeout'
  | 'gate-missing-program'
  | 'gate-output-missing'
  | 'gate-json-predicate'
  | 'tool-denied'
  | 'write-outside-sandbox'
  | 'plan-write-outside-scope'
  | 'unknown'

/** 一次失败的结构化修复契约。 */
export interface RepairContract {
  readonly ruleId: string
  readonly class: RepairClass
  readonly message: string
  readonly suggestedAction: string
  readonly evidence: string
}

/** 规则表的一行：`(失败特征, 程序/工具名) → (ruleId, 修复类, 建议动作, 消息)`。 */
interface RepairRule {
  readonly ruleId: string
  readonly class: RepairClass
  readonly suggestedAction: string
  readonly message: (source: string) => string
}

/** 全部已注册规则码（契约完整性测试与自检共用）。 */
export const REGISTERED_RULE_IDS: readonly string[] = [
  'gate-program-not-whitelisted',
  'gate-timeout',
  'gate-cargo-nonzero',
  'gate-check-script-nonzero',
  'gate-frontend-nonzero',
  'gate-exit-nonzero',
  'gate-output-glob-miss',
  'gate-json-predicate-fail',
  'tool-call-denied',
  'tool-write-outside-sandbox',
  'plan-phase-write-outside-scope',
  'unknown-failure',
]

/** 退出码非零规则按程序细化（上游 `classify` 的 `NonZeroExit` 分支）。 */
const EXIT_RULES: Readonly<Record<string, RepairRule>> = {
  cargo: {
    ruleId: 'gate-cargo-nonzero',
    class: 'fixable',
    suggestedAction: '按证据中的 rustc 报错逐条修复，再用 bash 复验 cargo check',
    message: () => 'cargo 门禁以非期望退出码结束',
  },
  bun: {
    ruleId: 'gate-check-script-nonzero',
    class: 'fixable',
    suggestedAction: '脚本输出即规约判据：按首条违规修复受控文件后重跑该检查',
    message: () => 'bun 门禁脚本以非期望退出码结束',
  },
  npx: {
    ruleId: 'gate-frontend-nonzero',
    class: 'fixable',
    suggestedAction: '按证据中首个类型/构建错误定位修复后重跑',
    message: () => 'npx 前端门禁以非期望退出码结束',
  },
}

const GENERIC_EXIT_RULE: RepairRule = {
  ruleId: 'gate-exit-nonzero',
  class: 'fixable',
  suggestedAction: '读证据原文定位失败根因，修复后重跑该门禁',
  message: source => `门禁程序以非期望退出码结束（${source}）`,
}

/** 按失败特征（+程序/工具名）查规则表；未登记特征落 `unknown-failure`（绝不返回空指引）。 */
function classify(failure: FailureKind, source: string): RepairRule {
  switch (failure) {
    case 'gate-exit':
      return EXIT_RULES[source] ?? GENERIC_EXIT_RULE
    case 'gate-timeout':
      return {
        ruleId: 'gate-timeout',
        class: 'retryable',
        suggestedAction: '缩小该步产出范围，或在 adapter 中提高该 gate 的 timeout_sec',
        message: source === '' ? () => '门禁程序超时' : name => `门禁程序超时（${name}）`,
      }
    case 'gate-missing-program':
      return {
        ruleId: 'gate-program-not-whitelisted',
        class: 'not-fixable',
        suggestedAction: '改用白名单内程序（bun / npx / cargo / bash / target/debug/ontology-mapping）',
        message: source === '' ? () => '门禁程序不可用' : name => `门禁程序不在白名单或不可启动（${name}）`,
      }
    case 'gate-output-missing':
      return {
        ruleId: 'gate-output-glob-miss',
        class: 'fixable',
        suggestedAction: '按该步产出口径产出文件（路径与命名对齐 output_glob）',
        message: () => '门禁产物 glob 未命中',
      }
    case 'gate-json-predicate':
      return {
        ruleId: 'gate-json-predicate-fail',
        class: 'fixable',
        suggestedAction: '按 require_json_pointer 指向字段与期望值修正产物内容',
        message: () => '产物 JSON 内容谓词不满足',
      }
    case 'tool-denied':
      return {
        ruleId: 'tool-call-denied',
        class: 'not-fixable',
        suggestedAction: '该调用被运行时拒绝：改用白名单内工具/路径，或改由人工在沙箱外执行',
        message: source === '' ? () => '工具调用被拒绝' : name => `工具调用被拒绝（${name}）`,
      }
    case 'write-outside-sandbox':
      return {
        ruleId: 'tool-write-outside-sandbox',
        class: 'not-fixable',
        suggestedAction: '写面限定在 Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/：把目标移入沙箱，或改由人工落地',
        message: source === '' ? () => '写入越出 namespace 沙箱' : name => `写入越出 namespace 沙箱（${name}）`,
      }
    case 'plan-write-outside-scope':
      return {
        ruleId: 'plan-phase-write-outside-scope',
        class: 'fixable',
        suggestedAction: '本步是 plan（方案）步：只写本步 output_glob 声明的方案产物，落地写移到后续 apply 步',
        message: () => 'plan 步写入其声明方案产物之外的路径',
      }
    case 'unknown':
      return {
        ruleId: 'unknown-failure',
        class: 'retryable',
        suggestedAction: '查看 trace 全文定位根因；确认非环境噪声后原样重试该步',
        message: source === '' ? () => '未分类失败' : name => `未分类失败（${name}）`,
      }
  }
}

/**
 * 构造一次失败的结构化修复契约。
 * @param failure - 结构化失败特征（调用点给出，不从文本推测）。
 * @param source - 程序或工具名；退出码非零/超时等规则据此细化。
 * @param evidence - 原始证据（终端输出片段等），进契约前会截断。
 */
export function repairContractFor(failure: FailureKind, source: string, evidence: string): RepairContract {
  const rule = classify(failure, source)
  return {
    ruleId: rule.ruleId,
    class: rule.class,
    message: rule.message(source),
    suggestedAction: rule.suggestedAction,
    evidence,
  }
}

/**
 * 从错误文本取规则码（唯一解析点）。无 `[rule:<id>]` 信封、信封未闭合或 id 为空 → `null`，
 * 调用方按无信封处理（重试预算退化为「工具名 + 错误文本哈希」签名）。
 */
export function ruleIdFromError(errorText: string): string | null {
  const start = errorText.indexOf(RULE_PREFIX)
  if (start < 0) return null
  const rest = errorText.slice(start + RULE_PREFIX.length)
  const end = rest.indexOf(']')
  if (end < 0) return null
  const id = rest.slice(0, end).trim()
  return id === '' ? null : id
}

/** 单行折行 + 截断的证据头（上游 `render` 的空白归并 + `EVIDENCE_HEAD_LIMIT` 截断）。 */
function evidenceHead(evidence: string): string {
  const collapsed = evidence.slice(0, EVIDENCE_HEAD_LIMIT).split(/\s+/).filter(part => part !== '').join(' ')
  if (collapsed === '') return ''
  return evidence.length > EVIDENCE_HEAD_LIMIT ? `${collapsed}（截断，全文见 trace）` : collapsed
}

/**
 * 渲染为单行错误文本（工具结果 `error` 字段的唯一形态）：
 * `[rule:<id>] class=<class> · <message> · 下一步：<action> · 证据：<head>`。
 * 证据为空时不产生空段。
 */
export function formatRepairError(contract: RepairContract): string {
  const head = evidenceHead(contract.evidence)
  const evidence = head === '' ? '' : ` · 证据：${head}`
  return `${RULE_PREFIX}${contract.ruleId}] class=${contract.class} · ${contract.message} · 下一步：${contract.suggestedAction}${evidence}`
}
