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
 * 运行位置无关：本文件会被 COPY 到 `/tmp` 之类「不在任何 `type: module` 包内」的路径执行，那里 tsx 按
 * **CJS** 加载，顶层 await 与 `import.meta` 都不可用（CI 实测：`Top-level await is currently not
 * supported with the "cjs" output format`）。故：逻辑包在 `main()` 里，脚本目录用 `argv[1]` 推导。
 *
 * 用法：`node --import tsx scripts/build-harness-web.ts [harnessRoot]`（缺省 `<脚本>/../../deepseek-harness`，
 * 即消费者仓库的同级 harness 检出；在别处执行时显式传 harnessRoot）。
 * @module @dsh-alioth/scripts/build-harness-web
 */

import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, rmSync } from 'node:fs'
import path from 'node:path'

/** 本脚本所在目录：`argv[1]` 是入口路径，CJS 与 ESM 下都成立。 */
const scriptDir = path.dirname(process.argv[1] ?? process.cwd())
const harnessRoot = path.resolve(process.argv[2] ?? path.join(scriptDir, '..', '..', 'deepseek-harness'))

/** harness 侧的 helper 面（我们只借用它算环境与写记录，不重写）。 */
interface ClientBuildHelper {
  readonly CLIENT_BUILD_PROFILE_SELECTOR: string
  readonly CLIENT_BUILD_RECORD_PATH: string
  repositoryClientBuildEnvironment(root: string, env: NodeJS.ProcessEnv): Record<string, string>
  resolveClientBuildEnvironment(repository: Record<string, string>, profile: string | undefined): Record<string, string>
  clientBuildProcessEnvironment(env: NodeJS.ProcessEnv, client: Record<string, string>): NodeJS.ProcessEnv
  writeClientBuildRecord(root: string, client: Record<string, string>): { artifacts: { fileCount: number } }
}

async function main(): Promise<void> {
  const helperPath = path.join(harnessRoot, 'scripts', 'client-build-environment.ts')
  if (!existsSync(helperPath)) {
    throw new Error(`build-harness-web: ${harnessRoot} 不是 harness checkout（缺 scripts/client-build-environment.ts）`)
  }
  const viteBin = path.join(harnessRoot, 'apps', 'web', 'node_modules', '.bin', 'vite')
  if (!existsSync(viteBin)) {
    throw new Error(`build-harness-web: 缺 ${viteBin} —— 先在 harness 里安装依赖（pnpm install），否则 web 面建不出来`)
  }

  // 容器构建上下文里没有 `.git`（.dockerignore 排除），而 harness 的 helper 只有 commit hash 依赖
  // git（dirty 探针在非 git 树里返回 undefined，不抛）。它认预置的 DSH_CLIENT_COMMIT_HASH，所以在
  // 无 `.git` 时用环境顶替：优先 CI 注入的 DSH_BUILD_COMMIT/GITHUB_SHA，否则一个中性占位符。
  // 不做这一步，Docker 构建会以 `Command failed: git rev-parse HEAD` 失败（CI 实测）。
  if (!existsSync(path.join(harnessRoot, '.git')) && !process.env['DSH_CLIENT_COMMIT_HASH']) {
    const injected = process.env['DSH_BUILD_COMMIT'] ?? process.env['GITHUB_SHA']
    const commit = injected !== undefined && /^[0-9a-f]{7,40}$/i.test(injected) ? injected.slice(0, 7) : '0000000'
    process.env['DSH_CLIENT_COMMIT_HASH'] = commit
    process.stdout.write(`build-harness-web: 无 .git（容器构建）→ commit=${commit}\n`)
  }

  // 顺序与 harness 的 scripts/build.ts 一致：先算环境 → 构建 → 最后写记录。
  const helper = await import(helperPath) as ClientBuildHelper
  const repositoryEnvironment = helper.repositoryClientBuildEnvironment(harnessRoot, process.env)
  const clientEnvironment = helper.resolveClientBuildEnvironment(
    repositoryEnvironment,
    process.env[helper.CLIENT_BUILD_PROFILE_SELECTOR],
  )
  rmSync(path.join(harnessRoot, helper.CLIENT_BUILD_RECORD_PATH), { force: true })

  const built = spawnSync(viteBin, ['build'], {
    cwd: path.join(harnessRoot, 'apps', 'web'),
    env: helper.clientBuildProcessEnvironment(process.env, clientEnvironment),
    stdio: 'inherit',
  })
  if (built.status !== 0) {
    throw new Error(
      `build-harness-web: vite build 退出码 ${String(built.status ?? built.signal)} —— 控制台会加载不到新客户端包，`
      + '别带着过期 web 面发布',
    )
  }

  const record = helper.writeClientBuildRecord(harnessRoot, clientEnvironment)
  const committed = JSON.parse(
    readFileSync(path.join(harnessRoot, helper.CLIENT_BUILD_RECORD_PATH), 'utf8'),
  ) as { environment?: Record<string, string> }
  process.stdout.write(
    `build-harness-web: ${String(record.artifacts.fileCount)} 个客户端资产，`
    + `记录 commit=${committed.environment?.['DSH_CLIENT_COMMIT_HASH'] ?? '?'}（harness ${harnessRoot}）\n`,
  )
}

main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
})
