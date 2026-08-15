/**
 * PTC orchestrator: `alioth_app_create` — a programmatic Tool Calling
 * pipeline. The sequence is fixed by code (semantic alignment is a
 * PRE-condition the model completes in dialogue via alioth_schema_* tools and
 * passes in as parameters): validate inputs → register missing entities →
 * write the artifact tree → read back and verify. Every step runs through
 * `ctx.tools.execute`, the same path the model uses — approvals, gates, and
 * the session log all apply. No LLM calls inside; the model appears only as
 * the caller and as the semantic-alignment step before the call.
 * @module @dsh-alioth/tool-alioth-orchestrator
 */

import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool, type ToolRunContext } from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'

export const name = 'tool-alioth-orchestrator'
export const inject = ['tools', 'aliothEnv']

export interface Config {}

export const Config: z<Config> = z.object({})

/** Execute one registered tool through the registry (model-equivalent path). */
async function runTool(
  ctx: Context,
  exec: ToolRunContext,
  toolName: string,
  args: unknown,
): Promise<Record<string, unknown>> {
  const result = await ctx.tools.execute({
    signal: exec.signal,
    callId: CallId(`${toolName}-orchestrated`),
    name: toolName,
    arguments: args,
    ...(exec.agent === undefined ? {} : { agent: exec.agent }),
  })
  if (result.isError) {
    throw new Error(`alioth_app_create: step ${toolName} failed: ${result.error.message}`)
  }
  return result.value as Record<string, unknown>
}

export function apply(ctx: Context, _config: Config): void {
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
        description: 'Alioth namespace, e.g. "Alioth".',
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
      // Phase 1 — deterministic validation happens inside each step tool; here
      // we just sequence them. Entities first: they may be referenced by
      // nothing later, but a failed entity write aborts the whole pipeline.
      let entitiesRegistered = 0
      for (const entity of args.entities ?? []) {
        await runTool(ctx, exec, 'alioth_entity_write', {
          table: entity.table,
          name: entity.name,
          ...(entity.inherits === undefined ? {} : { inherits: entity.inherits }),
          ...(entity.category === undefined ? {} : { category: entity.category }),
          ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
          fields: entity.fields ?? [],
        })
        entitiesRegistered += 1
      }

      // Phase 2 — artifact tree (contract gate inside alioth_app_write).
      const written = await runTool(ctx, exec, 'alioth_app_write', {
        namespace: args.namespace,
        code: args.code,
        name: args.name,
        modules: args.modules,
        ...(args.blocks === undefined ? {} : { blocks: args.blocks }),
      })

      // Phase 3 — verify by reading back.
      const inspected = await runTool(ctx, exec, 'alioth_app_inspect', {
        namespace: args.namespace,
        app: args.code,
      })
      const missing = Array.isArray(inspected.missing) ? inspected.missing as string[] : []
      const filesWritten = Array.isArray(written.files) ? written.files.length : 0
      return {
        namespace: args.namespace,
        code: args.code,
        entitiesRegistered,
        filesWritten,
        verified: missing.length === 0,
        summary: missing.length === 0
          ? `app ${args.namespace}/${args.code} verified against the registry`
          : `app ${args.namespace}/${args.code} missing: ${missing.join(', ')}`,
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
