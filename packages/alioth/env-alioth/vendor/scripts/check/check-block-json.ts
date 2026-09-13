#!/usr/bin/env bun
/**
 * check-block-json.ts — block.json 形状与引用校验（block-schema capability 的执行基板）
 *
 * 两档：
 *   - 基线档（默认；pre-commit）：违规与 `scripts/check/baselines/block-json-baseline.json` 比对，
 *     **只阻断基线之外的新增违规**（存量 509 块的键集/引用欠债可平滑接入）。
 *   - 严格档（`--strict`；全量判定）/ 单文件档（`--only <block.json>`；AppAgent 产物门禁）。
 *
 * 规则：
 *   R1 unknown-key      键不在 canonical 键集内（组合键 module/moduleId/order/group/entry/routePrefix 属此类，
 *                       组合关系 MUST 由 module.json#blockAssembly 持有）
 *   R2 missing-required 缺 REQUIRED 键（id/name/version/services/coordinates/aliothVersion/sharing）
 *   R3 ns-mismatch      `namespace` 存在但与路径不一致
 *   R4 dangling-service `services[]` 元素既非同 ns 服务目录，也非该 ns 约定的 factor 码
 *   R5 bad-coordinate   `coordinates.*.code` 为空/占位符/不在 DB 维度表（DB 不可达 → 该维度显式跳过并打印）
 *   R6 dangling-owner   `sharing.ownerModule` 指向不存在的 `<ns>/<moduleId>`
 *   R7 bad-sharing      `mode` 非法 / `shared` 时 consumers < 2 或元素不可解析
 *   R8 missing-prototype-version  同 ns `Prototypes/` 下该块目录存在原型痕迹（`b-v*.html` 或 `llm-tsx`）
 *                       时缺 `prototypeVersion`（无原型痕迹则 MAY 省略）
 *
 * Usage: bun scripts/check/check-block-json.ts [--ns NS] [--strict] [--only <block.json>] [--update-baseline]
 * Exit: 0 通过（基线档：无新增违规）; 1 存在新增违规（strict/only：任何违规）; 2 用法/IO 错误
 */
import { readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { isRecord } from "../lib/type-guards";
import {
  dbDimensionCodes, derivePrototypeVersion, fingerprint, hasPrototypeTraces, listNamespaces, loadBaseline, relPath,
  summarize, unitFiles, unitIds, writeBaseline, type Violation,
} from "../lib/preproc-artifacts";

const ROOT = resolve(import.meta.dirname, "../..");
const BASELINE_PATH = join(ROOT, "scripts/check/baselines/block-json-baseline.json");

/** canonical 键集：block.json 只描述块自身语义 + 文档化扩展（BLOCK_SCHEMA §1.1/§1.2/§1.3） */
const CANONICAL_KEYS: Record<string, true> = {
  id: true, block: true, name: true, version: true, prototypeVersion: true, services: true,
  coordinates: true, aliothVersion: true, sharing: true,
  namespace: true, flows: true, workbenchPosts: true,
  status: true, icon: true, navIcon: true, ontology: true, notes: true,
  factors: true, blockType: true, description: true,
};

const REQUIRED_KEYS = [
  "id", "name", "version", "services", "coordinates", "aliothVersion", "sharing",
];

/** 组合键：属 module.json#blockAssembly，出现即 R1（错误信息里点名） */
const COMPOSITION_KEYS: Record<string, true> = {
  module: true, moduleId: true, order: true, group: true, entry: true, routePrefix: true,
};

const PLACEHOLDER_CODES: Record<string, true> = {
  "↓_XX": true, "↑_XX": true, "!_XX": true, "↓__XX": true, "↑__XX": true, XX: true,
};

interface Ctx {
  services: Set<string>;
  modules: Set<string>;
  scenes: Set<string>;
  factors: Set<string>;
  functions: Set<string>;
  factorIsServiceAlias: boolean;
}

function parseArgs(argv: string[]): Map<string, string | true> {
  const out = new Map<string, string | true>();
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    if (i + 1 < argv.length && !argv[i + 1].startsWith("--")) { out.set(a.slice(2), argv[i + 1]); i++; }
    else out.set(a.slice(2), true);
  }
  return out;
}

function contextFor(ns: string, dims: { scenes: Set<string>; factors: Set<string>; functions: Set<string> }): Ctx {
  return {
    services: unitIds(ROOT, ns, "Services"),
    modules: unitIds(ROOT, ns, "Modules"),
    scenes: dims.scenes, factors: dims.factors, functions: dims.functions,
    // WZ 口径：block.services[] 可为 factor 码（BLOCK_SCHEMA §1.1 `services` 说明）
    factorIsServiceAlias: ns === "WZ",
  };
}

