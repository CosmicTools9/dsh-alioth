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

export interface ProgramResult {
  readonly ok: boolean
  readonly detail: string
}

export interface ProgramRunnerOptions {
  /** Working directory for spawned programs (adapter scripts resolve relative to it). */
  readonly cwd?: string
  /** Kill the program after this many ms. */
  readonly timeoutMs?: number
}

/**
 * Create a `runProgram` hook: spawn, capture output tails, resolve on close.
 * ENOENT and non-zero exits become `ok: false` with evidence.
 */
export function createProgramRunner(options: ProgramRunnerOptions = {}): (program: string, args: readonly string[]) => Promise<ProgramResult> {
  return (program, args) => new Promise((resolve) => {
    const child = spawn(program, [...args], {
      cwd: options.cwd,
      timeout: options.timeoutMs ?? 300_000,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout?.on('data', chunk => { stdout += String(chunk) })
    child.stderr?.on('data', chunk => { stderr += String(chunk) })
    child.on('error', error => resolve({ ok: false, detail: `spawn ${program} failed: ${error.message}` }))
    child.on('close', code => {
      if (code === 0) {
        resolve({ ok: true, detail: `${program} exited 0${stdout.length > 0 ? `: ${stdout.trim().slice(-300)}` : ''}` })
        return
      }
      resolve({
        ok: false,
        detail: `${program} exited ${String(code)}${stderr.length > 0 ? `: ${stderr.trim().slice(-300)}` : ''}`,
      })
    })
  })
}

/** Probe whether `bun` is on PATH. */
export function bunAvailable(): Promise<boolean> {
  return createProgramRunner({ timeoutMs: 15_000 })('bun', ['--version']).then(result => result.ok)
}
