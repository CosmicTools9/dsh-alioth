#!/usr/bin/env bun
/**
 * check-shared-kernel.ts — isahl 全局表实现单一性审计（共享内核 vs ns 壳）
 *
 * 规约来源:
 *   - openspec/specs/service-ownership-single-implementation（ns-shell-only 语义）
 *   - 用户裁定（2026-08-19）：基于 namespace 差异的数据结构/语义特化都是错的；
 *     isahl 表实现必须单一持有于 Framework 共享内核，ns 壳只允许路由前缀 + 契约投影
 *
 * 检查内容:
 *   1. 建立表→内核映射：扫描 Framework/backend 各 crate 的 src 中 AliothDbEntity::table_name()
 *      返回的 isahl.* 字面量 → 所属 crate
 *   2. ns 壳重复实现检测：扫描 Pre-Proc/{ns}/Sources/Services/{svc}/backend/src/ 本地
 *      models/handlers/repositories 文件（排除依赖 crate 重导出）中定义的 isahl.* 表——
 *      若该表已存在于内核映射 → 违规（ns 本地平行实现）
 *
 * 判定: ns 本地实现的表 MUST NOT 出现在内核映射中。
 *
 * 用法: bun scripts/check/check-shared-kernel.ts [--report] [ROOT]
 *
 * 迁移（change `deepen-check-substrate` C1 + 报告缝，2026-10-05）：判定面（内核映射 + ns 壳扫描）与
 * 输出面分离，且**单次遍历**把映射规模/ns 本地表计数经状态函数传入报告面；`--report` 由**检查自有旗标**
 * 声明后经 `ctx.flags` 消费（旧读 `process.argv`），输出 `table\tcrate`（按表名排序）+ `# N …` 行；
 * 违规块（stderr `❌ …` + `  - …` + 空行 + 修复行）与通过行逐字保留；`enforcement: 'always'`
 * （旧：有违规无条件 `exit(1)`）。取根沿用旧语义（位置参数经 `resolve` 优先，回退基座仓库根）。
 * 已知差异：`--self-test` 由基座统一承载（旧无自测 ⇒ 本迁移新增）。
 */
import { existsSync, readFileSync, readdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { gatewaySourcesKindDir } from '../lib/preproc-layout.mjs';
import { defineCheck, type CheckContext, type Violation } from './lib/check-interface';

/**
 * 共享表豁免表（allowlist）——同物理表多业务视图（dk 坐标区分，ALIOTH_ONTOLOGY_SPEC §4.3）。
 * 键: "isahl.<table>@<ns>/<service>"；值: 理由。
 */
const SHARED_TABLE_ALLOWLIST: Record<string, string> = {
  'isahl.zc_id_process@AVIC-CAASEC/monitor':
    'GateTemplate 与 approval ApprovalFlow 共享 zc_id_process（门禁模板视图），dk 坐标 JE/FUA/↓_NA 区分（§4.3）',
};

/** 提取 Rust 源码中 AliothDbEntity 实现的 table_name 字面量（isahl.xxx） */
export function extractTables(file: string): string[] {
  const src = readFileSync(file, 'utf8');
  const out: string[] = [];
  // 支持两种字面量格式：
  //   fn table_name() -> &'static str { "isahl.zc_id_xxx" }
  //   fn table_name() -> &'static str { "\"isahl\".\"zc_id_xxx\"" }
  const re = /fn\s+table_name\s*\(\s*\)\s*->\s*&'static\s+str\s*\{\s*"(?:\\"isahl\\"\s*\.\s*\\"([a-zA-Z0-9_"-]+)\\"|isahl\.([a-zA-Z0-9_"-]+))"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const table = m[1] ?? m[2]!;
    out.push(`isahl.${table}`);
  }
  return out;
}

function walkRs(dir: string, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'tests' || entry.name === 'target' || entry.name === 'bin') continue;
      walkRs(p, out);
    } else if (entry.name.endsWith('.rs')) {
      out.push(p);
    }
  }
  return out;
}

