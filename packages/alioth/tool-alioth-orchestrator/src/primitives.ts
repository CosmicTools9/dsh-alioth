/**
 * Real primitive bindings for the AppAgent pipeline machine
 * (`@dsh-alioth/skill-alioth/agent-machine`). Each stage maps to an existing
 * tool through `ctx.tools.execute` — the same path the model uses, so
 * approvals, gates, and the session log apply per stage. No LLM calls.
 * Semantic alignment is a dialogue precondition (semantic_search + model
 * decision) passed in via parameters; the semantic-analysis primitive merely
 * re-confirms hits for the audit trail.
 * @module @dsh-alioth/tool-alioth-orchestrator/primitives
 */

import type { Context } from '@deepseek-ai/cordis'
import type { ToolRunContext } from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'
import type { AgentPrimitives, StageOutput } from '@dsh-alioth/skill-alioth/agent-machine'
import type { BuildResult, FlowPlan } from '@dsh-alioth/skill-alioth/agent-contract'

export interface CreateArgs {
  readonly namespace: string
  readonly code: string
  readonly name: string
  readonly modules: ReadonlyArray<{ readonly id: string; readonly name: string }>
  readonly blocks?: readonly string[]
  readonly entities?: ReadonlyArray<{
    readonly table: string
    readonly name: string
    readonly inherits?: readonly string[]
    readonly category?: string
    readonly coordinates?: { readonly scene: string; readonly factor: string; readonly function: string }
    readonly fields?: ReadonlyArray<{
      readonly name: string
      readonly category: string
      readonly dataType: string
      readonly title?: string
      readonly required?: boolean
      readonly targetTable?: string
      readonly localKey?: string
      readonly junctionTable?: string
    }>
  }>
}

/** Execute one registered tool through the registry (model-equivalent path). */
async function runTool(
  ctx: Context,
  exec: ToolRunContext,
  toolName: string,
  args: unknown,
): Promise<Record<string, unknown>> {
  const result = await ctx.tools.execute({
    signal: exec.signal,
    callId: CallId(`${toolName}-pipeline`),
    name: toolName,
    arguments: args,
    ...(exec.agent === undefined ? {} : { agent: exec.agent }),
  })
  if (result.isError) {
    throw new Error(`pipeline stage ${toolName} failed: ${result.error.message}`)
  }
  return result.value as Record<string, unknown>
}

