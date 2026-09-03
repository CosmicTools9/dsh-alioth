/**
 * E2E verification evidence report — wire-compatible with the upstream
 * AppAgent `write_e2e_report` (dialog_tools/e2e_verify.rs): the JSON file
 * lands at `Pre-Proc/{ns}/Apps/{app}/e2e-report.json` for human review and
 * pipeline evidence retrieval. @module @dsh-alioth/skill-alioth/e2e-report
 */

import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'

export const E2E_REPORT_SCHEMA_VERSION = '1.0'

/** One verification check row (upstream `CheckItem`). */
export interface E2eCheck {
  readonly id: string
  readonly passed: boolean
  readonly description: string
}

/** The e2e-report.json shape (upstream `write_report` JSON literal). */
export interface E2eReport {
  readonly app: string
  readonly namespace: string
  readonly attempt: number
  readonly passed: boolean
  readonly checks: readonly E2eCheck[]
  readonly note: string
}

/** Assemble the report document (adds schema_version and timestamp). */
export function buildE2eReport(report: E2eReport): Record<string, unknown> {
  return {
    schema_version: E2E_REPORT_SCHEMA_VERSION,
    app: report.app,
    namespace: report.namespace,
    attempt: report.attempt,
    passed: report.passed,
    checks: report.checks.map(check => ({ id: check.id, passed: check.passed, description: check.description })),
    failures: report.checks
      .filter(check => !check.passed)
      .map(check => ({ id: check.id, description: check.description })),
    note: report.note,
    ts: new Date().toISOString(),
  }
}

/** Write `e2e-report.json` under the app dir (`Pre-Proc/{ns}/Apps/{app}`). */
export async function writeE2eReport(appDir: string, report: E2eReport): Promise<string> {
  await mkdir(appDir, { recursive: true })
  const target = path.join(appDir, 'e2e-report.json')
  await writeFile(target, `${JSON.stringify(buildE2eReport(report), null, 2)}\n`, 'utf8')
  return target
}
