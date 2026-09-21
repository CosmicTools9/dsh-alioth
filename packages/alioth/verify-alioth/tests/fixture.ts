/**
 * spec 共用的临时 app 目录夹具：建 `{root}/Pre-Proc/{ns}/Apps/{app}` 并写入给定文件。
 * @module @dsh-alioth/verify-alioth/tests/fixture
 */

import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

export interface TempApp {
  readonly root: string
  readonly preProcRoot: string
  readonly appDir: string
  /** 在 app 目录下写文件（自动建父目录）。 */
  write(rel: string, content: string): Promise<string>
  /** 在 `Pre-Proc/{ns}` 下写文件（Sources/、local/ 等非 app 目录产物）。 */
  writePreProc(rel: string, content: string): Promise<string>
  cleanup(): Promise<void>
}

/** 建临时 app 目录；`files` 的相对路径以 app 目录为基准。 */
export async function createTempApp(
  files: Readonly<Record<string, string>> = {},
  options: { readonly namespace?: string; readonly app?: string } = {},
): Promise<TempApp> {
  const root = await mkdtemp(path.join(tmpdir(), 'verify-alioth-'))
  const preProcRoot = path.join(root, 'Pre-Proc', options.namespace ?? 'TestNS')
  const appDir = path.join(preProcRoot, 'Apps', options.app ?? 'tm-app')
  await mkdir(appDir, { recursive: true })

  const write = async (rel: string, content: string): Promise<string> => {
    const target = path.join(appDir, rel)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, content, 'utf8')
    return target
  }
  for (const [rel, content] of Object.entries(files)) await write(rel, content)

  const writePreProc = async (rel: string, content: string): Promise<string> => {
    const target = path.join(preProcRoot, rel)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, content, 'utf8')
    return target
  }

  return {
    root,
    preProcRoot,
    appDir,
    write,
    writePreProc,
    cleanup: async () => {
      await rm(root, { recursive: true, force: true })
    },
  }
}

/** 合规的 app.json（schema_validity 满分形态）。 */
export const VALID_APP_JSON = `${JSON.stringify(
  {
    id: 'app-1',
    code: 'tm-app',
    namespace: 'TestNS',
    name: '临时应用',
    version: '0.1.0',
    status: 'active',
  },
  null,
  2,
)}\n`

/** standalone 合规的 prototype.html（无外部 CDN / 无越界引用）。 */
export const STANDALONE_PROTOTYPE_HTML = `<!doctype html>
<html lang="zh">
  <head><link rel="stylesheet" href="./app.css" /><title>t</title></head>
  <body><script src="./app.js"></script></body>
</html>
`
