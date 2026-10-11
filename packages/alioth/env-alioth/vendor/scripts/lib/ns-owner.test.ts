/**
 * ns-owner.sh 回归（add-ns-slot-ownership-supervisor §B 契约）：
 *   · write → read/field 往返（OWNER_START 为 `ps -o lstart=` 原样值）
 *   · `ns_owner_valid` 四态：缺记录 / pid 死 / lstart 不匹配（伪造 OWNER_START）/ 全对
 *   · `ns_owner_release_request`：有效 owner（`sleep 300`）→ rc=0 **且进程真被 SIGTERM**；
 *     无记录 → rc=2；**伪造 lstart 的活 pid → 不发信号**（负例，进程仍存活）；忽略 TERM → rc=1 超时
 *   · `ns_owner_clear`：expect_pid 不匹配 → rc=1 且不删；匹配 → 删除
 *
 * 隔离：`NS_RUNTIME_ROOT`（语义同 ns_registry_file / ns_lock_dir：一个含 `<ns>` 条目的根；
 * owner 条目 = `<ns>.owner.conf`）⇒ 全程不触碰真实 `Deploy/{ns}/.runtime/owner.conf`。
 * 测试真实起进程（`Bun.spawn`），收尾统一 SIGKILL + await 回收（不留孤儿/僵尸）。
 */
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..", "..");
const LIB = join(ROOT, "scripts", "lib", "ns-owner.sh");

type Res = { code: number; stdout: string; stderr: string };

/** 以 NS_RUNTIME_ROOT=<root> 调库（source + 表达式 + 以其 rc 退出），刻意开 `set -u`
 *  以覆盖「库在 set -u 调用方下安全」这条契约。extraEnv 供 locale 交叉用例覆写 LC_ALL/LANG。 */
function call(root: string, expr: string, extraEnv: Record<string, string> = {}): Res {
  const script = `set -uo pipefail\nsource "${LIB}"\n${expr}\nrc=$?\nexit $rc\n`;
  const r = spawnSync("bash", ["-c", script], {
    cwd: ROOT,
    encoding: "utf8",
    env: { ...process.env, NS_RUNTIME_ROOT: root, ...extraEnv },
  });
  return { code: r.status ?? 1, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
}

/** 手工造记录（负例夹具：伪造 OWNER_START / 死 pid 只能用原始文本，库自身拒绝写出无效记录） */
function writeRaw(root: string, ns: string, pid: number, start: string): void {
  writeFileSync(
    join(root, `${ns}.owner.conf`),
    `LAYER=dev\nNAMESPACE=${ns}\nOWNER_PID=${pid}\nOWNER_START=${start}\nSLOTS=41719,9005,9006\nROLES=fe,be,sso\n`,
  );
}

const ownerFile = (root: string, ns = "T1"): string => join(root, `${ns}.owner.conf`);

let root = "";
const procs: Bun.Subprocess<"ignore", "ignore", "ignore">[] = [];

function spawnBash(script: string) {
  const p = Bun.spawn(["bash", "-c", script], { stdout: "ignore", stderr: "ignore" });
  procs.push(p);
  return p;
}

/** 真实活进程 fixture：bash 直接 exec ⇒ pid 即 sleep 本身（SIGTERM 直达目标） */
const spawnSleep = (seconds = 300) => spawnBash(`exec sleep ${seconds}`);

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "ns-owner-"));
});

afterEach(async () => {
  const dying = procs.splice(0).map((p) => {
    try {
      p.kill("SIGKILL");
    } catch {
      /* 已退出 */
    }
    return p.exited.catch(() => {});
  });
  await Promise.allSettled(dying); // 显式回收：不留僵尸影响后续断言
  if (root) rmSync(root, { recursive: true, force: true });
  root = "";
});

