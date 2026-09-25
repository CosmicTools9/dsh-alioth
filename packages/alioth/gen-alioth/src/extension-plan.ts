/**
 * 计划驱动的扩展生成（上游 `Meta/backend/app-agent/src/composer.rs::compose_from_flow_plan` 的
 * 「计划 → extensions/*.yaml」那一段）。确定性、零 LLM：语义照搬上游的逐文件派生，形状照搬
 * Gateway `runtime-engine::extension::load_from_dir` 的反序列化契约（vendored
 * `Framework/backend/runtime-contract/src/{extension,behavior}.rs`）。
 *
 * 每个文件的上游派生关系（与 `composer.rs` 逐行对应）：
 * | 文件 | 来源 | 备注 |
 * |---|---|---|
 * | `constraints.yaml` | `plan.constraints` | `level` 只在 `"warning"` 时 `Warning`，其余 `Error`（上游同判） |
 * | `rules.yaml` | `plan.business_rules` | `ruleName→name`、`errorMessage→error_message`、`blocking: true`（上游硬编码） |
 * | `statemachines.yaml` | `plan.ontology_model_json.transaction_lifecycle` | `state_field: "t_state"`、`initial_state = phases[0].name`、guard = `join(" && ")` |
 * | `workflows.yaml` | `plan.workflow_steps` | 单条 `auto_workflow`，步骤动作 = `call_procedure`，`on_error: Abort` |
 * | `profiles.yaml` | 声明的模块集 | `profiles.default.modules`；**无模块时按上游不产出该文件** |
 *
 * 诚实口径：来源为空 ⇒ 该文件保持骨架（空数组），**绝不伪造**条目；`profiles.yaml` 在无模块时
 * 直接不产出（上游同规：避免空档案）。
 *
 * 枚举名 MUST 与 Rust serde 一致（写错 = 加载器反序列化失败 = 声明从未进入运行时）：
 * `ConstraintSeverity`/`WorkflowErrorHandling` 无 `rename_all` ⇒ 大写（`Error`/`Abort`）；
 * `WorkflowAction` 是 `tag = "type"` + `snake_case` ⇒ `{type: "call_procedure", …}`；
 * `WorkflowTrigger.event` 是 `LifecycleEvent`，`rename_all = "camelCase"` ⇒ `onCreate`。
 * @module @dsh-alioth/gen-alioth/extension-plan
 */

import { stringify } from 'yaml'

/** 计划里与扩展生成相关的**结构性子集**（gen-alioth 不依赖 skill-alioth 的契约类型）。 */
export interface ExtensionPlanInput {
  readonly constraints?: readonly {
    readonly entity: string
    readonly field?: string
    readonly expression: string
    /** `error` | `warning`（缺省按 error）。 */
    readonly level?: string
    readonly message: string
  }[]
  readonly businessRules?: readonly {
    readonly entity: string
    readonly ruleName: string
    /** `onCreate` | `onUpdate` | `onTransition` | `always`。 */
    readonly trigger: string
    readonly condition: string
    readonly action: string
    readonly priority?: number
    readonly errorMessage?: string
  }[]
  /** `plan.workflowSteps`：按序转成一条工作流的步骤。 */
  readonly workflowSteps?: readonly string[]
  /** 声明的模块 id（`profiles.yaml` 的 registry 来源）。 */
  readonly modules?: readonly string[]
  /** `plan.ontologyModelJson`（本体模型 JSON 字符串，含 `transaction_lifecycle`）。 */
  readonly ontologyModelJson?: string
}

/** 骨架（来源为空时的诚实占位）：注明生成方与来源，内容为空序列。 */
function skeleton(kind: string, code: string, reason: string): string {
  return `# dsh-alioth generated skeleton — ${kind}.yaml for app ${code}\n# ${reason}\n# Shape follows the Gateway ExtensionLoader contract; fill it before import.\n[]\n`
}

/** 生成头（真来源已产出条目时）。 */
function header(kind: string, code: string, source: string): string {
  return `# dsh-alioth generated from the flow plan — ${kind}.yaml for app ${code}\n# source: ${source}\n`
}

const dump = (value: unknown): string => stringify(value, { lineWidth: 0 })

/** `constraints.yaml` ← `plan.constraints`（上游 `ConstraintExtension` 逐字段对应）。 */
function constraintsYaml(code: string, plan: ExtensionPlanInput): string {
  const source = plan.constraints ?? []
  if (source.length === 0) {
    return skeleton('constraints', code, 'plan.constraints 为空')
  }
  const entries = source.map(constraint => ({
    entity: constraint.entity,
    ...(constraint.field === undefined ? {} : { field: constraint.field }),
    expression: constraint.expression,
    level: constraint.level === 'warning' ? 'Warning' : 'Error',
    message: constraint.message,
  }))
  return header('constraints', code, 'plan.constraints') + dump(entries)
}

/** `rules.yaml` ← `plan.businessRules`（上游 `RuleExtension`；`blocking` 上游硬编码 true）。 */
function rulesYaml(code: string, plan: ExtensionPlanInput): string {
  const source = plan.businessRules ?? []
  if (source.length === 0) {
    return skeleton('rules', code, 'plan.businessRules 为空')
  }
  const entries = source.map(rule => ({
    entity: rule.entity,
    name: rule.ruleName,
    trigger: rule.trigger,
    condition: rule.condition,
    action: rule.action,
    priority: rule.priority ?? 0,
    error_message: rule.errorMessage ?? '',
    blocking: true,
  }))
  return header('rules', code, 'plan.businessRules') + dump(entries)
}

