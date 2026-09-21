/**
 * 门禁程序白名单的**生效来源**可见性（上游 `add-appagent-degradation-evidence` 的落地）。
 *
 * 上游 `RunCommandTool` 从 `skill-adapters/_runtime.yaml` 读 `allowed_programs`；
 * 文件缺失 / 不可读 / 清单为空 / 解析失败都回退代码常量（`gate_program_whitelist()`），
 * 且回退 MUST 留**可区分**的证据（`allowed_programs_status()`：`file` / `code_default`
 * + 降级原因），降级不得只活在日志里。
 *
 * 本模块镜像 workflow 插件读取的同一路径（`<modelDir>/skill-adapters/_runtime.yaml`，
 * 见 `tool-alioth-workflow/src/index.ts` 的 `allowedGatePrograms`），并额外区分
 * 「解析失败」与「清单为空」——两态在 `parseRuntimeAllowedPrograms` 里都退化为空数组，
 * 故这里先用 YAML 解析器判可解析性，再用该函数取清单（单一提取口径）。
 * 每次调用现读，不做缓存：白名单放行面不受来源影响，本模块只负责**可见性**。
 * @module @dsh-alioth/guard-alioth/whitelist
 */

import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { parse as parseYaml } from 'yaml'
import { GATE_PROGRAM_WHITELIST, parseRuntimeAllowedPrograms } from '@dsh-alioth/skill-alioth'

/** 运行时镜像相对模型快照的路径（与 workflow 插件的读取点一致）。 */
export const RUNTIME_MIRROR_PATH = path.join('skill-adapters', '_runtime.yaml')

/** 生效来源：`file` = 人工治理的运行时镜像；`code_default` = 回退代码常量。 */
export type AllowedProgramsSource = 'file' | 'code_default'

/** 只读状态：清单 + 生效来源 + 降级原因（`file` 时 reason 说明来源本身）。 */
export interface WhitelistSourceReport {
  readonly source: AllowedProgramsSource
  readonly reason: string
  readonly programs: readonly string[]
}

function codeDefault(reason: string): WhitelistSourceReport {
  return { source: 'code_default', reason, programs: [...GATE_PROGRAM_WHITELIST] }
}

/**
 * 判定一次镜像读取的生效来源。镜像读取结果由调用点给出（IO 在边界，判定是纯函数）。
 * @param mirror - `{ content }` 读成功；`{ error }` 缺失或不可读。
 */
export function classifyRuntimeMirror(
  mirror: { readonly content: string } | { readonly error: string },
): WhitelistSourceReport {
  if ('error' in mirror) {
    return codeDefault(`运行时镜像缺失或不可读：${mirror.error}`)
  }
  try {
    parseYaml(mirror.content)
  } catch (error) {
    return codeDefault(`解析失败：${error instanceof Error ? error.message : String(error)}`)
  }
  const programs = parseRuntimeAllowedPrograms(mirror.content)
  if (programs.length === 0) {
    return codeDefault('allowed_programs 为空')
  }
  return {
    source: 'file',
    reason: `${RUNTIME_MIRROR_PATH}（人工治理的运行时镜像，${programs.length} 条程序）`,
    programs,
  }
}

/** 读取模型快照的运行时镜像并判定生效来源（缺失/不可读 → `code_default`，绝不静默）。 */
export async function readWhitelistSource(modelDir: string): Promise<WhitelistSourceReport> {
  const mirror = path.join(modelDir, RUNTIME_MIRROR_PATH)
  return classifyRuntimeMirror(
    await readFile(mirror, 'utf8')
      .then(content => ({ content }))
      .catch((error: unknown) => ({ error: `${mirror}: ${error instanceof Error ? error.message : String(error)}` })),
  )
}
