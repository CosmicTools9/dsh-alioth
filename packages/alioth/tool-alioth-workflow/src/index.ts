/**
 * Model-facing AppAgent workflow tools. The skill-adapter tracks/steps/gates
 * (from the model snapshot's `skill-adapters/*.yaml`) become a driveable
 * dialogue flow: `alioth_workflow_step` shows the current step's instruction
 * and gates; `alioth_workflow_complete` runs the gates (artifact globs +
 * external programs), advances the deterministic state machine, and returns
 * the next step. No LLM inside this path — the model executes the steps.
 * @module @dsh-alioth/tool-alioth-workflow
 */
import type { Context } from '@deepseek-ai/cordis'
import { provisionPrototypeRoot } from '@dsh-alioth/env-alioth'
import path from 'node:path'
import { readFile } from 'node:fs/promises'
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'
import {
  checkStepGates,
  completeCurrentStep,
  createProgramRunner,
  currentStep,
  isLlmFixable,
  loadAdapter,
  loadRun,
  parseRuntimeAllowedPrograms,
  saveRun,
  type Adapter,
  type GateContext,
  type RunState,
  type Step,
  type StepGate,
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
  // Content root (upstream repo-root layout): the dir containing Pre-Proc/ +
  // the provisioned `.agents/` references, `Framework/` utilities and gate
  // `scripts/`. Program gates run here; PROTOTYPE_TOOL_ROOT points here.
  const contentRoot = path.dirname(preProcRoot)
  let provisioned = false
  function ensureContentRoot(): { contentRoot: string } {
    if (!provisioned) {
      provisionPrototypeRoot(preProcRoot, contentRoot)
      provisioned = true
    }
    return { contentRoot }
  }
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
    // Adapter templates use per-track aliases ({service}/{crate}/{block}) for
    // the run's subject; they all key off the run's {ns}/{app} pair. {crate}
    // follows the scaffold's crate naming convention.
    return {
      preProcRoot,
      variables: {
        ns: namespace,
        app,
        service: app,
        block: app,
        crate: `alioth-service-${app}`,
      },
    }
  }

  /** Human-facing gate summary: glob form, or program form with contract fields. */
  function formatGates(gates: readonly StepGate[]): string[] {
    return gates.map(gate => {
      if (gate.kind === 'output-glob') {
        return `output_glob: ${gate.outputGlob}`
      }
      const exit = gate.expectedExitCode === 0 ? '' : ` expected_exit=${gate.expectedExitCode}`
      return `program: ${gate.program} ${gate.args.join(' ')}${exit} timeout=${gate.timeoutSec}s`
    })
  }

  const MAX_INPUT_CHARS = 4000

  /** Engine-injected step inputs (upstream `Step.inputs`): the engine reads
   * each template path under preProcRoot so the model doesn't have to
   * explore; unreadable files are reported without content. */
  async function readStepInputs(step: Step, context: GateContext): Promise<{ path: string; content?: string }[]> {
    const resolved: { path: string; content?: string }[] = []
    for (const template of step.inputs) {
      const target = template.replace(/\{(\w+)\}/g, (match, key: string) => context.variables[key] ?? match)
      const candidate = target.startsWith('Pre-Proc/')
        ? path.join(context.preProcRoot, target.slice('Pre-Proc/'.length))
        : path.resolve(context.preProcRoot, target)
      const content = candidate.startsWith(path.resolve(context.preProcRoot) + path.sep)
        ? await readFile(candidate, 'utf8')
          .then(text => text.length > MAX_INPUT_CHARS ? `${text.slice(0, MAX_INPUT_CHARS)}\n…(truncated)` : text)
          .catch(() => undefined)
        : undefined
      resolved.push(content === undefined ? { path: target } : { path: target, content })
    }
    return resolved
  }

  ctx.tools.register(defineTool({
    name: 'alioth_workflow_info',
    description:
      `Introspect the AppAgent workflow definition for this deployment (adapter ${adapterName}): `
      + 'every track with its steps — instruction, allowed tools, gates — plus the runtime program '
      + 'allowlist. This is the sanctioned way to view the flow: NEVER read adapter or vendor files '
      + 'with filesystem tools. Drive the flow step by step with alioth_workflow_step / '
      + 'alioth_workflow_complete.',
    parameters: {},
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          adapter: { type: 'string', required: true },
          tracks: {
            type: 'array', required: true,
            items: {
              type: 'object', additionalProperties: false,
              properties: {
                id: { type: 'string', required: true },
                name: { type: 'string', required: true },
                steps: {
                  type: 'array', required: true,
                  items: {
                    type: 'object', additionalProperties: false,
                    properties: {
                      id: { type: 'string', required: true },
                      instruction: { type: 'string', required: true },
                      tools: { type: 'array', required: true, items: { type: 'string' } },
                      gates: { type: 'array', required: true, items: { type: 'string' } },
                    },
                  },
                },
              },
            },
          },
          runtime: {
            type: 'object', required: true, additionalProperties: false,
            properties: {
              allowedPrograms: { type: 'array', required: true, items: { type: 'string' } },
            },
          },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `AppAgent adapter ${String(value.adapter)}: ${value.tracks.length} track(s) — `
          + value.tracks.map((track: { id: string; steps: unknown[] }) =>
            `${track.id} (${track.steps.length} steps)`).join(', '),
      }],
    },
    async execute() {
      const adapter = await adapterFor()
      const info = await ctx.aliothEnv.ready()
      const runtimeSource = await readFile(path.join(info.modelDir, 'skill-adapters', '_runtime.yaml'), 'utf8').catch(() => '')
      return {
        adapter: adapterName,
        tracks: adapter.tracks.map(track => ({
          id: track.name,
          name: track.name,
          steps: track.steps.map(step => ({
            id: step.id,
            instruction: step.instruction,
            tools: [...step.tools],
            gates: formatGates(step.gates),
          })),
        })),
        runtime: { allowedPrograms: parseRuntimeAllowedPrograms(runtimeSource) },
      }
    },
    presentCall: () => ({
      card: 'generic',
      title: 'AppAgent workflow info',
      kind: 'other',
      rawInput: {},
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_workflow_step',
    description:
      `Show the current AppAgent workflow step for an app (adapter ${adapterName}): the step's `
      + 'instruction, allowed tools, and gates. Call this at the start of each step and after '
      + 'alioth_workflow_complete advances. A finished run returns finished=true. '
      + 'PROGRAMMATIC-FIRST: the instruction is context — artifact content is generated by '
      + 'programmatic tools (alioth_app_write / alioth_app_configure / alioth_entity_write); '
      + 'write_file is NOT available for artifacts. Supply structured parameters, never raw JSON.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first.',
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
          referencePaths: { type: 'array', required: true, items: { type: 'string' } },
          inputs: {
            type: 'array', required: true,
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                path: { type: 'string', required: true },
                content: { type: 'string' },
              },
            },
          },
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
        return { finished: true, track: '', stepId: '', instruction: '', tools: [], gates: [], referencePaths: [], inputs: [] }
      }
      return {
        finished: false,
        track: current.track.name,
        stepId: current.step.id,
        instruction: current.step.instruction,
        tools: [...current.step.tools],
        gates: formatGates(current.step.gates),
        referencePaths: [...current.step.referencePaths],
        inputs: await readStepInputs(current.step, gateContext(args.namespace, args.app)),
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
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first.',
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
                status: { type: 'string', required: true },
                detail: { type: 'string', required: true },
                errorKind: { type: 'string' },
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
      const { contentRoot: gateCwd } = ensureContentRoot()
      const runner = createProgramRunner({
        cwd: gateCwd,
        env: {
          PROTOTYPE_TOOL_ROOT: gateCwd,
          // Service/DTO gates run against the namespace workspace
          // (Pre-Proc/{ns}/Cargo.toml) where the service crates are members.
          CARGO_WORKSPACE_DIR: path.join(preProcRoot, args.namespace),
        },
      })
      const results = await checkStepGates(current.step.gates, context, runner)
      const failed = results.filter(result => result.status === 'fail')
      if (failed.length > 0) {
        throw new Error(`alioth_workflow: gates failed for step ${current.step.id}:\n`
          + failed.map(result => `- [${result.errorKind ?? 'other'}/${isLlmFixable(result.errorKind ?? 'other') ? 'llm-fixable' : 'environment'}] ${result.detail}`).join('\n'))
      }
      const advanced = completeCurrentStep(state)
      const workflowRoot = config.workflowRoot ?? path.join(ctx.aliothEnv.dataRoot(), 'workflows')
      await saveRun(workflowRoot, { namespace: args.namespace, app: args.app }, advanced.state)
      const next = currentStep(advanced.state)
      return {
        finished: advanced.transition.finished,
        completedStep: current.step.id,
        gateResults: results.map(result => ({
          status: result.status,
          detail: result.detail,
          ...(result.errorKind === undefined ? {} : { errorKind: result.errorKind }),
        })),
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