describe("owner 记录读写（单一实现）", () => {
  test("write → read/field 往返；OWNER_START = ns_owner_lstart 原样", () => {
    const live = spawnSleep();
    const sock = `${root}/supervise.sock`;
    const w = call(root, `ns_owner_write T1 dev ${live.pid} fe,be,sso 41719,9005,9006 ${sock}`);
    expect(w.code).toBe(0);

    const read = call(root, "ns_owner_read T1");
    expect(read.code).toBe(0);
    expect(read.stdout).toContain(`OWNER_PID=${live.pid}`);
    expect(read.stdout).toContain("LAYER=dev");
    expect(read.stdout).toContain("SLOTS=41719,9005,9006");
    expect(read.stdout).toContain("ROLES=fe,be,sso");
    expect(read.stdout).toContain(`CONTROL_SOCK=${sock}`);

    const lstart = call(root, `ns_owner_lstart ${live.pid}`);
    expect(lstart.code).toBe(0);
    expect(lstart.stdout.trim().length).toBeGreaterThan(0);
    expect(call(root, "ns_owner_field T1 OWNER_START").stdout.trim()).toBe(lstart.stdout.trim());
    expect(call(root, "ns_owner_field T1 NAMESPACE").stdout.trim()).toBe("T1");
    expect(call(root, "ns_owner_field T1 LAYER").stdout.trim()).toBe("dev");
  });

  test("缺失语义：field → 空串 rc=0；read → rc=1；write 死 PID → rc=1 拒写", () => {
    expect(call(root, "ns_owner_field T1 OWNER_PID").code).toBe(0);
    expect(call(root, "ns_owner_field T1 OWNER_PID").stdout).toBe("\n");
    expect(call(root, "ns_owner_read T1").code).toBe(1);
    // 99999999 在任何平台都不可为存活 PID（macOS pid_max≈99998；Linux pid_max≤4194304）
    expect(call(root, "ns_owner_write T1 dev 99999999 fe 9005 /tmp/x.sock").code).toBe(1);
    expect(existsSync(ownerFile(root))).toBe(false);
  });

  test("ns_owner_valid 四态：缺记录 / pid 死 / lstart 不匹配 / 全对", async () => {
    // (a) 缺记录
    expect(call(root, "ns_owner_valid T1").code).toBe(1);
    expect(call(root, "ns_owner_pid T1").code).toBe(1);

    // (b) pid 已死（真实起过再 reap ⇒ 该 pid 已亡）
    const dead = Bun.spawn(["bash", "-c", "exit 0"], { stdout: "ignore", stderr: "ignore" });
    await dead.exited;
    writeRaw(root, "T1", dead.pid, "Thu Jan  1 00:00:00 1970");
    expect(call(root, "ns_owner_valid T1").code).toBe(1);

    // (c) 活 pid + 伪造 OWNER_START（lstart 不匹配）
    const live = spawnSleep();
    writeRaw(root, "T1", live.pid, "Thu Jan  1 00:00:00 1970");
    expect(call(root, "ns_owner_valid T1").code).toBe(1);

    // (d) 全对
    const lstart = call(root, `ns_owner_lstart ${live.pid}`).stdout.trim();
    writeRaw(root, "T1", live.pid, lstart);
    expect(call(root, "ns_owner_valid T1").code).toBe(0);
    expect(call(root, "ns_owner_pid T1").stdout.trim()).toBe(String(live.pid));

    // (d′) locale 归一：`ps -o lstart=` 字段填充在 CJK locale 带尾随空格 ⇒ 记录带尾空白仍有效
    writeRaw(root, "T1", live.pid, `${lstart}  `);
    expect(call(root, "ns_owner_valid T1").code).toBe(0);
  });

  test("ns_owner_clear：expect_pid 不匹配 → rc=1 且不删；匹配 → 删除", () => {
    const live = spawnSleep();
    expect(call(root, `ns_owner_write T1 dev ${live.pid} fe 9005 /tmp/x.sock`).code).toBe(0);

    expect(call(root, "ns_owner_clear T1 424242").code).toBe(1);
    expect(existsSync(ownerFile(root))).toBe(true);

    expect(call(root, `ns_owner_clear T1 ${live.pid}`).code).toBe(0);
    expect(existsSync(ownerFile(root))).toBe(false);

    // 无记录：无参幂等 rc=0；带 expect_pid 无从匹配 rc=1
    expect(call(root, "ns_owner_clear T1").code).toBe(0);
    expect(call(root, "ns_owner_clear T1 424242").code).toBe(1);
  });
});

