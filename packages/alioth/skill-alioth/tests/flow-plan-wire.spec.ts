import { describe, expect, it } from 'vitest'
import type { FlowPlan } from '../src/agent-contract.ts'
import { flowPlanFromWire, flowPlanToWire } from '../src/flow-plan-wire.ts'

/**
 * flow-plan 的 wire 面（上游 `state.rs::FlowPlan` 的 serde 名）：写出 snake_case，
 * 读入接受旧别名；缺必需键即 `null`（fail-closed，不猜默认值）。
 */
const plan: FlowPlan = {
  usedModules: ['inventory'],
  namespace: 'Demo',
  knownEntities: ['zc_id_unit'],
  workflowSteps: ['in'],
  missingInfo: [],
  createdModules: ['inventory'],
  createdBlocks: ['orders-board'],
  createdServices: [],
  semanticConcepts: ['库存', '盘点'],
  coreConstraints: ['币种：全程为 CNY'],
  computations: [{ entity: 'zc_id_unit', targetField: 'amount', formula: 'qty * price', dependsOn: ['qty', 'price'], trigger: 'onCreate' }],
}

describe('flowPlanToWire / flowPlanFromWire', () => {
  it('emits upstream snake_case keys and round-trips', () => {
    const wire = flowPlanToWire(plan)
    expect(Object.keys(wire)).toEqual(expect.arrayContaining([
      'used_modules', 'namespace', 'known_entities', 'workflow_steps', 'missing_info',
      'created_modules', 'created_blocks', 'created_services', 'semantic_concepts', 'core_constraints', 'computations',
    ]))
    expect(wire).not.toHaveProperty('usedModules')
    const back = flowPlanFromWire(JSON.parse(JSON.stringify(wire)))
    expect(back).toMatchObject({ namespace: 'Demo', usedModules: ['inventory'], semanticConcepts: ['库存', '盘点'] })
  })

  it('does not descend into the stringified ontology payload', () => {
    const wire = flowPlanToWire({ ...plan, ontologyModelJson: '{"someKey":1}' })
    expect(wire['ontology_model_json']).toBe('{"someKey":1}')
  })

  it('accepts the legacy aliases created_scenes / created_factors', () => {
    const legacy = {
      used_modules: [], namespace: 'Demo', known_entities: [], workflow_steps: [], missing_info: [],
      // 别名只覆盖 created_scenes/created_factors：created_modules 仍是必需键（缺即 fail-closed）。
      created_modules: [],
      created_scenes: ['orders-board'], created_factors: ['svc-a'],
    }
    const parsed = flowPlanFromWire(legacy)
    expect(parsed?.createdBlocks).toEqual(['orders-board'])
    expect(parsed?.createdServices).toEqual(['svc-a'])
  })

  it('fails closed on a missing required key', () => {
    expect(flowPlanFromWire({ namespace: 'Demo' })).toBeNull()
    expect(flowPlanFromWire([])).toBeNull()
    expect(flowPlanFromWire(null)).toBeNull()
  })
})