function checkBlock(ns: string, file: string, ctx: Ctx, out: Violation[]): void {
  const rel = relPath(ROOT, file);
  let data: unknown;
  try { data = JSON.parse(readFileSync(file, "utf-8")); }
  catch (e) { out.push({ ns, file: rel, rule: "R2", detail: `JSON 解析失败: ${e}` }); return; }
  if (!isRecord(data)) { out.push({ ns, file: rel, rule: "R2", detail: "顶层非对象" }); return; }

  // R1 unknown-key（组合键单独点名）
  for (const key of Object.keys(data)) {
    if (CANONICAL_KEYS[key]) continue;
    const hint = COMPOSITION_KEYS[key]
      ? "（组合键 —— 从属/顺序/分组 MUST 由 module.json#blockAssembly.blocks[] 声明）"
      : "";
    out.push({ ns, file: rel, rule: "R1", detail: `未知键 '${key}'${hint}` });
  }

  // R2 missing-required
  for (const key of REQUIRED_KEYS) {
    if (!(key in data)) out.push({ ns, file: rel, rule: "R2", detail: `缺 REQUIRED 键 '${key}'` });
  }

  // R8 prototype-version-missing：有原型痕迹（b-v*.html / llm-tsx）时 prototypeVersion 必填
  if (!("prototypeVersion" in data)) {
    const blockId = basename(dirname(file));
    if (hasPrototypeTraces(ROOT, ns, blockId)) {
      out.push({
        ns,
        file: rel,
        rule: "R8",
        detail: `Prototypes/ 下存在原型痕迹（b-v*.html 或 llm-tsx）但缺 prototypeVersion（推定为 '${derivePrototypeVersion(ROOT, ns, blockId)}'）`,
      });
    }
  }

  // R3 namespace 路径一致
  const nsField = data.namespace;
  if (typeof nsField === "string" && nsField !== ns) {
    out.push({ ns, file: rel, rule: "R3", detail: `namespace='${nsField}' 与路径 ns='${ns}' 不一致` });
  }

  // R4 services 解析（空数组合规：BLOCK_SCHEMA/check-config-json 明示 services 可为空数组）
  const services = data.services;
  if (Array.isArray(services)) {
    for (const s of services) {
      if (typeof s !== "string" || !s.trim()) continue;
      const id = s.trim();
      if (ctx.services.has(id)) continue;
      if (ctx.factorIsServiceAlias && ctx.factors.has(id)) continue;
      out.push({ ns, file: rel, rule: "R4", detail: `services[] 元素 '${id}' 非本 ns 服务目录，也不是该 ns 约定的 factor 码` });
    }
  }

  // R5 坐标码
  const coords = data.coordinates;
  if (isRecord(coords)) {
    const dims: Array<[string, Set<string>]> = [["scene", ctx.scenes], ["factor", ctx.factors], ["function", ctx.functions]];
    for (const [dim, valid] of dims) {
      const dimObj = coords[dim];
      if (!isRecord(dimObj)) continue;
      const code = typeof dimObj.code === "string" ? dimObj.code.trim() : "";
      if (!code) { out.push({ ns, file: rel, rule: "R5", detail: `coordinates.${dim}.code 为空` }); continue; }
      if (PLACEHOLDER_CODES[code]) { out.push({ ns, file: rel, rule: "R5", detail: `coordinates.${dim}.code='${code}' 为占位符` }); continue; }
      if (valid.size > 0 && !valid.has(code)) {
        out.push({ ns, file: rel, rule: "R5", detail: `coordinates.${dim}.code='${code}' 不在 isahl.zc_id_${dim}` });
      }
    }
  }

  // R6 sharing.ownerModule + R7 可复用声明（mode/consumers）
  const sharing = data.sharing;
  if (isRecord(sharing)) {
    if (typeof sharing.ownerModule === "string" && sharing.ownerModule.trim()) {
      const om = sharing.ownerModule.trim();
      const [mNs, mId] = om.split("/");
      if (!mNs || !mId) out.push({ ns, file: rel, rule: "R6", detail: `ownerModule='${om}' 非法（期望 <ns>/<moduleId>）` });
      else if (mNs !== ns) out.push({ ns, file: rel, rule: "R6", detail: `ownerModule 跨 ns（'${om}' vs 文件 ns '${ns}'）` });
      else if (!ctx.modules.has(mId)) out.push({ ns, file: rel, rule: "R6", detail: `ownerModule='${om}' 模块目录不存在` });
    }

    const mode = typeof sharing.mode === "string" ? sharing.mode.trim() : "";
    if (mode && mode !== "single" && mode !== "shared") {
      out.push({ ns, file: rel, rule: "R7", detail: `sharing.mode='${mode}' 非法（single|shared）` });
    }
    const consumers = sharing.consumers;
    if (Array.isArray(consumers)) {
      if (mode === "shared" && consumers.length < 2) {
        out.push({ ns, file: rel, rule: "R7", detail: `sharing.mode='shared' 时 consumers MUST ≥ 2（当前 ${consumers.length}）—— 否则该块非可复用单元` });
      }
      for (const c of consumers) {
        if (typeof c !== "string" || !c.trim()) continue;
        const [cNs, cId] = c.trim().split("/");
        if (!cNs || !cId) {
          out.push({ ns, file: rel, rule: "R7", detail: `consumers 元素 '${c}' 非法（期望 <ns>/<moduleId>）` });
          continue;
        }
        if (cNs !== ns) {
          out.push({ ns, file: rel, rule: "R7", detail: `consumers 元素 '${c}' 跨 ns（文件 ns='${ns}'）` });
          continue;
        }
        if (!ctx.modules.has(cId)) {
          out.push({ ns, file: rel, rule: "R7", detail: `consumers 元素 '${c}' 模块目录不存在` });
        }
      }
    } else if (mode === "shared") {
      out.push({ ns, file: rel, rule: "R7", detail: "sharing.mode='shared' 时 MUST 声明 consumers[]（≥ 2）" });
    }
  }
}