/** 判定面（单次遍历）：内核映射 + ns 壳违规 + ns 本地表计数。 */
export function sharedKernelState(ctx: CheckContext): {
  kernelMap: Map<string, string>;
  violations: Violation[];
  nsLocalTables: number;
} {
  const root = resolve(ctx.argv[0] ?? ctx.root);
  const FRAMEWORK_DIR = join(root, 'Framework/backend');
  const PREPROC_DIR = join(root, 'Pre-Proc');

  // ── 1. 表→内核映射 ──────────────────────────────────────────────
  const kernelMap = new Map<string, string>(); // isahl.table -> crate
  if (existsSync(FRAMEWORK_DIR)) {
    for (const crate of readdirSync(FRAMEWORK_DIR, { withFileTypes: true })) {
      if (!crate.isDirectory()) continue;
      const srcDir = join(FRAMEWORK_DIR, crate.name, 'src');
      if (!existsSync(srcDir)) continue;
      for (const file of walkRs(srcDir)) {
        for (const table of extractTables(file)) {
          if (!kernelMap.has(table)) kernelMap.set(table, crate.name);
        }
      }
    }
  }

  // ── 2. ns 壳重复实现检测 ─────────────────────────────────────────
  const violations: Violation[] = [];
  let nsLocalTables = 0;

  if (existsSync(PREPROC_DIR)) {
    for (const ns of readdirSync(PREPROC_DIR, { withFileTypes: true })) {
      if (!ns.isDirectory()) continue;
      const servicesDir = gatewaySourcesKindDir(root, ns.name, 'Services');
      if (!existsSync(servicesDir)) continue;
      for (const svc of readdirSync(servicesDir, { withFileTypes: true })) {
        if (!svc.isDirectory()) continue;
        const srcDir = join(servicesDir, svc.name, 'backend/src');
        if (!existsSync(srcDir)) continue;
        for (const file of walkRs(srcDir)) {
          for (const table of extractTables(file)) {
            nsLocalTables += 1;
            const owner = kernelMap.get(table);
            if (owner) {
              const allowKey = `${table}@${ns.name}/${svc.name}`;
              if (SHARED_TABLE_ALLOWLIST[allowKey]) continue;
              const msg =
                `isahl 表 ${table} 已有共享内核实现（Framework/backend/${owner}），` +
                `但 ${ns.name}/Sources/Services/${svc.name} 仍含本地实现：${file}` +
                `（若为共享表多视图，请登记 SHARED_TABLE_ALLOWLIST["${allowKey}"]）`;
              violations.push({ path: 'ns-shell', message: msg, detail: msg });
            }
          }
        }
      }
    }
  }
  return { kernelMap, violations, nsLocalTables };
}

const check = defineCheck({
  id: 'shared-kernel',
  label: 'isahl 表实现单一性（共享内核）',
  enforcement: 'always', // 旧：有违规无条件 exit(1)
  flags: ['--report'],
  predicate: (ctx) => sharedKernelState(ctx).violations,
  report(ctx: CheckContext, { violations }, io) {
    const { kernelMap, nsLocalTables } = sharedKernelState(ctx);
    if (ctx.flags.has('--report')) {
      const rows = [...kernelMap.entries()].sort((a, b) => a[0].localeCompare(b[0]));
      for (const [table, crate] of rows) io.out(`${table}\t${crate}\n`);
      io.out(`# ${rows.length} isahl 表由共享内核持有（Framework/backend）\n`);
      return 0;
    }
    if (violations.length > 0) {
      io.err(`❌ check-shared-kernel: ${violations.length} 处 ns 本地实现与共享内核重复\n`);
      for (const v of violations) io.err(`  - ${v.detail}\n`);
      io.err('\n修复: 将本地实现上移对应内核 crate（或删除），ns 壳仅保留 scope + 契约投影。\n');
      return 1;
    }
    io.out(
      `✅ check-shared-kernel: ${kernelMap.size} 个 isahl 表由共享内核持有，` +
        `扫描 ${nsLocalTables} 处 ns 本地表定义，零重复实现。\n`,
    );
    return 0;
  },
  selfTest() {
    let failed = 0;
    // 提取器（纯文本面）：两种字面量形态 + 非 table_name 面不命中
    const dir = mkdtempSync(join(tmpdir(), 'shared-kernel-selftest-'));
    try {
      const cases: Array<[string, string, string[]]> = [
        ['plain.rs', 'fn table_name() -> &\'static str { "isahl.zc_id_message" }\n', ['isahl.zc_id_message']],
        ['quoted.rs', 'fn table_name() -> &\'static str { "\\"isahl\\".\\"zc_id_msgs-system\\"" }\n', ['isahl.zc_id_msgs-system']],
        ['none.rs', 'fn other() -> &\'static str { "isahl.zc_id_lifecycle_r_primary-status" }\n', []],
      ];
      for (const [name, src, expect] of cases) {
        const p = join(dir, name);
        writeFileSync(p, src);
        const got = extractTables(p);
        if (JSON.stringify(got) !== JSON.stringify(expect)) {
          failed += 1;
          console.error(`shared-kernel selfTest: ${name} 期望 ${JSON.stringify(expect)} 实得 ${JSON.stringify(got)}`);
        }
      }
      // 报告面排序口径（纯）：表名字典序
      const rows = [['isahl.b', 'c1'], ['isahl.a', 'c2']] as Array<[string, string]>;
      const sorted = [...rows].sort((a, b) => a[0].localeCompare(b[0])).map((r) => r[0]);
      if (sorted.join(',') !== 'isahl.a,isahl.b') {
        failed += 1;
        console.error('shared-kernel selfTest: 排序口径错误');
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
    return failed === 0 ? 0 : 1;
  },
});

if (import.meta.main) process.exit(await check.main());
