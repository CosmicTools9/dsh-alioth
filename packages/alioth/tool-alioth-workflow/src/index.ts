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
import { defineTool, type ObjectValueSchemaSpec } from '@deepseek-ai/dsh-tools'
import {
  ADAPTER_TOOL_TO_DSH,
  checkStepGates,
  completeCurrentStep,
  createProgramRunner,
  currentStep,
  formatRepairError,
  GATE_PROGRAM_WHITELIST,
  loadAdapter,
  loadRun,
  manualToolSurface,
  missingToolSurface,
  parseRuntimeAllowedPrograms,
  precheckStepInputs,
  repairContractFor,
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
  /**
   * Content root (upstream repo-root layout): the dir that contains Pre-Proc/
   * plus the provisioned `.agents/` references, `Framework/` utilities and
   * gate `scripts/`. Defaults to the parent of preProcRoot — set it explicitly
   * when preProcRoot lives under a shared temp root (tests).
   */
  readonly contentRoot?: string
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  adapter: z.string().default('alioth-app.yaml'),
  workflowRoot: z.string(),
  contentRoot: z.string(),
})

const NAMESPACE_PATTERN_RE = /^[A-Z][a-zA-Z0-9-]*$/
const APP_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/**
 * One gate as presented to the model (read-only projection of the adapter's
 * `StepGate`): both forms, program parameters, and the content predicate
 * (`requireJsonEquals` is the expected value as JSON text — the adapter value
 * is any JSON value). Nothing here weakens execution: the gates still run
 * through `checkStepGates`.
 */
