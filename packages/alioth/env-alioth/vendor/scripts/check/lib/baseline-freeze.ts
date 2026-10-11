/**
 * baseline-freeze.ts — `check-all.sh` 基线冻结的**判定入口**（判定表唯一实现 = `./baseline`）
 * （openspec: harden-check-all-baseline-freeze；spec: precommit-execution
 *  :: check-all-baseline-freeze-reproducibility）
 *
 * 为什么存在：`check-all-baseline.json` 是**状态快照**（`<item> → pass|skip|fail`），其正确性隐含
 * 前提 = 「快照在**与判定面一致的树**上产生」。共享 checkout 下该前提极易被破坏——2026-10-04 实测：
 * 他会话对 10 个文件有**未提交**修复，一次 `--init` 把 `id-json-precision` 冻结为 `pass`；
 * 隔离验证（提交树）随后真实报 23 处违规，呈现形态却是「🆕 回归（基线 pass → 现 fail）」——
 * **掩盖了提交树真实回归**，且归因成本极高。
 *
 * 两条纪律（判定表见 change design）：
 *   ① 冻结 MUST 在可复现面上产生：脏面默认**拒绝**（`check-all.sh --init`）；显式放行时 MUST 标记 `dirtyFace`。
 *   ② 标记 MUST **剥夺豁免力**：`dirtyFace` 条目的 `pass` 基线 MUST NOT 豁免当前 `fail`。
 *
 * 本模块 = 纯判定（无 IO 副作用；`--self-test` 级断言在 `baseline-freeze.test.ts`，经 `scripts-unit-tests` 常驻）。
 *
 * CLI（供 `check-all.sh` bash 侧调用；语义与纯函数同源）:
 *   bun baseline-freeze.ts classify                     # stdin 逐行 <= `git status --porcelain` 输出
 *   bun baseline-freeze.ts entry-exempts --base-status pass --dirty-face true
 * 退出码: classify 恒 0；entry-exempts 0=豁免（grandfathered）｜1=不豁免｜2=参数错误
 */
import { exempts } from './baseline';

/** 脏面分类结果（`check-all.sh --init` 的冻结决策输入） */
export interface FreezeClass {
  /** 是否存在未提交改动（决定「拒绝冻结」或「标记 dirtyFace」） */
  dirtyFace: boolean;
  /** 未提交路径条目数（porcelain 行数；重命名算一行） */
  count: number;
  /** 展示用样例行（≤8 条，已截断；MUST NOT 用作判据） */
  sample: string[];
}

/**
 * 从 `git status --porcelain` 行集分类（单一实现）。
 * 判据只取「有无」与「计数」——**不做**逐条目 → 判据面的映射：item 的判据面无法机械可得
 * （各门禁扫描面各异），故语义只能是「树级不可复现 ⇒ 整份基线不具豁免力」（design D2）。
 */
export function classifyFreeze(porcelainLines: readonly string[]): FreezeClass {
  const lines = porcelainLines.map((l) => l.trimEnd()).filter((l) => l.length > 0);
  return {
    dirtyFace: lines.length > 0,
    count: lines.length,
    sample: lines.slice(0, 8).map((l) => (l.length > 120 ? `${l.slice(0, 120)}…` : l)),
  };
}

/**
 * 基线条目是否**豁免**当前 `fail`（判定表，唯一实现）。语义 = 「冻结时的存量结果，不拦截」：
 *
 * | base status | dirtyFace | 判定 |
 * |---|---|---|
 * | `pass` | 任意 | **不豁免**（pass→fail = 回归，既有语义；本 change 只加归因提示） |
 * | 缺席（`''`） | 任意 | **不豁免**（新项失败，既有语义） |
 * | `grandfathered` / `skip` / 其它 | false | 豁免（存量，既有语义，逐字不变） |
 * | `grandfathered` / `skip` / 其它 | true | **不豁免**（本 change 的严格化：脏面冻结的存量结果不得掩盖提交面失败） |
 *
 * 不变量：`dirtyFace` 只**增加**上报（MUST NOT 减少）——否则它就成了新的掩盖通道。
 */
export function entryExemptsBaseline(baseStatus: string, dirtyFace: boolean): boolean {
  // 判定单源 = ./baseline（本函数为薄 adapter，供既有 CLI/测试保持签名与语义不变）
  return exempts(baseStatus, dirtyFace);
}

/** 判据自证（供 `--self-test`；断言打在纯函数上） */
export function selfTest(): number {
  const cases: [string, FreezeClass, boolean][] = [
    ['空 porcelain ⇒ 干净面（可冻结）', classifyFreeze([]), false],
    ['单行 ⇒ 脏面', classifyFreeze([' M a/b.ts']), true],
    ['空行/尾随空 ⇒ 不计', classifyFreeze(['', '  ', ' M x.ts', '']), true],
    ['样例行截断到 8 条', classifyFreeze(Array.from({ length: 12 }, (_, i) => ` M f${i}.ts`)), true],
  ];
  let failed = 0;
  for (const [name, got, wantDirty] of cases) {
    const ok = got.dirtyFace === wantDirty && (name.includes('截断') ? got.sample.length === 8 : true);
    if (!ok) failed++;
    console.log(`${ok ? '✔' : '✘'} ${name}（dirtyFace=${got.dirtyFace} count=${got.count}）`);
  }
  const exemptCases: [string, string, boolean, boolean][] = [
    ['pass + 无标记 ⇒ 不豁免（pass→fail 即回归，既有语义）', 'pass', false, false],
    ['pass + 标记 ⇒ 不豁免（同左 + 归因提示）', 'pass', true, false],
    ['grandfathered + 无标记 ⇒ 豁免（存量，既有语义）', 'grandfathered', false, true],
    ['grandfathered + 标记 ⇒ 不豁免（本 change 的严格化）', 'grandfathered', true, false],
    ['skip + 无标记 ⇒ 豁免', 'skip', false, true],
    ['skip + 标记 ⇒ 不豁免', 'skip', true, false],
    ['缺席 ⇒ 不豁免（新项失败）', '', false, false],
    ['缺席 + 标记 ⇒ 不豁免', '', true, false],
  ];
  for (const [name, base, dirty, want] of exemptCases) {
    const got = entryExemptsBaseline(base, dirty);
    if (got !== want) failed++;
    console.log(`${got === want ? '✔' : '✘'} ${name}`);
  }
  console.log(failed === 0 ? '✅ baseline-freeze --self-test 通过（10 例）' : `❌ --self-test 失败 ${failed}/10`);
  return failed === 0 ? 0 : 1;
}

if (import.meta.main) {
  const [cmd, ...rest] = process.argv.slice(2);
  const flag = (name: string): string | null => {
    const i = rest.indexOf(name);
    return i >= 0 && i + 1 < rest.length ? rest[i + 1] : null;
  };
  if (cmd === 'classify') {
    const input = await new Response(Bun.stdin.stream()).text();
    console.log(JSON.stringify(classifyFreeze(input.split('\n'))));
    process.exit(0);
  }
  if (cmd === 'entry-exempts') {
    const base = flag('--base-status');
    const dirty = flag('--dirty-face');
    if (base === null || dirty === null) {
      console.error("用法: entry-exempts --base-status <pass|grandfathered|skip|空串> --dirty-face <true|false>");
      process.exit(2);
    }
    process.exit(entryExemptsBaseline(base, dirty === 'true') ? 0 : 1);
  }
  if (cmd === '--self-test') process.exit(selfTest());
  console.error('用法: classify | entry-exempts --base-status S --dirty-face B | --self-test');
  process.exit(2);
}
