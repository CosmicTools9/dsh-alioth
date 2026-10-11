#!/usr/bin/env bun
/**
 * temp-dirs.ts — 进程级临时目录的唯一实现：创建 + 同前缀残留收割 + 退出/信号清理。
 *
 * 为什么存在（2026-10-09 实测）：全仓 117 个 `mkdtemp*` 调用点各自
 * `mkdtempSync(join(tmpdir(), "<prefix>-"))` 并在 `finally` 里 `rmSync` —— `finally` 对
 * SIGKILL / 超时被杀**不可达**，残留无人收割。实测 `$TMPDIR` 累积：
 * `ns-frontend-drift-empty-*`（180+，其中一部分是**无条件泄漏**：mkdtemp 直接内联进表达式、
 * 从未注册清理）、`parallel-test-*`（90+）、`pc-contract-viol.*`（38）、`chk-env-setup-*`（30）、
 * `seed-convergence-*`（35）、`seed-conv-gate-*`、`ns-app-host-pkg-*`、`idx-staged-*`（57 MB）等。
 *
 * 契约（三条，缺一即漏）：
 *   ① 创建 MUST 经 `tempDir(prefix)`；前缀即归属判据（他者不会用同一前缀）。
 *   ② 创建**前** MUST 收割同前缀且 mtime 早于 `TEMP_DIR_MIN_AGE_MS` 的残留 —— 这是杀不死的
 *      兜底路径：SIGKILL 下没有任何代码能运行，只能靠下一次运行自愈。
 *   ③ 正常退出与 SIGINT/SIGTERM MUST 删除本次创建的目录（best-effort，失败不抛出、绝不阻断判定）。
 *
 * 边界（2026-10-09 实测，**测试面必须知道**）：
 *   · `bun test` **不触发** `process.on("exit")` 与 `beforeExit`（对照：`bun run` 触发）⇒ 契约 ③
 *     在测试进程内**不成立**，测试面的兜底只有契约 ②（下次运行 / 门禁入口 sweep）。
 *   · 相应地，夹具 MUST NOT 建在 `describe` 体内：bun 实测 **skipped suite 的 describe-body 仍执行，
 *     而 beforeAll/afterAll 都不执行** ⇒ body 里建夹具必然残留（`ns-app-host-pkg-*` /
 *     `ns-app-host-reg-*` 曾每跑一次留一份）。夹具 MUST 建在 `beforeAll`，释放用 `releaseTempDir`。
 */
import { mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { join } from "node:path";

/** 残留收割的活跃保护窗口：窗口内被写入的同前缀目录视为「可能有并发会话在用」，不删。 */
export const TEMP_DIR_MIN_AGE_MS = 10 * 60_000;

/**
 * 已登记前缀 → 归属（创建该前缀的文件）。
 * 用途：`sweepRegisteredTempDirs()` 在门禁入口一次性回收**所有**前缀的历史残留——
 * 覆盖「本次没跑到」的检查（被 SIGKILL 的那些正是永远不会自己清理的）。
 */
export const TEMP_DIR_PREFIXES: Record<string, string> = {
  "alioth-gatefiles-": "scripts/pre/run-gates.ts",
  "chk-env-setup-": "scripts/check/check-env-setup.ts",
  "host-reg-": "scripts/runtime/ns-app-host.test.ts",
  "idx-staged-": "scripts/check/check-openspec-index-drift.ts",
  "idx-view-": "scripts/check/check-openspec-index-drift.ts",
  "index-drift-": "scripts/check/check-openspec-index-drift.ts",
  "ns-app-host-conf-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-crossform-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-fakebin-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-norunner-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-pkg-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-reg-": "scripts/runtime/ns-app-host.test.ts",
  "ns-app-host-set-": "scripts/runtime/ns-app-host.test.ts",
  "ns-frontend-drift-empty-": "scripts/check/check-ns-deploy-frontend-drift.ts",
  "ns-frontend-drift-self-test-":
    "scripts/check/check-ns-deploy-frontend-drift.ts",
  "ns-frontend-drift-src-": "scripts/check/check-ns-deploy-frontend-drift.ts",
  "parallel-test-": "scripts/parallel/store.test.ts",
  "pc-contract-viol.": "scripts/check/check-prototype-contract.sh",
  "seed-conv-gate-": "scripts/check/check-seed-convergence.test.ts",
  "seed-convergence-": "scripts/seed/seed-convergence.test.ts",
};

/** 本次进程创建的临时目录（退出/信号时统一删除）。 */
const liveDirs = new Set<string>();
let disposalWired = false;

function disposeAll(): void {
  for (const dir of liveDirs) {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {
      /* best-effort：清理失败绝不影响退出码 */
    }
  }
  liveDirs.clear();
}

/** 接线退出与信号处理（幂等；信号处理清理后按默认语义重新抛出）。 */
function wireDisposal(): void {
  if (disposalWired) return;
  disposalWired = true;
  process.once("exit", disposeAll);
  for (const sig of ["SIGINT", "SIGTERM"] as const) {
    process.once(sig, () => {
      disposeAll();
      process.removeAllListeners(sig);
      process.kill(process.pid, sig);
    });
  }
}

/**
 * 收割同前缀残留：跳过 `keep` 与 `TEMP_DIR_MIN_AGE_MS` 内有写入者，逐个 `rmSync(force)`。
 * 返回被删除的绝对路径（供日志/证据）。
 */
export function reapTempDirs(
  prefix: string,
  opts: { keep?: string; minAgeMs?: number } = {},
): string[] {
  const minAgeMs = opts.minAgeMs ?? TEMP_DIR_MIN_AGE_MS;
  const now = Date.now();
  const reaped: string[] = [];
  let entries: string[];
  try {
    entries = readdirSync(tmpdir()).filter((n) => n.startsWith(prefix));
  } catch {
    return reaped;
  }
  for (const name of entries) {
    const full = join(tmpdir(), name);
    if (full === opts.keep) continue;
    try {
      if (!statSync(full).isDirectory()) continue;
      if (now - statSync(full).mtimeMs < minAgeMs) continue;
      rmSync(full, { recursive: true, force: true });
      reaped.push(full);
    } catch {
      /* 与并发会话竞争（目录刚被删）等一律跳过 */
    }
  }
  return reaped;
}

/** 收割全部已登记前缀的残留（门禁入口的兜底调用）。 */
export function sweepRegisteredTempDirs(
  opts: { minAgeMs?: number } = {},
): string[] {
  return Object.keys(TEMP_DIR_PREFIXES).flatMap((prefix) =>
    reapTempDirs(prefix, opts),
  );
}

/**
 * 建临时目录（前缀式）：先收割同前缀残留，再创建，并登记退出/信号清理。
 * 返回绝对路径。同一进程内可多次调用（各自独立登记）。
 */
export function tempDir(
  prefix: string,
  opts: { minAgeMs?: number } = {},
): string {
  reapTempDirs(prefix, opts);
  const dir = mkdtempSync(join(tmpdir(), prefix));
  wireDisposal();
  liveDirs.add(dir);
  return dir;
}

/** 显式释放（`finally` 里的正常路径；重复调用安全）。 */
export function releaseTempDir(dir: string): void {
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {
    /* best-effort */
  }
  liveDirs.delete(dir);
}
