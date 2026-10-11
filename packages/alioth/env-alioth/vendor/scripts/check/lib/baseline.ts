/**
 * baseline.ts — 基线**判定**的单一实现（change `deepen-check-substrate`，C3）。
 *
 * 分层（每层唯一属主）：
 *   · **IO** = `baseline-store`（INV-1 写入归属 / INV-2 读侧归属过滤）——本模块不直接读写文件
 *   · **判定** = 本模块（`partition` 比较 / `rot` 死条目 / `exempts` 冻结豁免表）
 *   · **消费** = `gate-kit.ratchet`（薄 adapter，输出格式逐字不变）+ `check-all.sh`（经 `baseline-freeze` CLI）
 *
 * 为何单源：迁移前 ≥25 份检查各自 `loadBaseline()`+`filter()`（行语法 ≈11 种）⇒「同一条规约的强制力度
 * 取决于读的是哪份检查」；`baseline-freeze` 又另持一份冻结豁免判定。本模块把两处judgment 收成一份。
 */

import { readBaselineKeys, readBaselineText } from './baseline-store';

export interface Partitioned<T> {
  /** 基线外的违规（= 新增，MUST 拦） */
  fresh: T[];
  /** 基线内的存量（grandfathered，不拦） */
  grandfathered: T[];
}

/** 比较（单一实现）：键在基线内 ⇒ 存量；否则 ⇒ 新增。 */
export function partition<T>(items: readonly T[], keys: ReadonlySet<string>, keyOf: (item: T) => string): Partitioned<T> {
  const fresh: T[] = [];
  const grandfathered: T[] = [];
  for (const item of items) (keys.has(keyOf(item)) ? grandfathered : fresh).push(item);
  return { fresh, grandfathered };
}

/**
 * 死条目（R2「基线卫生」）：基线登记但**当前不再命中**的键。
 * `liveKeys` = 本次全量扫描得到的键集（调用方提供）；返回排序后的死键。
 */
export function rot(entries: Iterable<string>, liveKeys: ReadonlySet<string>): string[] {
  const dead: string[] = [];
  for (const e of entries) if (!liveKeys.has(e)) dead.push(e);
  return dead.sort();
}

/**
 * 基线条目是否**豁免**当前 `fail`（冻结判定表的唯一实现；`baseline-freeze` 委派到此）。
 *
 * | base status | dirtyFace | 判定 |
 * |---|---|---|
 * | `pass` | 任意 | **不豁免**（pass→fail = 回归） |
 * | 缺席（`''`） | 任意 | **不豁免**（新项失败） |
 * | `grandfathered` / `skip` / 其它 | false | 豁免（存量既有语义） |
 * | `grandfathered` / `skip` / 其它 | true | **不豁免**（脏面冻结的存量不得掩盖提交面失败） |
 */
export function exempts(baseStatus: string, dirtyFace: boolean): boolean {
  if (dirtyFace) return false;
  return baseStatus !== '' && baseStatus !== 'pass';
}

/** 装载基线键（经 `baseline-store`，自动带 INV-2 归属过滤）。 */
export function loadKeys(root: string, rel: string): Set<string> {
  return readBaselineKeys(root, rel);
}

/** 基线原文（判定需要全文时用；同样经 IO 属主）。 */
export function loadText(root: string, rel: string): string | null {
  return readBaselineText(root, rel);
}

/** 判据自证（R1：含负例——豁免不得放宽、死条目不得漏报）。 */
export function selfTest(): number {
  const cases: Array<{ name: string; ok: boolean }> = [];
  const keyOf = (v: { path: string }): string => v.path;

  const items = [{ path: 'a' }, { path: 'b' }, { path: 'c' }];
  const keys = new Set(['a', 'c']);
  const p = partition(items, keys, keyOf);
  cases.push({ name: '比较：基线内 ⇒ 存量（a/c）', ok: p.grandfathered.length === 2 });
  cases.push({ name: '比较：基线外 ⇒ 新增（b）', ok: p.fresh.length === 1 && p.fresh[0]?.path === 'b' });
  cases.push({
    name: '负例：空基线 ⇒ 全部算新增（不得静默放行）',
    ok: partition(items, new Set(), keyOf).fresh.length === 3,
  });

  const dead = rot(['a', 'b', 'zombie'], new Set(['a', 'b']));
  cases.push({ name: '死条目：登记但不再命中 ⇒ 报出', ok: dead.length === 1 && dead[0] === 'zombie' });
  cases.push({ name: '负例：全部命中 ⇒ 无死条目（不得误报）', ok: rot(['a', 'b'], new Set(['a', 'b'])).length === 0 });

  cases.push({ name: '豁免表：pass ⇒ 不豁免（回归不得被掩盖）', ok: !exempts('pass', false) });
  cases.push({ name: '豁免表：缺席 ⇒ 不豁免', ok: !exempts('', false) });
  cases.push({ name: '豁免表：grandfathered + 干净面 ⇒ 豁免', ok: exempts('grandfathered', false) });
  cases.push({ name: '豁免表：grandfathered + 脏面 ⇒ 不豁免（C5 严格化）', ok: !exempts('grandfathered', true) });
  cases.push({ name: '负例：skip + 脏面 ⇒ 不豁免（豁免不得放宽）', ok: !exempts('skip', true) });

  let failed = 0;
  for (const c of cases) {
    process.stdout.write(`${c.ok ? 'PASS' : 'FAIL'}  ${c.name}\n`);
    if (!c.ok) failed += 1;
  }
  process.stdout.write(`\nbaseline self-test: ${cases.length - failed}/${cases.length} passed\n`);
  return failed === 0 ? 0 : 1;
}

if (import.meta.main && process.argv.includes('--self-test')) process.exit(selfTest());
