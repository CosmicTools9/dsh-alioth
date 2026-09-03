import { describe, expect, it, beforeAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { parseAdapterDocument, loadAdapter, parseRuntimeAllowedPrograms, type Adapter } from '../src/adapter.ts'
import { completeCurrentStep, currentStep, initialRunState, type RunState } from '../src/state.ts'
import { checkStepGates } from '../src/gates.ts'
import { loadRun, saveRun } from '../src/workspace.ts'
import { ADAPTER_TOOL_TO_DSH, missingToolSurface } from '../src/mapping.ts'
import { bunAvailable, createProgramRunner } from '../src/bun.ts'

/** The real `alioth-app.yaml` shape: tracks with steps carrying tools/schema/gates. */
const APP_ADAPTER = `
name: alioth-app
description: "App 级原型集成"
version: "2.0"
tracks:
  - name: App 原型集成
    steps:
      - id: "1.1"
        instruction: "preflight — 确认 App 上下文"
        tools: [read_file, search_files]
        schema: {type: object, required: [ns, app]}
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/"
      - id: "1.2"
        instruction: "AppLayout 设计"
        tools: [read_file, write_file]
        schema: {type: object, required: [app_tsx_path]}
        gates:
          - program: "bun"
            args: ["scripts/prototype-tool.js", "build", "Pre-Proc/{ns}/Apps/{app}/llm-tsx/app.tsx"]
            output_glob: "Pre-Proc/{ns}/Prototypes/Apps/{app}/a-v*.html"
  - name: 第二 track
    steps:
      - id: "2.1"
        instruction: "done"
        gates: []
`

let adapter: Adapter

beforeAll(() => {
  adapter = parseAdapterDocument(APP_ADAPTER, 'alioth-app.yaml')
})

describe('skill-alioth adapter parsing', () => {
  it('parses name/version/tracks with typed steps', () => {
    expect(adapter.name).toBe('alioth-app')
    expect(adapter.version).toBe('2.0')
    expect(adapter.tracks).toHaveLength(2)
    const first = adapter.tracks[0]!.steps[0]!
    expect(first).toMatchObject({
      id: '1.1',
      tools: ['read_file', 'search_files'],
      schema: { type: 'object', required: ['ns', 'app'] },
    })
  })

  it('parses both gate kinds', () => {
    const gates = adapter.tracks[0]!.steps[1]!.gates
    expect(gates).toHaveLength(1)
    expect(gates[0]).toEqual({
      kind: 'program',
      program: 'bun',
      args: ['scripts/prototype-tool.js', 'build', 'Pre-Proc/{ns}/Apps/{app}/llm-tsx/app.tsx'],
      expectedExitCode: 0,
      timeoutSec: 120,
      outputGlob: 'Pre-Proc/{ns}/Prototypes/Apps/{app}/a-v*.html',
    })
    expect(adapter.tracks[0]!.steps[0]!.gates[0]).toEqual({ kind: 'output-glob', outputGlob: 'Pre-Proc/{ns}/Apps/{app}/' })
  })

  it('rejects malformed documents loudly', () => {
    expect(() => parseAdapterDocument('name: x\n', 'broken.yaml')).toThrow('tracks must be an array')
    expect(() => parseAdapterDocument('{bad yaml', 'broken.yaml')).toThrow('invalid YAML')
    expect(() => parseAdapterDocument('name: x\ntracks:\n  - name: t\n    steps:\n      - id: "1"\n        instruction: "do"\n        gates:\n          - {}\n', 'bad.yaml')).toThrow('must declare output_glob or program')
  })

  it('loads an adapter file from a snapshot layout', async () => {
    const dir = await mkdtemp(path.join(tmpdir(), 'skill-alioth-'))
    try {
      await mkdir(path.join(dir, 'skill-adapters'), { recursive: true })
      await writeFile(path.join(dir, 'skill-adapters', 'alioth-app.yaml'), APP_ADAPTER)
      const loaded = await loadAdapter(dir, 'alioth-app.yaml')
      expect(loaded.name).toBe('alioth-app')
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })

  it('parses the runtime program allowlist, tolerating missing/broken files', () => {
    expect(parseRuntimeAllowedPrograms('allowed_programs:\n  - bun\n  - target/debug/ontology-mapping\n'))
      .toEqual(['bun', 'target/debug/ontology-mapping'])
    expect(parseRuntimeAllowedPrograms('allowed_programs: []')).toEqual([])
    expect(parseRuntimeAllowedPrograms('name: x\n')).toEqual([])
    expect(parseRuntimeAllowedPrograms('{bad yaml')).toEqual([])
  })
})

describe('skill-alioth state machine', () => {
  it('walks steps in order and finishes after the last track', () => {
    let state = initialRunState(adapter)
    expect(currentStep(state)?.step.id).toBe('1.1')
    expect(currentStep(state)?.track.name).toBe('App 原型集成')

    const ids: string[] = []
    for (let guard = 0; guard < 10; guard += 1) {
      const step = currentStep(state)
      if (step === undefined) break
      ids.push(step.step.id)
      const advanced = completeCurrentStep(state)
      state = advanced.state
      if (advanced.transition.finished) break
    }
    expect(ids).toEqual(['1.1', '1.2', '2.1'])
    expect(currentStep(state)).toBeUndefined()
  })

  it('is idempotent against double completion of the same step', () => {
    // Position still on step 1.1 while its id is already recorded: a re-run
    // after a crash must not double-advance.
    const state: RunState = {
      adapter,
      position: { trackIndex: 0, stepIndex: 0 },
      completed: ['1.1'],
    }
    const advanced = completeCurrentStep(state)
    expect(advanced.transition).toEqual({
      previous: { trackIndex: 0, stepIndex: 0 },
      next: { trackIndex: 0, stepIndex: 0 },
      finished: false,
    })
    expect(advanced.state.completed).toEqual(['1.1'])
  })
})

describe('skill-alioth gates', () => {
  it('checks output-glob existence with placeholder resolution', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'skill-gates-'))
    try {
      const appDir = path.join(root, 'Alioth', 'Apps', 'demo')
      await mkdir(appDir, { recursive: true })
      await writeFile(path.join(appDir, 'app.json'), '{}')
      const context = { preProcRoot: root, variables: { ns: 'Alioth', app: 'demo' } }
      const results = await checkStepGates(
        [{ kind: 'output-glob', outputGlob: 'Pre-Proc/{ns}/Apps/{app}/app.json' }],
        context,
      )
      expect(results).toHaveLength(1)
      expect(results[0]?.status).toBe('pass')
      const missing = await checkStepGates(
        [{ kind: 'output-glob', outputGlob: 'Pre-Proc/{ns}/Apps/{app}/nope.json' }],
        context,
      )
      expect(missing[0]?.status).toBe('fail')
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })

  it('runs program gates through the hook and declares them otherwise', async () => {
    const context = { preProcRoot: '/tmp/x', variables: { ns: 'Alioth', app: 'demo' } }
    const gate = { kind: 'program', program: 'bun', args: ['build'], expectedExitCode: 0, timeoutSec: 120 } as const
    const declared = await checkStepGates([gate], context)
    expect(declared[0]?.status).toBe('not-attempted')
    expect(declared[0]?.detail).toContain('not executed')
    const run = await checkStepGates([gate], context, async () => ({ ok: true, exitCode: 0, detail: 'built' }))
    expect(run[0]?.status).toBe('pass')
    expect(run[0]?.detail).toBe('built')
  })

  it('rejects globs escaping preProcRoot', async () => {
    const context = { preProcRoot: '/tmp/x', variables: {} }
    const results = await checkStepGates(
      [{ kind: 'output-glob', outputGlob: '../../../etc/passwd' }],
      context,
    )
    expect(results[0]?.status).toBe('fail')
    expect(results[0]?.detail).toContain('escapes')
  })
})