export interface GateView {
  readonly kind: 'output-glob' | 'program'
  readonly outputGlob?: string
  readonly program?: string
  readonly args?: string[]
  readonly expectedExitCode?: number
  readonly timeoutSec?: number
  readonly requireJsonPointer?: string
  readonly requireJsonEquals?: string
}

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
  const contentRoot = path.resolve(config.contentRoot ?? path.dirname(preProcRoot))
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

  /**
   * Gate-program whitelist for the runner: the vendored `_runtime.yaml` mirror,
   * falling back to the code-truth floor when the mirror is absent or empty
   * (upstream RunCommandTool behavior — a missing mirror must never widen the
   * surface to "anything").
   */
  async function allowedGatePrograms(): Promise<readonly string[]> {
    const info = await ctx.aliothEnv.ready()
    const source = await readFile(path.join(info.modelDir, 'skill-adapters', '_runtime.yaml'), 'utf8').catch(() => '')
    const mirrored = parseRuntimeAllowedPrograms(source)
    return mirrored.length > 0 ? mirrored : GATE_PROGRAM_WHITELIST
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

  /**
   * Gate form as presented to the model — the **whole** adapter gate, read-only:
   * both forms keep their `output_glob`, program gates keep args/expected exit
   * code/timeout, and a content predicate keeps both halves
   * (`require_json_pointer` + the expected value as JSON text). Presentation
   * only: the executor still runs the gate through skill-alioth's
   * `checkStepGates` (no re-implementation, no weakening).
   */
  function gateView(gate: StepGate): GateView {
    const predicate = {
      ...(gate.requireJsonPointer === undefined ? {} : { requireJsonPointer: gate.requireJsonPointer }),
      ...(gate.requireJsonEquals === undefined ? {} : { requireJsonEquals: JSON.stringify(gate.requireJsonEquals) }),
    }
    if (gate.kind === 'output-glob') {
      return { kind: 'output-glob', outputGlob: gate.outputGlob, ...predicate }
    }
    return {
      kind: 'program',
      program: gate.program,
      args: [...gate.args],
      expectedExitCode: gate.expectedExitCode,
      timeoutSec: gate.timeoutSec,
      ...(gate.outputGlob === undefined ? {} : { outputGlob: gate.outputGlob }),
      ...predicate,
    }
  }

  /** Output-schema node for one {@link GateView} (presentation only). */
  const GATE_VIEW_SCHEMA = {
    type: 'object',
    additionalProperties: false,
    properties: {
      kind: { type: 'string', required: true },
      outputGlob: { type: 'string' },
      program: { type: 'string' },
      args: { type: 'array', items: { type: 'string' } },
      expectedExitCode: { type: 'number' },
      timeoutSec: { type: 'number' },
      requireJsonPointer: { type: 'string' },
      requireJsonEquals: { type: 'string' },
    },
  } as const satisfies ObjectValueSchemaSpec

  /** The step's resolved write surface: every `output_glob` the step gates on
   * (plan steps write only these; the reviewer sees the same set the guard
   * enforces). */
  function stepWriteGlobs(step: Step, context: GateContext): string[] {
    const resolved = step.gates.flatMap(gate =>
      gate.outputGlob === undefined ? [] : [gate.outputGlob.replace(/\{(\w+)\}/g, (match, key: string) => context.variables[key] ?? match)],
    )
    return [...new Set(resolved)]
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
                      phase: { type: 'string', required: true },
                      tools: { type: 'array', required: true, items: { type: 'string' } },
                      gates: { type: 'array', required: true, items: GATE_VIEW_SCHEMA },
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
      return {
        adapter: adapterName,
        tracks: adapter.tracks.map(track => ({
          id: track.name,
          name: track.name,
          steps: track.steps.map(step => ({
            id: step.id,
            instruction: step.instruction,
            phase: step.phase,
            tools: [...step.tools],
            gates: step.gates.map(gateView),
          })),
        })),
        runtime: { allowedPrograms: [...await allowedGatePrograms()] },
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
          phase: { type: 'string', required: true },
          planWriteGlobs: { type: 'array', required: true, items: { type: 'string' } },
          tools: { type: 'array', required: true, items: { type: 'string' } },
          harnessTools: { type: 'array', required: true, items: { type: 'string' } },
          manualTools: {
            type: 'array', required: true,
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                adapterTool: { type: 'string', required: true },
                reason: { type: 'string', required: true },
              },
            },
          },
          missingTools: { type: 'array', required: true, items: { type: 'string' } },
          gates: { type: 'array', required: true, items: GATE_VIEW_SCHEMA },
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
        return { finished: true, track: '', stepId: '', instruction: '', phase: 'apply', planWriteGlobs: [], tools: [], harnessTools: [], manualTools: [], missingTools: [], gates: [], referencePaths: [], inputs: [] }
      }
      // The step's declared tools are an execution contract, not a suggestion
      // (upstream rejects calls outside `default_tools ∪ step.tools`). The
      // harness executes tools itself, so the deployment cannot refuse a call —
      // instead the payload names the concrete harness tools that satisfy the
      // step, the ones only a human/skill path can satisfy, and the declared
      // ones nothing satisfies.
      const adapter = await adapterFor()
      const registered = new Set(ctx.tools.schemas().map(schema => schema.name))
      const declared = current.step.tools
      const manual = manualToolSurface(adapter).filter(entry => declared.includes(entry.adapterTool))
      const missing = missingToolSurface(adapter, registered).filter(entry => declared.includes(entry.adapterTool))
      const context = gateContext(args.namespace, args.app)
      // Upstream `precheck_step_inputs` (FAIL-FAST, run_skill.rs): a step whose declared upstream
      // inputs are absent MUST NOT start — handing it to the model burns a slow call for nothing
      // (upstream measured 10×; session 22). Nothing advances, and the error carries the repair
      // contract so the caller sees a rule id plus the missing paths.
      const precheck = precheckStepInputs({
        declaredInputs: current.step.inputs,
        ownOutputs: stepWriteGlobs(current.step, context),
        preProcRoot: context.preProcRoot,
        variables: context.variables,
      })
      if (precheck.missing.length > 0) {
        throw new Error(
          `alioth_workflow: step ${current.step.id} 声明输入缺失（${precheck.missing.length} 项，未启动、未推进）\n`
          + formatRepairError(repairContractFor('step-input-missing', current.step.id, precheck.missing.join(', '))),
        )
      }
      return {
        finished: false,
        track: current.track.name,
        stepId: current.step.id,
        instruction: current.step.instruction,
        phase: current.step.phase,
        planWriteGlobs: stepWriteGlobs(current.step, context),
        tools: [...declared],
        harnessTools: declared
          .flatMap(tool => ADAPTER_TOOL_TO_DSH[tool] ?? [])
          .filter(name => registered.has(name)),
        manualTools: manual.map(entry => ({ adapterTool: entry.adapterTool, reason: entry.reason })),
        missingTools: missing.map(entry => entry.adapterTool),
        gates: current.step.gates.map(gateView),
        referencePaths: [...current.step.referencePaths],
        inputs: await readStepInputs(current.step, context),
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
        allowedPrograms: await allowedGatePrograms(),
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
        // Structured repair contract per failed gate (rule id + class +
        // suggested action + the raw output as evidence) instead of a bare
        // terminal dump: the error text is the single carrier, and
        // `ruleIdFromError` is the only parser. Nothing advances.
        const lines = failed.map(result =>
          formatRepairError(result.repair ?? repairContractFor('unknown', current.step.id, result.detail)),
        )
        throw new Error(
          `alioth_workflow: gates failed for step ${current.step.id}`
          + `（${failed.length}/${results.length} 门禁未通过，未推进）\n${lines.join('\n')}`,
        )
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
