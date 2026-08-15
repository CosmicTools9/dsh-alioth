/**
 * Model-facing AppAgent workflow tools. The skill-adapter tracks/steps/gates
 * (from the model snapshot's `skill-adapters/*.yaml`) become a driveable
 * dialogue flow: `alioth_workflow_step` shows the current step's instruction
 * and gates; `alioth_workflow_complete` runs the gates (artifact globs +
 * external programs), advances the deterministic state machine, and returns
 * the next step. No LLM inside this path — the model executes the steps.
 * @module @dsh-alioth/tool-alioth-workflow
 */

import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'
import {
  checkStepGates,
  completeCurrentStep,
  createProgramRunner,
  currentStep,
  loadAdapter,
  loadRun,
  saveRun,
  type Adapter,
  type GateContext,
  type RunState,
} from '@dsh-alioth/skill-alioth'

export const name = 'tool-alioth-workflow'
export const inject = ['tools', 'aliothEnv']

/** Deployment choices for the workflow bridge. */
export interface Config {
  /** Pre-Proc artifact tree root; gate globs resolve under it (env ALIOTH_PRE_PROC_ROOT). */
  readonly preProcRoot: string
  /** Adapter file under the snapshot's `skill-adapters/` (default alioth-app.yaml). */
  readonly adapter?: string
  /** Run-state root; default `<dataRoot>/workflows`. */
  readonly workflowRoot?: string
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  adapter: z.string().default('alioth-app.yaml'),
  workflowRoot: z.string(),
})

const NAMESPACE_PATTERN_RE = /^[A-Z][a-zA-Z0-9-]*$/
const APP_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

function assertNsApp(namespace: string, app: string): void {
  if (!NAMESPACE_PATTERN_RE.test(namespace)) {
    throw new Error(`alioth_workflow: invalid namespace ${JSON.stringify(namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
  }
  if (!APP_PATTERN_RE.test(app)) {
    throw new Error(`alioth_workflow: invalid app code ${JSON.stringify(app)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$)`)
  }
}

export function apply(ctx: Context, config: Config): void {
  const adapterName = config.adapter ?? 'alioth-app.yaml'
  const preProcRoot = path.resolve(config.preProcRoot)
  const adapterCache = new Map<string, Adapter>()

  async function adapterFor(): Promise<Adapter> {
    const cached = adapterCache.get(adapterName)
    if (cached !== undefined) {
      return cached
    }
    const info = await ctx.aliothEnv.ready()
    const adapter = await loadAdapter(info.modelDir, adapterName)
    adapterCache.set(adapterName, adapter)
    return adapter
  }

  async function stateFor(namespace: string, app: string): Promise<RunState> {
    const adapter = await adapterFor()
    const workflowRoot = config.workflowRoot ?? path.join(ctx.aliothEnv.dataRoot(), 'workflows')
    return loadRun(workflowRoot, { namespace, app }, adapter)
  }

  function gateContext(namespace: string, app: string): GateContext {
    return { preProcRoot, variables: { ns: namespace, app } }
  }

  ctx.tools.register(defineTool({
    name: 'alioth_workflow_step',
    description:
      `Show the current AppAgent workflow step for an app (adapter ${adapterName}): the step's `
      + 'instruction, allowed tools, and gates. Call this at the start of each step and after '
      + 'alioth_workflow_complete advances. A finished run returns finished=true.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'Alioth namespace, e.g. "Alioth".',
      },
      app: {
        type: 'string',
        required: true,
        description: 'App code (directory under Apps/).',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          finished: { type: 'boolean', required: true },
          track: { type: 'string', required: true },
          stepId: { type: 'string', required: true },
          instruction: { type: 'string', required: true },
          tools: { type: 'array', required: true, items: { type: 'string' } },
          gates: { type: 'array', required: true, items: { type: 'string' } },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.finished
          ? 'Workflow finished'
          : `[${String(value.track)}] step ${String(value.stepId)}: ${String(value.instruction)}`,
      }],
    },
    async execute(args) {
      assertNsApp(args.namespace, args.app)
      const state = await stateFor(args.namespace, args.app)
      const current = currentStep(state)
      if (current === undefined) {
        return { finished: true, track: '', stepId: '', instruction: '', tools: [], gates: [] }
      }
      return {
        finished: false,
        track: current.track.name,
        stepId: current.step.id,
        instruction: current.step.instruction,
        tools: [...current.step.tools],
        gates: current.step.gates.map(gate => gate.kind === 'output-glob'
          ? `output_glob: ${gate.outputGlob}`
          : `program: ${gate.program} ${gate.args.join(' ')}`),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Workflow step ${args.namespace}/${args.app}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_workflow_complete',
    description:
      `Run the current workflow step's gates for an app (adapter ${adapterName}): artifact globs are `
      + 'checked on disk, program gates execute through the deployment runner. All gates must pass; '
      + 'the state machine then advances and the next step is returned. On gate failure nothing '
      + 'advances and every failed gate is listed.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'Alioth namespace, e.g. "Alioth".',
      },
      app: {
        type: 'string',
        required: true,
        description: 'App code (directory under Apps/).',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          finished: { type: 'boolean', required: true },
          completedStep: { type: 'string', required: true },
          gateResults: {
            type: 'array',
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                ok: { type: 'boolean', required: true },
                detail: { type: 'string', required: true },
              },
            },
          },
          nextStep: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.finished
          ? `Completed ${String(value.completedStep)} — workflow finished`
          : `Completed ${String(value.completedStep)} — next: ${String(value.nextStep)}`,
      }],
    },
    async execute(args) {
      assertNsApp(args.namespace, args.app)
      const state = await stateFor(args.namespace, args.app)
      const current = currentStep(state)
      if (current === undefined) {
        return { finished: true, completedStep: '', gateResults: [], nextStep: '' }
      }
      const context = gateContext(args.namespace, args.app)
      const runner = createProgramRunner({ cwd: preProcRoot })
      const results = await checkStepGates(current.step.gates, context, runner)
      if (results.some(result => !result.ok)) {
        throw new Error(`alioth_workflow: gates failed for step ${current.step.id}:\n`
          + results.filter(result => !result.ok).map(result => `- ${result.detail}`).join('\n'))
      }
      const advanced = completeCurrentStep(state)
      const workflowRoot = config.workflowRoot ?? path.join(ctx.aliothEnv.dataRoot(), 'workflows')
      await saveRun(workflowRoot, { namespace: args.namespace, app: args.app }, advanced.state)
      const next = currentStep(advanced.state)
      return {
        finished: advanced.transition.finished,
        completedStep: current.step.id,
        gateResults: results.map(result => ({ ok: result.ok, detail: result.detail })),
        nextStep: next === undefined ? '' : next.step.id,
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Complete workflow step ${args.namespace}/${args.app}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