describe("locale 无关性（`ps` 渲染随调用方 locale 变化 ⇒ 单实现内钉死 LC_ALL=C）", () => {
  const ZH = { LC_ALL: "zh_CN.UTF-8", LANG: "zh_CN.UTF-8" };

  test("非 C locale 下 lstart 仍为 C 规范串（跨调用方 locale 逐字可比）", () => {
    const live = spawnSleep();
    const c = call(root, `ns_owner_lstart ${live.pid}`);
    const zh = call(root, `ns_owner_lstart ${live.pid}`, ZH);
    expect(zh.code).toBe(0);
    expect(c.stdout.trim()).not.toBe("");
    expect(zh.stdout).toBe(c.stdout); // 无 C 钉死时 zh 渲染为 `二  9月/29 …` ⇒ 此处必失败
  });

  test("交叉 locale：C 写 → zh 读 / zh 写 → C 读 均判有效", () => {
    const live = spawnSleep();
    const write = `ns_owner_write T1 dev ${live.pid} fe 9005 /tmp/x.sock`;

    expect(call(root, write).code).toBe(0); // 写者 = C
    expect(call(root, "ns_owner_valid T1", ZH).code).toBe(0); // 读者 = zh
    expect(call(root, "ns_owner_pid T1", ZH).stdout.trim()).toBe(String(live.pid));

    expect(call(root, write, ZH).code).toBe(0); // 写者 = zh
    expect(call(root, "ns_owner_valid T1").code).toBe(0); // 读者 = C
    expect(call(root, "ns_owner_field T1 OWNER_START").stdout.trim()).toBe(
      call(root, `ns_owner_lstart ${live.pid}`).stdout.trim(),
    );
  });

  test("zh 环境下请退仍识别有效 owner（rc=0 且进程被 TERM，不退回 legacy）", async () => {
    const live = spawnSleep();
    expect(call(root, `ns_owner_write T1 dev ${live.pid} fe,be,sso 41719,9005,9006 /tmp/x.sock`).code).toBe(0); // C 写
    const r = call(root, "ns_owner_release_request T1 5", ZH); // zh 读
    expect(r.code).toBe(0);
    expect(r.stderr).toContain("经 owner 请退");
    await live.exited;
    expect(live.signalCode).toBe("SIGTERM");
  });
});

describe("ns_owner_release_request（经 owner 请退）", () => {
  test("无有效记录 → rc=2（调用方 legacy 处置），不投递信号", () => {
    expect(call(root, "ns_owner_release_request T1 1").code).toBe(2);
  });

  test("有效 owner → rc=0 且进程真被 SIGTERM", async () => {
    const live = spawnSleep();
    expect(call(root, `ns_owner_write T1 dev ${live.pid} fe,be,sso 41719,9005,9006 /tmp/x.sock`).code).toBe(0);
    expect(call(root, "ns_owner_valid T1").code).toBe(0);

    const r = call(root, "ns_owner_release_request T1 5");
    expect(r.code).toBe(0);
    expect(r.stderr).toContain("经 owner 请退");
    expect(r.stderr).toContain(String(live.pid));

    await live.exited; // 被 TERM 后退出
    expect(live.signalCode).toBe("SIGTERM");
  });

  test("伪造 lstart 的活 pid → MUST NOT 投信号（进程仍存活，rc=2）", async () => {
    const live = spawnSleep();
    writeRaw(root, "T1", live.pid, "Thu Jan  1 00:00:00 1970");
    expect(call(root, "ns_owner_valid T1").code).toBe(1);

    const r = call(root, "ns_owner_release_request T1 2");
    expect(r.code).toBe(2);
    expect(r.stderr).not.toContain("经 owner 请退"); // 投信号前的日志行缺席 = 未进入 kill 分支

    // 负例需在真实时钟上留出误投信号的生效窗口：bash 子进程的 SIGTERM 投递无法被 fake timers 驱动
    const deadline = Date.now() + 400;
    while (Date.now() < deadline && live.exitCode === null) await Bun.sleep(20);
    expect(live.exitCode).toBeNull();
    expect(live.signalCode).toBeNull();
  });

  test("owner 忽略 SIGTERM → 超时 rc=1（记录与进程仍在）", () => {
    // 忽略 TERM 的 fixture（bash 自身持 SIG_IGN；exec 出去的 sleep 继承 SIG_IGN）
    const stubborn = spawnBash('trap "" TERM; exec sleep 300');
    expect(call(root, `ns_owner_write T1 dev ${stubborn.pid} fe 9005 /tmp/x.sock`).code).toBe(0);

    const t0 = Date.now();
    const r = call(root, "ns_owner_release_request T1 1");
    expect(r.code).toBe(1);
    expect(r.stderr).toContain("请退超时");
    expect(r.stderr).toContain("经 owner 请退");
    // 真实轮询等待（bash 侧 sleep 0.5s 无法被 fake timers 驱动）⇒ 至少跑满一个轮询周期
    expect(Date.now() - t0).toBeGreaterThanOrEqual(500);
    expect(stubborn.exitCode).toBeNull(); // 记录/进程仍在：交由调用方 fail-loud
  });
});
