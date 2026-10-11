/**
 * repo-role.ts — 仓角色（platform / ns）与「本 ns DSN」的 TS 侧入口
 *
 * 规约来源：`openspec/specs/deploy-per-namespace-db :: gate-dsn-resolution-by-repo-role`
 *           ＋ `precommit-execution :: 门禁档位 MUST 由仓库角色判定`
 *
 * 为什么需要：拆分后同一套门禁在两类仓运行，而 DB 判定面必须随仓角色分流。ns 仓若沿用平台共享
 * dev 库缺省值，判定面就是**别的仓的库**（错误判定，劣于诚实 skip）。判据与 DSN 解析 MUST 与
 * shell 侧同源（`scripts/lib/repo-role.sh` / `scripts/lib/db-url-resolve.sh`）——本模块**不重写**
 * 解析逻辑，只提供角色判据（marker 存在性，与 `scripts/pre/repo-profile.ts` 同锚点）与对 shell
 * 单一实现的进程内缓存包装（MUST NOT 出现第二份缺省值/第二套组件扫描）。
 */
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { join } from "node:path";

/** ns 仓标记（判据唯一来源；`scripts/pre/repo-profile.ts` 同值导入）。 */
export const NS_REPO_MARKER = ".alioth/platform.conf";

export type RepoRole = "platform" | "ns";

/** 角色判据（纯函数，无 IO 缓存语义）：仓根含 marker ⇒ ns 仓。 */
export function repoRoleOf(root: string): RepoRole {
  return existsSync(join(root, NS_REPO_MARKER)) ? "ns" : "platform";
}

/** 仓根（git toplevel）；非 git 仓 ⇒ null（调用方自行决定缺省）。 */
export function repoRootOf(start: string = process.cwd()): string | null {
  const res = spawnSync("git", ["-C", start, "rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  });
  const top = (res.stdout ?? "").trim();
  return res.status === 0 && top !== "" ? top : null;
}

/**
 * 仓根的**物理**形态（符号链接被解析，= shell 侧 `repo_root_physical`）。
 *
 * ns 仓经符号链接**消费**平台面（`scripts` / `Framework` / …）、平台仓经 `Pre-Proc/{ns}` 看到
 * ns 内容 ⇒ 一切「按路径写入 / 格式化」的前置判定 MUST 用物理形态——纯字符串前缀比较会被
 * 符号链接骗过（误判「在仓内」，越界写在**对方仓**留痕且本仓 `git status` 看不见）。
 */
export function repoRootPhysical(start: string = process.cwd()): string | null {
  const top = repoRootOf(start);
  if (top === null) return null;
  try {
    return realpathSync(top);
  } catch {
    return null;
  }
}

/** 本 ns DB 解析结果（`url = null` ⇒ 本 ns env 不可得，调用方 MUST NOT 回落平台库缺省）。 */
export interface RepoDbUrl {
  url: string | null;
  source: string;
}

const cache = new Map<string, RepoDbUrl>();

/**
 * 解析本仓 DSN（委托 shell 单一实现 `db-url-resolve.sh`；按「仓根 + 显式 env」缓存）。
 *
 * 语义：显式 `DATABASE_URL` 环境变量 > ns 仓的本 ns env（`Deploy/{ns}/.env`）> 平台仓的组件
 * `.mise.toml [env]`（shell 侧扫描）。ns 仓解析不到 ⇒ `url = null`（不回落平台库缺省）。
 *
 * 缓存键 MUST 含显式 `DATABASE_URL`：解析结果随该变量变化，仅按 root 缓存会让「同进程内先解析
 * 一次、再设 env」的调用拿到陈旧值 ⇒ 优先级语义被缓存覆盖（2026-09-30 实测：`repo-role.test.ts`
 * 的「显式 DATABASE_URL 优先级最高」用例恒 fail，根因即缓存命中绕过 env）。
 */
export function resolveRepoDbUrl(root: string): RepoDbUrl {
  const key = `${root}\u0000${process.env.DATABASE_URL ?? ""}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  const lib = join(import.meta.dirname, "db-url-resolve.sh");
  // `bash -c` 内 source 后取两值（url=… / source=… 两行，单一实现见 db-url-resolve.sh）
  // env 显式传入：bun 的 `spawnSync` **不继承运行期对 `process.env` 的后续写入**（子进程拿进程启动
  // 快照）⇒ 省略会让子 shell 看不见调用方刚设置的 `DATABASE_URL`：① 单测「显式 env 优先级最高」恒
  // fail；② 真机上「解析前注入 env」的调用会静默改连本 ns env 指向的库（2026-09-30 实测定位）。
  const res = spawnSync(
    "bash",
    ["-c", '. "$1" && db_url_resolve_report', "bash", lib],
    {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env },
    },
  );
  let url: string | null = null;
  let source = "";
  if (res.status === 0) {
    for (const line of (res.stdout ?? "").split("\n")) {
      if (line.startsWith("url="))
        url = line.slice(4).trim() === "" ? null : line.slice(4).trim();
      else if (line.startsWith("source=")) source = line.slice(7).trim();
    }
  }
  const out: RepoDbUrl = { url, source };
  cache.set(key, out);
  return out;
}

/** 解析不到时的哨兵 DSN：**快速失败**，MUST NOT 落到本机默认库（`psql` 空参即连本机缺省库）。 */
export const UNRESOLVED_DSN =
  "postgres://unresolved@127.0.0.1:1/aliothstudio_unresolved";
