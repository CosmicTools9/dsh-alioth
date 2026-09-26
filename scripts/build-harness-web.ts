/**
 * 为 harness checkout 构建**客户端（web）面**并写客户端构建记录。
 *
 * 控制台发的是构建产物：`apps/web/dist` + `.dsh-build/client-build-environment.json`（记录构建时的
 * commit/dirty 与公共环境值）。harness 的官方入口是 `pnpm run build`（`build:native-system` → `build:lib`
 * → **`build:web`** → `writeClientBuildRecord`），但部署机与本仓上 `pnpm run` 会被既有的锁/manifest
 * quirk 拦住（工作区外部成员 `../dsh-chess` 的 `link:` 字面量），所以这里用 harness 自己的 helper 复刻
 * `scripts/build.ts` 的 web 段——**单一入口**：Dockerfile 与运维发布脚本共用它，不各抄一份。
 *
 * 只建库不建 web 面的后果（2026-09-26 在 dev 上实测）：控制台 `Failed to load plugins`、新客户端包
 * `@deepseek-ai/dsh-client-*: failed`、~18 个客户端条目 pending、连 composer 都没有——而 `/landing` 仍
 * 200、node 侧门禁全绿，任何非浏览器判据都看不见。
 *
 * 用法：`node --import tsx scripts/build-harness-web.ts [harnessRoot]`（缺省 `../deepseek-harness`）。
 * @module @dsh-alioth/scripts/build-harness-web
 */

import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, rmSync } from 'node:fs'
import path from 'node:path'

const harnessRoot = path.resolve(process.argv[2] ?? path.join(import.meta.dirname, '..', '..', 'deepseek-harness'))
const helperPath = path.join(harnessRoot, 'scripts', 'client-build-environment.ts')
if (!existsSync(helperPath)) {
  throw new Error(`build-harness-web: ${harnessRoot} 不是 harness checkout（缺 scripts/client-build-environment.ts）`)
}
const viteBin = path.join(harnessRoot, 'apps', 'web', 'node_modules', '.bin', 'vite')
if (!existsSync(viteBin)) {
  throw new Error(`build-harness-web: 缺 ${viteBin} —— 先在 harness 里安装依赖（pnpm install），否则 web 面建不出来`)
}

// 用 harness 自己的 helper 算环境，顺序与 scripts/build.ts 一致：先算 → 构建 → 最后写记录。
const helper = await import(helperPath) as {
  CLIENT_BUILD_PROFILE_SELECTOR: string
  CLIENT_BUILD_RECORD_PATH: string
  repositoryClientBuildEnvironment(root: string, env: NodeJS.ProcessEnv): Record<string, string>
  resolveClientBuildEnvironment(repository: Record<string, string>, profile: string | undefined): Record<string, string>
  clientBuildProcessEnvironment(env: NodeJS.ProcessEnv, client: Record<string, string>): NodeJS.ProcessEnv
  writeClientBuildRecord(root: string, client: Record<string, string>): { artifacts: { fileCount: number } }
}
const repositoryEnvironment = helper.repositoryClientBuildEnvironment(harnessRoot, process.env)
const clientEnvironment = helper.resolveClientBuildEnvironment(repositoryEnvironment, process.env[helper.CLIENT_BUILD_PROFILE_SELECTOR])
rmSync(path.join(harnessRoot, helper.CLIENT_BUILD_RECORD_PATH), { force: true })

const built = spawnSync(viteBin, ['build'], {
  cwd: path.join(harnessRoot, 'apps', 'web'),
  env: helper.clientBuildProcessEnvironment(process.env, clientEnvironment),
  stdio: 'inherit',
})
if (built.status !== 0) {
  throw new Error(`build-harness-web: vite build 退出码 ${String(built.status ?? built.signal)} —— 控制台会加载不到新客户端包，别带着过期 web 面发布`)
}

const record = helper.writeClientBuildRecord(harnessRoot, clientEnvironment)
const committed = JSON.parse(readFileSync(path.join(harnessRoot, helper.CLIENT_BUILD_RECORD_PATH), 'utf8')) as {
  environment?: Record<string, string>
}
process.stdout.write(
  `build-harness-web: ${String(record.artifacts.fileCount)} 个客户端资产，记录 commit=${committed.environment?.['DSH_CLIENT_COMMIT_HASH'] ?? '?'}（harness ${harnessRoot}）\n`,
)
