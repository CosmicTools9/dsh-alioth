/**
 * Step input precheck — the fail-fast the upstream executor performs **before** it builds a
 * prompt (`Meta/backend/app-agent/src/dialog_tools/run_skill.rs:precheck_step_inputs`,
 * `FailureKind::StepInputMissing`): a step that declares `inputs` MUST NOT be started while a
 * declared upstream artifact is missing. The upstream comment records why (session 22: injecting
 * only a warning and calling anyway burned 10 slow calls for nothing).
 *
 * Mirrored rules, in upstream order:
 * - no declared inputs → nothing to check;
 * - a declared input that one of the step's own `output_glob` patterns also matches is the step's
 *   own product, not an upstream dependency → skipped;
 * - `{placeholder}` templates resolve against the run's gate variables first;
 * - existence means a **file**; a pattern containing `*` passes when any expansion is a file.
 *
 * Pure: the caller supplies the root, the variables and the step's own output patterns, so the
 * workflow bridge and the tests share one implementation.
 * @module @dsh-alioth/skill-alioth/step-inputs
 */

import { globSync, statSync } from 'node:fs'
import path from 'node:path'

export interface StepInputPrecheckInput {
  /** `Step.inputs` — declared upstream artifacts, as the adapter writes them. */
  readonly declaredInputs: readonly string[]
  /** The step's own output patterns (already template-resolved) — its own products. */
  readonly ownOutputs: readonly string[]
  /** Run root (`GateContext.preProcRoot`); a leading `Pre-Proc/` is stripped like the reader does. */
  readonly preProcRoot: string
  /** Gate variables for `{ns}` / `{app}` substitution. */
  readonly variables: Readonly<Record<string, string>>
}

export interface StepInputPrecheckResult {
  /** Declared inputs that resolve to no file, in declaration order (empty = step may start). */
  readonly missing: readonly string[]
  /** Every declared input that was actually checked (own products excluded). */
  readonly checked: readonly string[]
}

/** Substitute the run's `{placeholder}` variables, leaving unknown ones verbatim. */
export function resolveInputTemplate(template: string, variables: Readonly<Record<string, string>>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => variables[key] ?? match)
}

/** `Pre-Proc/{ns}/…` is written root-relative by adapters; the reader strips the prefix. */
function withoutRootPrefix(target: string): string {
  return target.startsWith('Pre-Proc/') ? target.slice('Pre-Proc/'.length) : target
}

/** Does the (already template-resolved) pattern resolve to a file under the run root? */
export function stepInputExists(preProcRoot: string, pattern: string): boolean {
  const relative = withoutRootPrefix(pattern)
  if (!relative.includes('*')) {
    return statSync(path.resolve(preProcRoot, relative), { throwIfNoEntry: false })?.isFile() ?? false
  }
  try {
    // `DEFAULT_SKIP_DIRS` equivalent: `node_modules`/`.git` trees are never step inputs.
    return globSync(relative, { cwd: preProcRoot, withFileTypes: true, exclude: entry => entry.name === 'node_modules' || entry.name === '.git' })
      .some(entry => entry.isFile())
  } catch {
    return false // an unusable pattern is a missing input, never a pass
  }
}

/**
 * Run the precheck.
 * @param input - declared inputs, own outputs, root and variables.
 * @returns the missing declarations (with the checked list for evidence).
 */
export function precheckStepInputs(input: StepInputPrecheckInput): StepInputPrecheckResult {
  const missing: string[] = []
  const checked: string[] = []
  for (const declared of input.declaredInputs) {
    const resolved = resolveInputTemplate(declared, input.variables)
    if (input.ownOutputs.some(pattern => path.matchesGlob(resolved, pattern))) {
      continue // this step produces it
    }
    checked.push(resolved)
    if (!stepInputExists(input.preProcRoot, resolved)) {
      missing.push(resolved)
    }
  }
  return { missing, checked }
}
