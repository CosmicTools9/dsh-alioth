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
 * 模式:
 *   默认 / --fail     审计并输出违规，存在违规时退出码 1（阻断 pre-commit）
 *   --report         输出表→内核索引（供 AppAgent 生成前探测，不检查）
 *
 * 用法:
 *   bun scripts/check/check-shared-kernel.ts [ROOT] [--report]
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

const ROOT = resolve(process.argv.slice(2).find((a) => !a.startsWith('--')) || '.');
const REPORT_ONLY = process.argv.includes('--report');

const FRAMEWORK_DIR = join(ROOT, 'Framework/backend');
const PREPROC_DIR = join(ROOT, 'Pre-Proc');

/**
 * 共享表豁免表（allowlist）——同物理表多业务视图（dk 坐标区分，ALIOTH_ONTOLOGY_SPEC §4.3）。
 * 键: "isahl.<table>@<ns>/<service>"；值: 理由。
 */
const SHARED_TABLE_ALLOWLIST: Record<string, string> = {
  'isahl.zc_id_process@AVIC-CAASEC/monitor':
    'GateTemplate 与 approval ApprovalFlow 共享 zc_id_process（门禁模板视图），dk 坐标 JE/FUA/↓_NA 区分（§4.3）',
};



/** 提取 Rust 源码中 AliothDbEntity 实现的 table_name 字面量（isahl.xxx） */
function extractTables(file: string): string[] {
  const src = readFileSync(file, 'utf8');
  const out: string[] = [];
  // 支持两种字面量格式：
  //   fn table_name() -> &'static str { "isahl.zc_id_xxx" }
  //   fn table_name() -> &'static str { "\"isahl\".\"zc_id_xxx\"" }
  const re = /fn\s+table_name\s*\(\s*\)\s*->\s*&'static\s+str\s*\{\s*"(?:\\"isahl\\"\s*\.\s*\\"([a-zA-Z0-9_"-]+)\\"|isahl\.([a-zA-Z0-9_"-]+))"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const table = m[1] ?? m[2];
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

// ── 1. 表→内核映射 ──────────────────────────────────────────────
const kernelMap = new Map<string, string>(); // isahl.table -> crate
if (existsSync(FRAMEWORK_DIR)) {
  for (const crate of readdirSync(FRAMEWORK_DIR, { withFileTypes: true })) {
    if (!crate.isDirectory()) continue;
    const crateDir = join(FRAMEWORK_DIR, crate.name);
    const srcDir = join(crateDir, 'src');
    if (!existsSync(srcDir)) continue;
    for (const file of walkRs(srcDir)) {
      for (const table of extractTables(file)) {
        if (!kernelMap.has(table)) kernelMap.set(table, crate.name);
      }
    }
  }
}

if (REPORT_ONLY) {
  const rows = [...kernelMap.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  for (const [table, crate] of rows) {
    console.log(`${table}\t${crate}`);
  }
  console.log(`# ${rows.length} isahl 表由共享内核持有（Framework/backend）`);
  process.exit(0);
}

// ── 2. ns 壳重复实现检测 ─────────────────────────────────────────
const violations: string[] = [];
let nsLocalTables = 0;

if (existsSync(PREPROC_DIR)) {
  for (const ns of readdirSync(PREPROC_DIR, { withFileTypes: true })) {
    if (!ns.isDirectory()) continue;
    const servicesDir = join(PREPROC_DIR, ns.name, 'Sources/Services');
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
            violations.push(
              `isahl 表 ${table} 已有共享内核实现（Framework/backend/${owner}），` +
                `但 ${ns.name}/Sources/Services/${svc.name} 仍含本地实现：${file}` +
                `（若为共享表多视图，请登记 SHARED_TABLE_ALLOWLIST["${allowKey}"]）`,
            );
          }
        }
      }
    }
  }
}

if (violations.length > 0) {
  console.error(`❌ check-shared-kernel: ${violations.length} 处 ns 本地实现与共享内核重复`);
  for (const v of violations) console.error(`  - ${v}`);
  console.error(`\n修复: 将本地实现上移对应内核 crate（或删除），ns 壳仅保留 scope + 契约投影。`);
  process.exit(1);
}

console.log(
  `✅ check-shared-kernel: ${kernelMap.size} 个 isahl 表由共享内核持有，` +
    `扫描 ${nsLocalTables} 处 ns 本地表定义，零重复实现。`,
);
process.exit(0);
