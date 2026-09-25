import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { precheckStepInputs, resolveInputTemplate, stepInputExists } from '../src/step-inputs.ts'

/**
 * Mirrors the upstream startup precheck (`run_skill.rs:precheck_step_inputs`,
 * `FailureKind::StepInputMissing`): a step whose declared upstream inputs are absent must not
 * start. Own products and unmatched globs are the two cases that must NOT be reported.
 */
let root: string

beforeAll(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'step-inputs-'))
  await mkdir(path.join(root, 'U-x', 'Apps', 'a'), { recursive: true })
  await writeFile(path.join(root, 'U-x', 'Apps', 'a', 'app.json'), '{}\n')
  await mkdir(path.join(root, 'U-x', 'Prototypes', 'Blocks', 'b', 'llm-tsx'), { recursive: true })
  await writeFile(path.join(root, 'U-x', 'Prototypes', 'Blocks', 'b', 'llm-tsx', 'block.tsx'), 'x\n')
})

afterAll(async () => {
  await rm(root, { recursive: true, force: true })
})

const variables = { ns: 'U-x', app: 'a' }

function check(declaredInputs: readonly string[], ownOutputs: readonly string[] = []) {
  return precheckStepInputs({ declaredInputs, ownOutputs, preProcRoot: root, variables })
}

describe('precheckStepInputs', () => {
  it('reports nothing when the step declares no inputs', () => {
    expect(check([])).toEqual({ missing: [], checked: [] })
  })

  it('accepts a declared input that resolves to a file', () => {
    const result = check(['Pre-Proc/{ns}/Apps/{app}/app.json'])
    expect(result.missing).toEqual([])
    expect(result.checked).toEqual(['Pre-Proc/U-x/Apps/a/app.json'])
  })

  it('reports a declared upstream input with no file under the run root', () => {
    const result = check(['Pre-Proc/{ns}/Apps/{app}/upstream.json'])
    expect(result.missing).toEqual(['Pre-Proc/U-x/Apps/a/upstream.json'])
  })

  it('skips a declared input the step itself produces', () => {
    // The own-output comparison happens before existence: a step producing the file it declares
    // is not waiting on an upstream artifact.
    const result = check(['Pre-Proc/{ns}/Apps/{app}/upstream.json'], ['Pre-Proc/U-x/Apps/a/*.json'])
    expect(result.missing).toEqual([])
    expect(result.checked).toEqual([])
  })

  it('accepts a glob input when any expansion is a file, and reports it when none is', () => {
    const withBlock = (declaredInputs: readonly string[]) => precheckStepInputs({
      declaredInputs,
      ownOutputs: [],
      preProcRoot: root,
      variables: { ...variables, block: 'b' },
    })
    expect(withBlock(['Pre-Proc/{ns}/Prototypes/Blocks/{block}/llm-tsx/*.tsx']).missing).toEqual([])
    expect(withBlock(['Pre-Proc/{ns}/Prototypes/Blocks/{block}/llm-tsx/*.css']).missing)
      .toEqual(['Pre-Proc/U-x/Prototypes/Blocks/b/llm-tsx/*.css'])
  })

  it('reports an input whose placeholder never resolved (literal path cannot exist)', () => {
    expect(check(['{module}/notes.md']).missing).toEqual(['{module}/notes.md'])
  })

  it('accepts an absolute input path that exists', () => {
    expect(check(['/etc/hosts']).missing).toEqual([])
  })

  it('treats an unusable pattern as missing (fail-closed, never a pass)', () => {
    expect(stepInputExists(root, 'Pre-Proc/U-x/Apps/a/[unterminated')).toBe(false)
  })
})

describe('resolveInputTemplate', () => {
  it('substitutes known placeholders and leaves unknown ones verbatim', () => {
    expect(resolveInputTemplate('Pre-Proc/{ns}/Apps/{app}/{other}/x', variables))
      .toBe('Pre-Proc/U-x/Apps/a/{other}/x')
  })
})
