/**
 * PTC orchestrator: `alioth_app_create` — the complete AppAgent pipeline
 * driven deterministically. The 7-stage machine (semantic analysis → function
 * decomposition → ontology analysis → module/block creation → ontology
 * transfer → service API → publishing) runs through `ctx.tools.execute` —
 * the same path the model uses, so approvals, gates, and the session log
 * apply per stage. No LLM calls inside; semantic alignment is a PRE-condition
 * the model completes in dialogue via alioth_schema_* tools and passes in as
 * parameters (re-confirmed by the semantic-analysis stage for the audit
 * trail). Data contracts are unified with the Meta AppAgent
 * (`@dsh-alioth/skill-alioth/agent-contract`).
 * @module @dsh-alioth/tool-alioth-orchestrator
 */

import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { runPipeline } from '@dsh-alioth/skill-alioth/agent-machine'
import { stageOf } from '@dsh-alioth/skill-alioth/agent-machine'
import { buildPlan, buildPrimitives } from './primitives.ts'

export const name = 'tool-alioth-orchestrator'
export const inject = ['tools', 'aliothEnv']

export interface Config {
  /**
   * When set, `alioth_app_create` runs the AppAgent workflow gate after
   * writing artifacts: it opens a run state for the adapter and executes the
   * first step's gates via alioth_workflow_complete. Gate failure fails the
   * create (artifacts stay; fix them and re-run workflow_complete).
   */
  readonly adapter?: string
}

export const Config: z<Config> = z.object({
  adapter: z.string(),
})


export function apply(ctx: Context, config: Config): void {
  const adapterName = config.adapter
  ctx.tools.register(defineTool({
    name: 'alioth_app_create',
    description:
      'Programmatic end-to-end app creation: validates inputs, registers any missing entities '
      + '(alioth_entity_write), writes the contract-validated artifact tree (alioth_app_write), '
      + 'and verifies by reading it back (alioth_app_inspect). All steps run through the tool '
      + 'registry — approvals and gates apply per step. Semantic alignment is NOT done here: '
      + 'align concepts to registry entities with alioth_schema_semantic_search first and pass '
      + 'the resolved entity references as `entities`. Fails atomically before writing when any '
      + 'referenced entity does not exist and is not declared.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first.',
      },
      code: {
        type: 'string',
        required: true,
        description: 'App code (directory under Apps/).',
      },
      name: {
        type: 'string',
        required: true,
        description: 'App display name.',
      },
      modules: {
        type: 'array',
        required: true,
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            id: { type: 'string', required: true },
            name: { type: 'string', required: true },
            description: { type: 'string' },
            icon: { type: 'string' },
          },
        },
        description: 'Module specs (as alioth_app_write).',
      },
      blocks: {
        type: 'array',
        items: { type: 'string' },
      },
      entities: {
        type: 'array',
        items: {
          type: 'object',
          additionalProperties: false,
          properties: {
            table: { type: 'string', required: true },
            name: { type: 'string', required: true },
            inherits: { type: 'array', items: { type: 'string' } },
            category: { type: 'string' },
            coordinates: {
              type: 'object',
              additionalProperties: false,
              properties: {
                scene: { type: 'string', required: true },
                factor: { type: 'string', required: true },
                function: { type: 'string', required: true },
              },
            },
            fields: {
              type: 'array',
              items: {
                type: 'object',
                additionalProperties: false,
                properties: {
                  name: { type: 'string', required: true },
                  category: { type: 'string', required: true },
                  dataType: { type: 'string', required: true },
                  title: { type: 'string' },
                  required: { type: 'boolean' },
                  targetTable: { type: 'string' },
                  localKey: { type: 'string' },
                  junctionTable: { type: 'string' },
                },
              },
            },
          },
        },
        description: 'New entities to register (alioth_entity_write semantics). Existing entities: reference them implicitly via field targetTable.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          code: { type: 'string', required: true },
          entitiesRegistered: { type: 'number', required: true },
          filesWritten: { type: 'number', required: true },
          verified: { type: 'boolean', required: true },
          workflowGate: { type: 'string', required: true },
          stages: { type: 'array', items: { type: 'string' }, required: true },
          summary: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Created ${value.namespace}/${value.code}: ${value.entitiesRegistered} entities registered, `
          + `${value.filesWritten} files written, verified=${value.verified}`,
      }],
    },
    async execute(args, exec) {
      // The full pipeline: each stage runs a registered tool through the
      // registry (deterministic, zero LLM). Stage history is returned for
      // audit; the terminal state decides success.
      const plan = buildPlan(args)
      const run = await runPipeline('app creation request', buildPrimitives(ctx, exec, args, adapterName), plan)
      const transitions = run.history
      const stages = transitions.map(t => `${t.from.kind}->${stageOf(t.to) ?? t.to.kind}`)
      if (run.state.kind !== 'published') {
        const reason = run.state.kind === 'failed'
          ? `pipeline failed at ${stages.at(-1)}: ${run.state.error ?? 'unknown'}`
          : `pipeline ended at ${stages.at(-1)}`
        throw new Error(`alioth_app_create: ${reason}`)
      }
      const result = run.state.result
      const workflowGate = result.runtimeValidation?.checks
        .find(check => check.name === 'workflow-gate')?.detail ?? 'not-configured'
      const verified = result.runtimeValidation?.valid ?? false

      return {
        namespace: args.namespace,
        code: args.code,
        entitiesRegistered: plan.knownEntities.length,
        filesWritten: result.generatedFiles.length,
        verified,
        workflowGate,
        stages,
        summary: verified
          ? `app ${args.namespace}/${args.code} published (${stages.length} stages: ${stages.join(' -> ')})`
          : `app ${args.namespace}/${args.code} published with validation warnings (${stages.join(' -> ')})`,
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Create Alioth app ${args.namespace}/${args.code}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
