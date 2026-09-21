import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { parseAdapterDocument, unreachableGatePrograms, type Adapter } from '@dsh-alioth/skill-alioth'
import { unreachableAdapterReferences } from '../scripts/lib/adapter-references.ts'

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const VENDOR = path.join(REPO_ROOT, 'packages', 'alioth', 'env-alioth', 'vendor')
const ADAPTERS = path.join(VENDOR, 'skill-adapters')

/**
 * Vendored adapters parsed into the model; underscore-prefixed files are
 * runtime config (`_runtime.yaml`), not adapters.
 */
function loadVendoredAdapters(): Map<string, Adapter> {
  const loaded = new Map<string, Adapter>()
  for (const name of readdirSync(ADAPTERS).sort()) {
    if (!name.endsWith('.yaml') || name.startsWith('_')) continue
    loaded.set(name, parseAdapterDocument(readFileSync(path.join(ADAPTERS, name), 'utf8'), name))
  }
  return loaded
}

const VENDORED_ADAPTERS = loadVendoredAdapters()

describe('vendored adapter assets', () => {
  it('resolves every gate script and every declared reference the vendored adapters carry', () => {
    // Two halves of one contract: a gate script the tree cannot spawn is an
    // ENOENT stall (path-missing, not LLM-fixable), and a declared
    // reference_paths/inputs asset the tree cannot resolve is the same defect
    // one layer out — the step's context is silently incomplete. Both are
    // properties of the SYNC_SET, so an empty result here is what the sync
    // asserts on every run. Regressions this guards: check-prototype-types.ts /
    // check-prototype-render.ts / capability-catalog.ts absent while adapters
    // spawn them; docs/specs/*.md and the alioth-service reference set absent
    // while adapters declare them.
    expect(unreachableGatePrograms(ADAPTERS, VENDOR)).toEqual([])
    expect(unreachableAdapterReferences(ADAPTERS, VENDOR)).toEqual([])
  })

  it('parses every vendored adapter and keeps the v2.2 keys', () => {
    // The relocated adapter set is the model surface itself: names must not
    // silently shrink with the directory sync, and every `alioth-*` adapter
    // carries `default_tools` (v2.2 — the tool floor the adapter grants).
    expect([...VENDORED_ADAPTERS.keys()]).toEqual([
      'alioth-app.yaml',
      'alioth-block.yaml',
      'alioth-compose.yaml',
      'alioth-gui.yaml',
      'alioth-module.yaml',
      'alioth-service.yaml',
      'mini-write.yaml',
    ])
    for (const [name, adapter] of VENDORED_ADAPTERS) {
      if (!name.startsWith('alioth-')) continue
      expect(adapter.defaultTools.length, `${name} default_tools`).toBeGreaterThan(0)
    }
  })

  it('keeps the phase surface: alioth-module plans before it applies', () => {
    // v2.2 `phase`: a plan step narrows its write surface to its own
    // output_glob, and the artifacts it proposes are landed by a later apply
    // step of the same track. A vendored adapter that lost the key would run
    // the plan step as an ordinary write step.
    const moduleAdapter = VENDORED_ADAPTERS.get('alioth-module.yaml')
    expect(moduleAdapter).toBeDefined()
    const planned = (moduleAdapter?.tracks ?? [])
      .map(track => ({ track, plan: track.steps.findIndex(step => step.phase === 'plan') }))
      .filter(entry => entry.plan >= 0)
    expect(planned.length).toBeGreaterThan(0)
    for (const { track, plan } of planned) {
      const planStep = track.steps[plan]
      expect(planStep?.gates.some(gate => gate.kind === 'output-glob')).toBe(true)
      const applied = track.steps
        .slice(plan + 1)
        .some(step => step.phase === 'apply')
      expect(applied, `${track.name} applies after ${planStep?.id}`).toBe(true)
    }
  })

  it('keeps the content predicate: alioth-gui grades the verdict artifact', () => {
    // v2.2 `require_json_pointer`/`require_json_equals`: artifact existence is
    // not enough — REVISE/FAIL reports exist too, so the gate must read the
    // verdict out of the mtime-newest output_glob hit.
    const guiAdapter = VENDORED_ADAPTERS.get('alioth-gui.yaml')
    expect(guiAdapter).toBeDefined()
    const predicates = (guiAdapter?.tracks ?? [])
      .flatMap(track => track.steps)
      .flatMap(step => step.gates)
      .filter(gate => gate.requireJsonPointer !== undefined)
    expect(predicates.length).toBeGreaterThan(0)
    for (const gate of predicates) {
      expect(gate.requireJsonPointer).toBe('/verdict')
      expect(gate.requireJsonEquals).toBe('PASS')
      expect(gate.outputGlob).toContain('report.json')
    }
  })
})

describe('unreachableAdapterReferences', () => {
  /** Minimal vendor-shaped tree plus one adapter declaring both reference kinds. */
  function probeTree(): { root: string; adapters: string } {
    const root = mkdtempSync(path.join(tmpdir(), 'adapter-refs-'))
    const adapters = path.join(root, 'skill-adapters')
    mkdirSync(path.join(root, 'docs', 'specs'), { recursive: true })
    mkdirSync(path.join(root, 'Pre-Proc'), { recursive: true })
    mkdirSync(adapters, { recursive: true })
    writeFileSync(path.join(root, 'docs', 'specs', 'PRESENT.md'), '# present\n')
    writeFileSync(path.join(adapters, 'probe.yaml'), [
      'name: probe',
      'version: "2.2"',
      'reference_paths:',
      '  - "docs/specs/PRESENT.md"',
      '  - "docs/specs/MISSING.md"',
      'tracks:',
      '  - name: 轨道',
      '    steps:',
      '      - id: "1.1"',
      '        instruction: "probe"',
      '        inputs:',
      '          - "Pre-Proc/{ns}/Sources/Apps/Modules/{module}/module.json"',
      '        gates:',
      '          - output_glob: "Pre-Proc/{ns}/Out.txt"',
      '      - id: "1.2"',
      '        instruction: "probe"',
      '        inputs:',
      '          - "Pre-Proc/Gone/seed.json"',
      '        gates:',
      '          - output_glob: "Pre-Proc/{ns}/Out.txt"',
      '',
    ].join('\n'))
    return { root, adapters }
  }

  it('flags a fixed reference the tree does not carry, per adapter and step', () => {
    const { root, adapters } = probeTree()
    try {
      expect(unreachableAdapterReferences(adapters, root)).toEqual([
        { adapter: 'probe.yaml', step: 'adapter', reference: 'docs/specs/MISSING.md' },
        { adapter: 'probe.yaml', step: '轨道.1.2', reference: 'Pre-Proc/Gone/seed.json' },
      ])
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  })

  it('accepts a runtime coordinate whose static prefix exists', () => {
    // `{ns}`/`{module}` name artifacts the pipeline produces in the content
    // root at run time (scaffolding, an earlier step, a plan-step artifact) —
    // the tree cannot be expected to hold them, so the static prefix is the
    // strongest assertion the vendor tree can make.
    const { root, adapters } = probeTree()
    try {
      const reported = unreachableAdapterReferences(adapters, root)
        .map(item => item.reference)
      expect(reported).not.toContain('Pre-Proc/{ns}/Sources/Apps/Modules/{module}/module.json')
      expect(reported).not.toContain('docs/specs/PRESENT.md')
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  })
})