describe('skill-alioth run persistence', () => {
  it('starts fresh, persists, and resumes at the saved position', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'skill-runs-'))
    try {
      const meta = { namespace: 'Alioth', app: 'demo' }
      const fresh = await loadRun(root, meta, adapter)
      expect(currentStep(fresh)?.step.id).toBe('1.1')

      const advanced = completeCurrentStep(fresh)
      await saveRun(root, meta, advanced.state)

      const resumed = await loadRun(root, meta, adapter)
      expect(currentStep(resumed)?.step.id).toBe('1.2')
      expect(resumed.completed).toEqual(['1.1'])
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })

  it('rejects corrupt run state', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'skill-runs-bad-'))
    try {
      const meta = { namespace: 'Alioth', app: 'bad' }
      await mkdir(path.join(root, 'Alioth', 'bad'), { recursive: true })
      await writeFile(
        path.join(root, 'Alioth', 'bad', 'run-state.json'),
        '{not json',
      )
      await expect(loadRun(root, meta, adapter)).rejects.toThrow('corrupt run state')
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })
})

describe('skill-alioth tool-surface mapping', () => {
  it('reports missing harness tools per adapter reference', () => {
    const missing = missingToolSurface(adapter, new Set(['read', 'write']))
    // write_file now maps to the harness write surface (gated code authoring);
    // search_files remains unmapped without glob/grep registered.
    expect(missing).toHaveLength(1)
    expect(missing[0]).toMatchObject({ adapterTool: 'search_files', usedBy: ['1.1'] })
  })

  it('maps write_file to the harness write surface (code files in gated steps)', () => {
    const registered = new Set(['read', 'tool:read', 'write', 'tool:write', 'glob', 'tool:glob', 'grep', 'tool:grep'])
    const missing = missingToolSurface(adapter, registered)
    expect(missing.find(item => item.adapterTool === 'write_file')).toBeUndefined()
    expect(ADAPTER_TOOL_TO_DSH.write_file).toEqual(['write', 'tool:write'])
  })
})

describe('skill-alioth program runner', () => {
  const run = createProgramRunner({ timeoutMs: 15_000 })

  it('resolves ok on exit 0 with stdout evidence', async () => {
    const result = await run('node', ['-e', 'console.log("runner-ok")'], {
      kind: 'program', program: 'node', args: [], expectedExitCode: 0, timeoutSec: 15,
    })
    expect(result.ok).toBe(true)
    expect(result.exitCode).toBe(0)
    expect(result.detail).toContain('runner-ok')
  })

  it('reports non-zero exits with stderr evidence', async () => {
    const result = await run('node', ['-e', 'console.error("boom"); process.exit(3)'], {
      kind: 'program', program: 'node', args: [], expectedExitCode: 0, timeoutSec: 15,
    })
    expect(result.ok).toBe(false)
    expect(result.exitCode).toBe(3)
    expect(result.detail).toContain('exited 3')
    expect(result.detail).toContain('boom')
  })

  it('reports missing binaries as spawn failures', async () => {
    const result = await run('definitely-not-a-real-binary-xyz', [], {
      kind: 'program', program: 'x', args: [], expectedExitCode: 0, timeoutSec: 15,
    })
    expect(result.ok).toBe(false)
    expect(result.exitCode).toBe(null)
    expect(result.detail).toContain('spawn')
  })

  it('probes bun availability (the distribution gate toolchain)', async () => {
    const available = await bunAvailable()
    // The model distribution executes prototype gates with bun; a dev machine
    // for Alioth work has it. This asserts the probe agrees with reality.
    expect(available).toBe(true)
  })
})