function main(): void {
  const args = parseArgs(process.argv.slice(2));
  const strict = args.has("strict");
  const update = args.has("update-baseline");
  const only = typeof args.get("only") === "string" ? resolve(ROOT, args.get("only") as string) : null;
  const nsFilter = typeof args.get("ns") === "string" ? (args.get("ns") as string) : null;

  const dims = {
    scenes: dbDimensionCodes(ROOT, "scene"),
    factors: dbDimensionCodes(ROOT, "factor"),
    functions: dbDimensionCodes(ROOT, "function"),
  };
  if (dims.scenes.size === 0 || dims.factors.size === 0 || dims.functions.size === 0) {
    console.log("  ⚠️  DB 维度码不可得 —— 坐标维度（R5 的 DB 存在性）本次跳过（不静默：本条即声明）");
  }

  // 单文件档：AppAgent 产物门禁（严格判失败，不比对基线）
  if (only) {
    const ns = only.split("/Pre-Proc/")[1]?.split("/")[0] ?? "";
    const violations: Violation[] = [];
    checkBlock(ns, only, contextFor(ns, dims), violations);
    console.log(`\n📋 block.json 校验 — ${relPath(ROOT, only)}（单文件/严格档）`);
    console.log(`   违规统计: ${summarize(violations)}`);
    for (const v of violations.slice(0, 20)) console.log(`   [${v.rule}] ${v.detail}`);
    console.log(violations.length === 0 ? "\n✅ block.json 校验通过" : "\n❌ block.json 校验未通过");
    process.exit(violations.length === 0 ? 0 : 1);
  }

  const violations: Violation[] = [];
  let files = 0;
  for (const ns of listNamespaces(ROOT)) {
    if (nsFilter && ns !== nsFilter) continue;
    const ctx = contextFor(ns, dims);
    for (const f of unitFiles(ROOT, ns, "Blocks", "block.json")) { files++; checkBlock(ns, f, ctx, violations); }
  }

  const baseline = loadBaseline(BASELINE_PATH);
  const fresh = violations.filter((v) => !baseline.entries.has(fingerprint(v)));

  console.log(`\n📋 block.json 校验 — ${files} 个文件（${nsFilter ?? "全 ns"}，${strict ? "严格档" : "基线档"}）`);
  console.log(`   违规统计: ${summarize(violations)}`);
  if (baseline.entries.size > 0) console.log(`   基线已登记: ${baseline.entries.size} 条；本次新增: ${fresh.length} 条`);
  else console.log(`   （基线不存在：执行 --update-baseline 生成）`);

  const show = strict ? violations : fresh;
  for (const v of show.slice(0, 15)) console.log(`   [${v.rule}] ${v.file}: ${v.detail}`);
  if (show.length > 15) console.log(`   … 其余 ${show.length - 15} 条省略`);

  if (update) {
    writeBaseline(BASELINE_PATH, violations, "block.json 形状/引用存量欠债基线（check-block-json.ts）；新增违规不得落入此文件");
    console.log(`\n✅ 基线已写入 ${relPath(ROOT, BASELINE_PATH)}（${violations.length} 条）`);
    process.exit(0);
  }

  const failed = strict ? violations.length > 0 : fresh.length > 0;
  console.log(failed ? "\n❌ block.json 校验未通过" : "\n✅ block.json 校验通过");
  process.exit(failed ? 1 : 0);
}

main();