/** `transaction_lifecycle` 的最小读取面（本体模型 JSON 的一个子对象；形状不对即当作不存在）。 */
interface TransactionLifecycle {
  readonly name?: unknown
  readonly phases?: unknown
  readonly transitions?: unknown
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : null
}

const asText = (value: unknown): string => (typeof value === 'string' ? value : '')

/**
 * `statemachines.yaml` ← 本体模型的 `transaction_lifecycle`（上游 `state_machine_from_lifecycle`）。
 * JSON 不可解析 / 缺 `transaction_lifecycle` ⇒ 骨架（如实说没有来源，不猜）。
 */
function statemachinesYaml(code: string, plan: ExtensionPlanInput): string {
  const raw = plan.ontologyModelJson
  if (raw === undefined || raw.trim() === '') {
    return skeleton('statemachines', code, 'plan.ontologyModelJson 缺失')
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw) as unknown
  } catch {
    return skeleton('statemachines', code, 'plan.ontologyModelJson 不可解析为 JSON')
  }
  const lifecycle = asRecord(asRecord(parsed)?.['transaction_lifecycle']) as TransactionLifecycle | null
  if (lifecycle === null) {
    return skeleton('statemachines', code, '本体模型不含 transaction_lifecycle')
  }
  const phases = (Array.isArray(lifecycle.phases) ? lifecycle.phases : [])
    .map(asRecord)
    .filter((phase): phase is Record<string, unknown> => phase !== null)
  if (phases.length === 0) {
    return skeleton('statemachines', code, 'transaction_lifecycle.phases 为空')
  }
  const nameOf = (id: string): string => asText(phases.find(phase => asText(phase['id']) === id)?.['name']) || id
  const transitions = (Array.isArray(lifecycle.transitions) ? lifecycle.transitions : [])
    .map(asRecord)
    .filter((transition): transition is Record<string, unknown> => transition !== null)
    .map(transition => {
      const guards = (Array.isArray(transition['guard_conditions']) ? transition['guard_conditions'] : [])
        .map(asText)
        .filter(guard => guard !== '')
      return {
        event: asText(transition['trigger_event']),
        from: [nameOf(asText(transition['from_phase']))],
        to: nameOf(asText(transition['to_phase'])),
        ...(guards.length === 0 ? {} : { guard: guards.join(' && ') }),
      }
    })
  const entries = [{
    entity: asText(lifecycle.name),
    state_field: 't_state',
    states: phases.map(phase => ({ name: asText(phase['name']), description: asText(phase['id']) })),
    transitions,
    initial_state: asText(phases[0]?.['name']),
  }]
  return header('statemachines', code, 'plan.ontologyModelJson.transaction_lifecycle') + dump(entries)
}

/** `workflows.yaml` ← `plan.workflowSteps`（上游 `workflow_from_steps` 逐字段同形）。 */
function workflowsYaml(code: string, plan: ExtensionPlanInput): string {
  const steps = plan.workflowSteps ?? []
  if (steps.length === 0) {
    return skeleton('workflows', code, 'plan.workflowSteps 为空')
  }
  const entries = [{
    name: 'auto_workflow',
    description: 'Auto-generated workflow from LLM steps',
    trigger: { entity: '*', event: 'onCreate' },
    steps: steps.map((step, index) => ({
      name: `step_${index + 1}`,
      action: { type: 'call_procedure', name: step, params: [] },
      on_error: 'Abort',
    })),
  }]
  return header('workflows', code, 'plan.workflowSteps') + dump(entries)
}

/**
 * `profiles.yaml` ← 声明的模块集（上游 `ProfilesWrapper` 契约：`profiles.<name>.modules.<id>`）。
 * 无模块 ⇒ 不产出该文件（上游同规：不写空档案）。
 * @returns the YAML body, or `null` when the file must not be written.
 */
function profilesYaml(code: string, plan: ExtensionPlanInput): string | null {
  const modules = plan.modules ?? []
  if (modules.length === 0) {
    return null
  }
  const registry = Object.fromEntries(modules.map(id => [id, { enabled_entities: [], disabled_entities: [] }]))
  const body = { profiles: { default: { modules: registry } } }
  return header('profiles', code, 'plan.modules') + dump(body)
}

/**
 * Extension files for an app: plan-derived content where the plan carries it, skeletons otherwise.
 * @param code - app code (goes into the generated header).
 * @param plan - the flow plan subset (omit for the pure skeleton tree).
 * @returns `{ '<kind>.yaml': body }` — the four loader forms plus `profiles.yaml` when modules are declared.
 */
export function generateExtensions(code: string, plan: ExtensionPlanInput = {}): Readonly<Record<string, string>> {
  const files: Record<string, string> = {
    'constraints.yaml': constraintsYaml(code, plan),
    'rules.yaml': rulesYaml(code, plan),
    'statemachines.yaml': statemachinesYaml(code, plan),
    'workflows.yaml': workflowsYaml(code, plan),
  }
  const profiles = profilesYaml(code, plan)
  if (profiles !== null) {
    files['profiles.yaml'] = profiles
  }
  return files
}
