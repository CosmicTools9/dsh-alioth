/**
 * External-program gate runner. The adapter gates declare `program` checks
 * (e.g. `bun scripts/prototype-tool.js build ...`) that the distribution
 * executes with bun; porting that toolchain to TS is out of scope, so the
 * deployment must provide the binary. This module offers the runner contract
 * (`gates.checkGate`'s `runProgram` hook) plus bun availability probing —
 * a missing binary fails the gate with a clear message, never silently.
 * @module @dsh-alioth/skill-alioth/bun
 */

import { spawn } from 'node:child_process'
import type { ProgramResult, ProgramRunner } from './gates.ts'

export interface ProgramRunnerOptions {
  /** Working directory for spawned programs (adapter scripts resolve relative to it). */
  readonly cwd?: string
  /** Kill the program after this many ms (overridden by the gate's `timeout_sec`). */
  readonly timeoutMs?: number
  /** Extra environment for spawned programs (merged over process.env). */
  readonly env?: Readonly<Record<string, string>>
}

const DEFAULT_TIMEOUT_MS = 300_000

/**
 * Create a `runProgram` hook: spawn, capture output tails, resolve on close.
 * ENOENT and non-zero exits become `ok: false` with evidence; the gate's
 * `timeout_sec` (× 1000 ms) wins over the default when set.
 */
export function createProgramRunner(options: ProgramRunnerOptions = {}): ProgramRunner {
  return (program, args, gate) => {
    const timeoutMs = gate.kind === 'program' && gate.timeoutSec > 0
      ? gate.timeoutSec * 1000
      : options.timeoutMs ?? DEFAULT_TIMEOUT_MS
    const { promise, resolve } = Promise.withResolvers<ProgramResult>()
    const child = spawn(program, [...args], {
      cwd: options.cwd,
      timeout: timeoutMs,
      env: options.env === undefined ? process.env : { ...process.env, ...options.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout?.on('data', chunk => { stdout += String(chunk) })
    child.stderr?.on('data', chunk => { stderr += String(chunk) })
    child.on('error', error => resolve({ ok: false, exitCode: null, detail: `spawn ${program} failed: ${error.message}` }))
    child.on('close', code => {
      if (code === 0) {
        resolve({ ok: true, exitCode: 0, detail: `${program} exited 0${stdout.length > 0 ? `: ${stdout.trim().slice(-300)}` : ''}` })
        return
      }
      if (code === null) {
        resolve({ ok: false, exitCode: null, detail: `${program} timed out after ${Math.round(timeoutMs / 1000)}s${stderr.length > 0 ? `: ${stderr.trim().slice(-300)}` : ''}` })
        return
      }
      resolve({ ok: false, exitCode: code, detail: `${program} exited ${String(code)}${stderr.length > 0 ? `: ${stderr.trim().slice(-300)}` : ''}` })
    })
    return promise
  }
}

/** Probe whether `bun` is on PATH. */
export function bunAvailable(): Promise<boolean> {
  return createProgramRunner({ timeoutMs: 15_000 })('bun', ['--version'], {
    kind: 'program',
    program: 'bun',
    args: ['--version'],
    expectedExitCode: 0,
    timeoutSec: 15,
  }).then(result => result.ok)
}
