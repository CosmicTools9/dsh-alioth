import { describe, expect, it } from 'vitest'
import { parse } from 'yaml'
import { EXTENSION_FILES, generateExtensions, type ExtensionPlanInput } from '../src/index.ts'

/**
 * 计划驱动的扩展生成（上游 `composer.rs::compose_from_flow_plan` 的扩展段）。
 * 断言面 = **反序列化后**的字段与枚举名（写错枚举 = Gateway 加载器反序列化失败 = 声明从未进入运行时）。
 */
const CONSTRAINTS: NonNullable<ExtensionPlanInput['constraints']> = [
  { entity: 'zc_id_unit', expression: 'qty > 0', message: '数量须为正', level: 'error' },
  { entity: 'zc_id_unit', field: 'price', expression: 'price >= 0', message: '价格不得为负', level: 'warning' },
]
const RULES: NonNullable<ExtensionPlanInput['businessRules']> = [
  { entity: 'zc_id_unit', ruleName: 'discount', trigger: 'onCreate', condition: 'qty > 100', action: 'discount_rate = 0.15', priority: 5, errorMessage: '' },
]

const PLAN: ExtensionPlanInput = {
  constraints: CONSTRAINTS,
  businessRules: RULES,
  workflowSteps: ['reserve', 'ship'],
  modules: ['inventory'],
  ontologyModelJson: JSON.stringify({
    transaction_lifecycle: {
      name: 'zc_id_bill',
      phases: [{ id: 'p1', name: 'Created' }, { id: 'p2', name: 'Confirmed' }],
      transitions: [{ trigger_event: 'confirm', from_phase: 'p1', to_phase: 'p2', guard_conditions: ['amount > 0', 'stock > 0'] }],
    },
  }),
}

describe('generateExtensions (plan-driven)', () => {
  it('emits skeletons only when the plan carries nothing', () => {
    const files = generateExtensions('demo')
    expect(Object.keys(files).sort()).toEqual([...EXTENSION_FILES].map(kind => `${kind}.yaml`))
    for (const body of Object.values(files)) {
      expect(body).toContain('dsh-alioth generated skeleton')
      expect(parse(body)).toEqual([])
    }
    // 无模块 ⇒ 不产出 profiles.yaml（上游同规：不写空档案）
    expect(files['profiles.yaml']).toBeUndefined()
  })

  it('derives constraints with the upstream severity spelling', () => {
    const entries = parse(generateExtensions('demo', PLAN)['constraints.yaml']!) as Array<Record<string, unknown>>
    expect(entries).toHaveLength(2)
    expect(entries[0]).toEqual({ entity: 'zc_id_unit', expression: 'qty > 0', level: 'Error', message: '数量须为正' })
    // `field` 是可选项：给了才出现；`warning` 只在这一档出现。
    expect(entries[1]?.['field']).toBe('price')
    expect(entries[1]?.['level']).toBe('Warning')
  })

  it('derives rules with the loader-required keys', () => {
    const entries = parse(generateExtensions('demo', PLAN)['rules.yaml']!) as Array<Record<string, unknown>>
    expect(Object.keys(entries[0]!).sort()).toEqual([
      'action', 'blocking', 'condition', 'entity', 'error_message', 'name', 'priority', 'trigger',
    ])
    expect(entries[0]).toMatchObject({ name: 'discount', trigger: 'onCreate', blocking: true, priority: 5 })
  })

  it('derives the state machine from transaction_lifecycle', () => {
    const entries = parse(generateExtensions('demo', PLAN)['statemachines.yaml']!) as Array<Record<string, unknown>>
    expect(entries[0]).toMatchObject({ entity: 'zc_id_bill', state_field: 't_state', initial_state: 'Created' })
    expect(entries[0]?.['states']).toEqual([
      { name: 'Created', description: 'p1' },
      { name: 'Confirmed', description: 'p2' },
    ])
    // 相位 id 映射成 name；guards 以 " && " 相联（上游 join_guards）。
    expect(entries[0]?.['transitions']).toEqual([
      { event: 'confirm', from: ['Created'], to: 'Confirmed', guard: 'amount > 0 && stock > 0' },
    ])
  })

  it('derives one auto workflow from the plan steps', () => {
    const entries = parse(generateExtensions('demo', PLAN)['workflows.yaml']!) as Array<Record<string, unknown>>
    expect(entries[0]).toMatchObject({
      name: 'auto_workflow',
      trigger: { entity: '*', event: 'onCreate' },
    })
    expect(entries[0]?.['steps']).toEqual([
      { name: 'step_1', action: { type: 'call_procedure', name: 'reserve', params: [] }, on_error: 'Abort' },
      { name: 'step_2', action: { type: 'call_procedure', name: 'ship', params: [] }, on_error: 'Abort' },
    ])
  })

  it('derives the profiles registry from the declared modules', () => {
    const body = parse(generateExtensions('demo', PLAN)['profiles.yaml']!) as Record<string, unknown>
    expect(body).toEqual({ profiles: { default: { modules: { inventory: { enabled_entities: [], disabled_entities: [] } } } } })
  })

  it('keeps a form as a skeleton when its own source is empty, and says so', () => {
    const files = generateExtensions('demo', { constraints: CONSTRAINTS })
    expect(files['constraints.yaml']).toContain('generated from the flow plan')
    expect(files['rules.yaml']).toContain('plan.businessRules 为空')
    expect(parse(files['rules.yaml']!)).toEqual([])
  })

  it('degrades honestly when the ontology payload is unusable', () => {
    for (const ontologyModelJson of ['{not json', JSON.stringify({ other: 1 }), JSON.stringify({ transaction_lifecycle: { phases: [] } })]) {
      const body = generateExtensions('demo', { ontologyModelJson })['statemachines.yaml']!
      expect(body).toContain('dsh-alioth generated skeleton')
      expect(parse(body)).toEqual([])
    }
  })
})
