/**
 * Core-contract spec (contract §4/T2): 门内容谓词的 fail-closed 三态、未知 gate 键、
 * 步骤相位不变量、修复契约的规则码稳定性、重试预算三档动作与优先级、适配器工具映射完备性。
 */
import { describe, expect, it } from 'vitest'
import { mkdir, mkdtemp, rm, utimes, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { parseAdapterDocument, validateAdapterStructure, type Adapter, type StepGate } from '../src/adapter.ts'
import { checkGate, type GateContext } from '../src/gates.ts'
import {
  EVIDENCE_HEAD_LIMIT,
  REGISTERED_RULE_IDS,
  formatRepairError,
  repairContractFor,
  ruleIdFromError,
  type FailureKind,
} from '../src/repair.ts'
import { RetryBudget, callSignature, errorSignature } from '../src/retry-budget.ts'
import { ADAPTER_TOOL_TO_DSH, ADAPTER_TOOL_VOCABULARY, MANUAL_ADAPTER_TOOLS } from '../src/mapping.ts'

/** Run a body against a throwaway Pre-Proc root. */
async function withRoot(fn: (root: string) => Promise<void>): Promise<void> {
  const root = await mkdtemp(path.join(tmpdir(), 'skill-contracts-'))
  try {
    await fn(root)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

function gateContext(root: string): GateContext {
  return { preProcRoot: root, variables: { ns: 'Alioth', module: 'demo' } }
}

const REPORT_GLOB = 'Pre-Proc/{ns}/Prototypes/Modules/{module}/visual-verify/*.json'

function visualGate(): StepGate {
  return { kind: 'output-glob', outputGlob: REPORT_GLOB, requireJsonPointer: '/verdict', requireJsonEquals: 'PASS' }
}

async function writeReport(root: string, name: string, body: string, mtimeSec: number): Promise<string> {
  const dir = path.join(root, 'Alioth', 'Prototypes', 'Modules', 'demo', 'visual-verify')
  await mkdir(dir, { recursive: true })
  const file = path.join(dir, name)
  await writeFile(file, body)
  await utimes(file, mtimeSec, mtimeSec)
  return file
}

/** One-track adapter document; `steps` is the raw YAML step list. */
function adapterSource(steps: string): string {
  return `name: probe\nversion: "1.0"\ntracks:\n  - name: probe-track\n    steps:\n${steps}`
}

describe('gate content predicate (fail-closed)', () => {
  it('passes only when the newest-mtime hit carries the expected pointer value', async () => {
    await withRoot(async root => {
      await writeReport(root, 'report-old.json', '{"verdict":"PASS"}', 1000)
      await writeReport(root, 'report-new.json', '{"verdict":"REVISE"}', 2000)
      const newerWins = await checkGate(visualGate(), gateContext(root))
      expect(newerWins.status).toBe('fail')
      expect(newerWins.detail).toContain("pointer '/verdict'")

      // Same tree, other order: the PASS report becomes the newest → gate passes.
      await utimes(
        path.join(root, 'Alioth', 'Prototypes', 'Modules', 'demo', 'visual-verify', 'report-old.json'),
        3000,
        3000,
      )
      const olderWins = await checkGate(visualGate(), gateContext(root))
      expect(olderWins.status).toBe('pass')
    })
  })

  it('fails on value mismatch, absent pointer, unparsable JSON, and missing file', async () => {
    await withRoot(async root => {
      await writeReport(root, 'report.json', '{"verdict":"REVISE"}', 1000)
      const mismatch = await checkGate(visualGate(), gateContext(root))
      expect(mismatch.status).toBe('fail')
      expect(mismatch.detail).toContain('不满足')
      expect(mismatch.repair?.ruleId).toBe('gate-json-predicate-fail')

      await writeReport(root, 'report.json', '{"score":1}', 2000)
      const absent = await checkGate(visualGate(), gateContext(root))
      expect(absent.status).toBe('fail')
      expect(absent.detail).toContain("pointer '/verdict' 缺失")

      await writeReport(root, 'report.json', '{"verdict": "PASS"', 3000)
      const unparsable = await checkGate(visualGate(), gateContext(root))
      expect(unparsable.status).toBe('fail')
      expect(unparsable.detail).toContain('解析失败')

      await rm(path.join(root, 'Alioth', 'Prototypes', 'Modules', 'demo', 'visual-verify', 'report.json'))
      const missing = await checkGate(visualGate(), gateContext(root))
      expect(missing.status).toBe('fail')
      expect(missing.detail).toContain('no match for glob')
      expect(missing.repair?.ruleId).toBe('gate-output-glob-miss')
    })
  })

  it('compares nested values deeply (RFC 6901 pointers, ~0/~1 escaping)', async () => {
    await withRoot(async root => {
      const deep: StepGate = {
        kind: 'output-glob',
        outputGlob: REPORT_GLOB,
        requireJsonPointer: '/checks/0/a~1b',
        requireJsonEquals: 3,
      }
      await writeReport(root, 'report.json', '{"checks":[{"a/b":3}]}', 1000)
      expect((await checkGate(deep, gateContext(root))).status).toBe('pass')
      await writeReport(root, 'report.json', '{"checks":[{"a/b":4}]}', 2000)
      expect((await checkGate(deep, gateContext(root))).status).toBe('fail')
    })
  })

  it('leaves predicate-free gates on the existence semantics', async () => {
    await withRoot(async root => {
      await writeReport(root, 'report.json', 'not json at all', 1000)
      const plain: StepGate = { kind: 'output-glob', outputGlob: REPORT_GLOB }
      expect((await checkGate(plain, gateContext(root))).status).toBe('pass')
    })
  })
})

describe('adapter load-time invariants', () => {
  it('throws on an unknown gate key instead of degrading to an existence check', () => {
    const source = adapterSource(
      '      - id: "1"\n        instruction: "x"\n        gates:\n'
      + '          - output_glob: "Pre-Proc/{ns}/a.json"\n            require_json_regex: "^PASS$"\n',
    )
    expect(() => parseAdapterDocument(source, 'probe.yaml')).toThrow(/unknown gate key 'require_json_regex'/)
  })

  it('throws when the predicate pair is set on one side only', () => {
    const source = adapterSource(
      '      - id: "1"\n        instruction: "x"\n        gates:\n'
      + '          - output_glob: "Pre-Proc/{ns}/a.json"\n            require_json_pointer: "/verdict"\n',
    )
    expect(() => parseAdapterDocument(source, 'probe.yaml')).toThrow(/须同时设置/)
  })

  it('throws when a predicate has no output_glob to evaluate', () => {
    const source = adapterSource(
      '      - id: "1"\n        instruction: "x"\n        gates:\n'
      + '          - program: "bun"\n            args: ["check"]\n'
      + '            require_json_pointer: "/verdict"\n            require_json_equals: "PASS"\n',
    )
    expect(() => parseAdapterDocument(source, 'probe.yaml')).toThrow(/谓词须依附 output_glob/)
  })

  it('throws when a plan step declares no output_glob', () => {
    const source = adapterSource(
      '      - id: "1"\n        instruction: "plan"\n        phase: plan\n        gates: []\n'
      + '      - id: "2"\n        instruction: "apply"\n        gates: []\n',
    )
    expect(() => parseAdapterDocument(source, 'probe.yaml')).toThrow(/plan 步但未声明任何 gate\.output_glob/)
  })

  it('throws when a plan step has no later apply step in the same track', () => {
    const source = adapterSource(
      '      - id: "1"\n        instruction: "apply"\n        gates: []\n'
      + '      - id: "2"\n        instruction: "plan"\n        phase: plan\n'
      + '        gates:\n          - output_glob: "Pre-Proc/{ns}/plan.json"\n',
    )
    expect(() => parseAdapterDocument(source, 'probe.yaml')).toThrow(/其后同 Track 内无 apply 步/)
  })

  it('accepts a plan/apply pair and defaults an undeclared phase to apply', () => {
    const adapter = parseAdapterDocument(
      adapterSource(
        '      - id: "1"\n        instruction: "plan"\n        phase: plan\n'
        + '        gates:\n          - output_glob: "Pre-Proc/{ns}/plan.json"\n'
        + '      - id: "2"\n        instruction: "land"\n        gates: []\n'
        + '      - id: "3"\n        instruction: "later"\n        gates: []\n',
      ),
      'probe.yaml',
    )
    expect(adapter.tracks[0]?.steps.map(step => step.phase)).toEqual(['plan', 'apply', 'apply'])
    expect(validateAdapterStructure(adapter)).toEqual([])
  })

  it('reports every structural problem for a hand-built adapter without throwing', () => {
    const bad: Adapter = {
      name: 'bad',
      description: '',
      version: '',
      defaultTools: [],
      referencePaths: [],
      tracks: [{
        name: 't',
        steps: [
          {
            id: '1',
            instruction: '',
            tools: [],
            schema: undefined,
            referencePaths: [],
            inputs: [],
            phase: 'plan',
            gates: [{ kind: 'output-glob', outputGlob: 'x', requireJsonPointer: '/a' }],
          },
        ],
      }],
    }
    const problems = validateAdapterStructure(bad)
    expect(problems.some(problem => problem.includes('须同时设置'))).toBe(true)
    expect(problems.some(problem => problem.includes('无 apply 步'))).toBe(true)
  })

  it('parses the predicate pair into the typed gate', () => {
    const adapter = parseAdapterDocument(
      adapterSource(
        '      - id: "1"\n        instruction: "x"\n        gates:\n'
        + '          - output_glob: "Pre-Proc/{ns}/visual-verify/*.json"\n'
        + '            require_json_pointer: "/verdict"\n            require_json_equals: "PASS"\n',
      ),
      'probe.yaml',
    )
    expect(adapter.tracks[0]?.steps[0]?.gates[0]).toEqual({
      kind: 'output-glob',
      outputGlob: 'Pre-Proc/{ns}/visual-verify/*.json',
      requireJsonPointer: '/verdict',
      requireJsonEquals: 'PASS',
    })
  })
})

describe('repair contract', () => {
  const OUTPUT_A = 'gate output: line 3 unexpected token REVISE'
  const OUTPUT_B = 'gate output: line 91 failed at /verdict (drifted text)'

  it('keeps the rule id stable across drifting output text', () => {
    const first = repairContractFor('gate-json-predicate', '', OUTPUT_A)
    const second = repairContractFor('gate-json-predicate', '', OUTPUT_B)
    expect(first.ruleId).toBe(second.ruleId)
    expect(ruleIdFromError(formatRepairError(first))).toBe(first.ruleId)
    expect(ruleIdFromError(formatRepairError(second))).toBe(second.ruleId)
    expect(errorSignature('bash', formatRepairError(first))).toBe(errorSignature('bash', formatRepairError(second)))
  })

  it('refines exit-code rules by program and never yields an empty action', () => {
    expect(repairContractFor('gate-exit', 'cargo', 'e').ruleId).toBe('gate-cargo-nonzero')
    expect(repairContractFor('gate-exit', 'bun', 'e').ruleId).toBe('gate-check-script-nonzero')
    expect(repairContractFor('gate-exit', 'npx', 'e').ruleId).toBe('gate-frontend-nonzero')
    expect(repairContractFor('gate-exit', 'whatever', 'e').ruleId).toBe('gate-exit-nonzero')
    const kinds: readonly FailureKind[] = [
      'gate-exit', 'gate-timeout', 'gate-missing-program', 'gate-output-missing',
      'gate-json-predicate', 'tool-denied', 'write-outside-sandbox', 'plan-write-outside-scope', 'unknown',
    ]
    for (const kind of kinds) {
      const contract = repairContractFor(kind, 'bun', 'evidence')
      expect(REGISTERED_RULE_IDS).toContain(contract.ruleId)
      expect(contract.suggestedAction.trim()).not.toBe('')
      expect(contract.message.trim()).not.toBe('')
    }
    expect(repairContractFor('write-outside-sandbox', 'write_file', 'e').class).toBe('not-fixable')
    expect(repairContractFor('gate-timeout', 'bun', 'e').class).toBe('retryable')
  })

  it('renders a single line with a truncated evidence head', () => {
    const contract = repairContractFor('gate-exit', 'cargo', `first\n${'e'.repeat(EVIDENCE_HEAD_LIMIT * 2)}`)
    const rendered = formatRepairError(contract)
    expect(rendered.startsWith('[rule:gate-cargo-nonzero]')).toBe(true)
    expect(rendered).not.toContain('\n')
    expect(rendered).toContain('截断')
    expect(rendered).toContain('下一步：')
  })

  it('returns null for text without a closed, non-empty envelope', () => {
    expect(ruleIdFromError('gate io error: boom')).toBeNull()
    expect(ruleIdFromError('[rule:] class=x')).toBeNull()
    expect(ruleIdFromError('[rule:no-close')).toBeNull()
  })

  it('falls back to the tool+text hash signature when no envelope is present', () => {
    const plain = errorSignature('bash', 'boom')
    expect(plain.startsWith('bash:')).toBe(true)
    expect(plain).not.toBe(errorSignature('bash', 'different boom'))
    expect(errorSignature('bash', 'boom')).not.toBe(errorSignature('read', 'boom'))
  })
})

describe('retry budget', () => {
  const RULE_ERROR = formatRepairError(repairContractFor('gate-timeout', 'bun', 'timeout after 120s'))

  it('escalates error signatures: allow → trim-retry (2nd) → escalate (3rd)', () => {
    const budget = new RetryBudget()
    expect(budget.recordError('bash', RULE_ERROR)).toBe(1)
    expect(budget.decide()).toBe('allow')
    expect(budget.recordError('bash', RULE_ERROR)).toBe(2)
    expect(budget.decide()).toBe('trim-retry')
    expect(budget.recordError('bash', RULE_ERROR)).toBe(3)
    expect(budget.decide()).toBe('escalate')
  })

  it('terminates on consecutive identical calls and resets on progress', () => {
    const budget = new RetryBudget()
    budget.recordCall('bash', { cmd: 'ls' })
    budget.recordCall('bash', { cmd: 'ls' })
    expect(budget.decide()).toBe('allow')
    budget.recordCall('bash', { cmd: 'ls' })
    expect(budget.decide()).toBe('terminate')
    budget.recordCall('bash', { cmd: 'ls -la' })
    expect(budget.decide()).toBe('allow')
  })

  it('treats key order as the same call signature', () => {
    expect(callSignature('write_file', { path: 'a', content: 'x' }))
      .toBe(callSignature('write_file', { content: 'x', path: 'a' }))
  })

  it('applies precedence terminate > escalate > trim-retry > allow', () => {
    const budget = new RetryBudget()
    budget.recordError('bash', RULE_ERROR)
    budget.recordError('bash', RULE_ERROR)
    expect(budget.decide()).toBe('trim-retry')
    budget.recordCall('bash', { cmd: 'x' })
    budget.recordCall('bash', { cmd: 'x' })
    budget.recordCall('bash', { cmd: 'x' })
    expect(budget.decide()).toBe('terminate')

    // Without the ping-pong, a wall-count error outranks a trim-retry one.
    const second = new RetryBudget({ maxRepeatCall: 9 })
    const other = formatRepairError(repairContractFor('gate-exit', 'bun', 'exit 1'))
    second.recordError('bash', RULE_ERROR)
    second.recordError('bash', RULE_ERROR)
    second.recordError('bash', other)
    second.recordError('bash', other)
    second.recordError('bash', other)
    expect(second.decide()).toBe('escalate')
  })

  it('honours custom budgets and rejects non-positive limits', () => {
    const budget = new RetryBudget({ maxRepeatCall: 2, maxRepeatError: 2 })
    budget.recordCall('read', { p: 1 })
    budget.recordCall('read', { p: 1 })
    expect(budget.decide()).toBe('terminate')
    const errors = new RetryBudget({ maxRepeatCall: 9, maxRepeatError: 2 })
    errors.recordError('bash', RULE_ERROR)
    errors.recordError('bash', RULE_ERROR)
    expect(errors.decide()).toBe('escalate')
    expect(() => new RetryBudget({ maxRepeatCall: 0 })).toThrow(/integer >= 1/)
  })
})

describe('adapter tool mapping completeness', () => {
  /** Harness tools verified against `defineTool({ name })` in the deepseek-harness checkout. */
  const HARNESS_TOOLS: readonly string[] = [
    'read', 'write', 'edit', 'glob', 'grep', 'bash', 'lsp', 'todo_write', 'web_search',
    'web_fetch', 'read_image', 'terminal_open', 'terminal_read', 'terminal_close',
    // 本仓 Wave 2 提供的 deferred 登记工具（映射先行，实现随后）。
    'alioth_deferred',
  ]

  it('covers every vocabulary tool with a mapping or an explicit manual entry', () => {
    const uncovered = ADAPTER_TOOL_VOCABULARY.filter(
      tool => ADAPTER_TOOL_TO_DSH[tool] === undefined && MANUAL_ADAPTER_TOOLS[tool] === undefined,
    )
    expect(uncovered).toEqual([])
    // No tool may be both mapped and manual (ambiguous routing).
    const both = ADAPTER_TOOL_VOCABULARY.filter(
      tool => ADAPTER_TOOL_TO_DSH[tool] !== undefined && MANUAL_ADAPTER_TOOLS[tool] !== undefined,
    )
    expect(both).toEqual([])
  })

  it('maps only onto real harness tool names', () => {
    for (const [tool, names] of Object.entries(ADAPTER_TOOL_TO_DSH)) {
      expect(names.length).toBeGreaterThan(0)
      for (const name of names) {
        expect(HARNESS_TOOLS, `${tool} → ${name}`).toContain(name)
      }
    }
  })

  it('gives every manual entry a reason', () => {
    for (const [tool, reason] of Object.entries(MANUAL_ADAPTER_TOOLS)) {
      expect(reason.length, tool).toBeGreaterThan(8)
    }
  })
})