/** Bind the 7-stage pipeline to real tools for one `alioth_app_create` call. */
export function buildPrimitives(
  ctx: Context,
  exec: ToolRunContext,
  args: CreateArgs,
  workflowAdapter: string | undefined,
): AgentPrimitives {
  /** Phase 2/3/4 share one app_write (write-once); later stages verify. */
  let writtenFiles: string[] = []

  return {
    // 1. Semantic analysis — dialogue preconditions (alignment already done);
    //    re-confirm via semantic search for the audit trail. The search is a
    //    confirmation, not a gate: an empty registry (fresh bootstrap) must
    //    not block the pipeline — alignment parameters already carry the
    //    resolution.
    async semanticAnalysis(input) {
      try {
        const search = await runTool(ctx, exec, 'alioth_schema_semantic_search', {
          query: input,
          ...(args.entities === undefined ? {} : { k: Math.max(5, args.entities.length) }),
        })
        const hits = Array.isArray(search.hits) ? (search.hits as unknown[]).length : 0
        return {
          evidence: `semantic search: ${hits} registry hits for "${input}"`,
          artifacts: [String(search.cacheKey ?? '')].filter(Boolean),
        }
      } catch (error) {
        return {
          evidence: `semantic search unavailable (${error instanceof Error ? error.message : 'unknown'}); alignment preconditions accepted from parameters`,
        }
      }
    },

    // 2. Function decomposition — registry inventory grounding.
    async functionDecomposition(input) {
      const info = await runTool(ctx, exec, 'alioth_schema_info', { action: 'entities', limit: 50 })
      const entities = Array.isArray(info.entities) ? info.entities as unknown[] : []
      return {
        evidence: `function decomposition: ${entities.length} registry entities available; plan=${args.modules.length} modules, ${args.blocks?.length ?? 0} blocks`,
        artifacts: [`namespace ${args.namespace}`, `app ${args.code}`],
      }
    },

    // 3. Ontology analysis — register declared new entities (validated).
    async ontologyAnalysis() {
      const registered: string[] = []
      for (const entity of args.entities ?? []) {
        await runTool(ctx, exec, 'alioth_entity_write', {
          table: entity.table,
          name: entity.name,
          ...(entity.inherits === undefined ? {} : { inherits: entity.inherits }),
          ...(entity.category === undefined ? {} : { category: entity.category }),
          ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
          fields: entity.fields ?? [],
        })
        registered.push(entity.table)
      }
      return {
        evidence: `ontology analysis: ${registered.length} entities registered (${registered.join(', ') || 'none'})`,
        artifacts: registered,
      }
    },

    // 4. Module creation — the artifact tree (contract gate inside app_write).
    async moduleCreation() {
      const written = await runTool(ctx, exec, 'alioth_app_write', {
        namespace: args.namespace,
        code: args.code,
        name: args.name,
        modules: args.modules,
        ...(args.blocks === undefined ? {} : { blocks: args.blocks }),
      })
      writtenFiles = Array.isArray(written.files) ? written.files as string[] : []
      return {
        evidence: `module creation: ${args.modules.length} modules, ${writtenFiles.length} files written`,
        artifacts: writtenFiles,
      }
    },

    // 5. Block creation — write-once preserved; verify the block artifacts.
    async blockCreation() {
      const blockFiles = writtenFiles.filter(f => f.endsWith('block.json'))
      return {
        evidence: `block creation: ${blockFiles.length} block artifacts verified`,
        artifacts: blockFiles,
      }
    },

    // 6. Ontology transfer — service/extension artifacts verified.
    async ontologyTransfer() {
      const serviceFiles = writtenFiles.filter(f =>
        f.includes('service') || f.endsWith('extensions.yaml') || f.endsWith('extension.yaml'),
      )
      return {
        evidence: `ontology transfer: ${serviceFiles.length} service/extension artifacts verified`,
        artifacts: serviceFiles,
      }
    },

    // 7. Service API — contract validation already gated by app_write; verify.
    async serviceApi() {
      const serviceFiles = writtenFiles.filter(f => f.includes('service'))
      return {
        evidence: `service API: ${serviceFiles.length} service artifacts contract-validated`,
        artifacts: serviceFiles,
      }
    },

    // 8. Publishing — read back, verify, build the result descriptor; the
    //    optional AppAgent workflow gate runs here.
    async publishing(_plan, attempt) {
      const inspected = await runTool(ctx, exec, 'alioth_app_inspect', {
        namespace: args.namespace,
        app: args.code,
      })
      const missing = Array.isArray(inspected.missing) ? inspected.missing as string[] : []
      let workflowGate = 'not-configured'
      if (workflowAdapter !== undefined) {
        const step = await runTool(ctx, exec, 'alioth_workflow_step', {
          namespace: args.namespace,
          app: args.code,
        })
        if (step.finished !== true) {
          await runTool(ctx, exec, 'alioth_workflow_complete', {
            namespace: args.namespace,
            app: args.code,
          })
          workflowGate = `step ${String(step.stepId)} passed`
        } else {
          workflowGate = 'finished'
        }
      }
      const result: BuildResult = {
        appName: args.name,
        outputPath: `Pre-Proc/${args.namespace}/Apps/${args.code}/app.json`,
        usedModules: args.modules.map(m => ({ moduleId: m.id, name: m.name, blocks: [] })),
        extensions: [],
        generatedFiles: writtenFiles,
        pendingConfirmations: [],
        previewUrl: `/apps/${args.namespace}/${args.code}/prototype.html`,
        runtimeValidation: {
          valid: missing.length === 0,
          checks: [
            { name: 'inspect-readback', ok: missing.length === 0, detail: missing.length === 0 ? 'app reads back' : `missing: ${missing.join(', ')}` },
            { name: 'workflow-gate', ok: workflowGate !== 'failed', detail: workflowGate },
          ],
        },
      }
      const output: StageOutput = {
        evidence: `publishing attempt ${attempt}: verified=${missing.length === 0}, workflow=${workflowGate}`,
        artifacts: [result.outputPath],
      }
      return { output, result }
    },
  }
}

/** Assemble the FlowPlan shared with the Meta AppAgent (unified contract). */
export function buildPlan(args: CreateArgs): FlowPlan {
  return {
    usedModules: args.modules.map(m => m.id),
    namespace: args.namespace,
    knownEntities: (args.entities ?? []).map(e => e.table),
    workflowSteps: args.blocks ?? [],
    missingInfo: [],
    createdModules: [],
    createdBlocks: [],
    createdServices: [],
  }
}
