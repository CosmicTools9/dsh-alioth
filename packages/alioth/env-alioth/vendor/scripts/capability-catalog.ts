#!/usr/bin/env bun
/**
 * capability-catalog.ts — 构建能力清单（只读投影）
 *
 * 定位：**投影而非判据**。本命令把 Pre-Proc/{ns}/Sources/Apps/** 的既有声明收敛为
 * 一份机器可读的能力清单，供技能在**生成前**收窄选择面；权威门禁仍是
 * check-composition.ts / check-block-json.ts 等既有检查——本命令不替代、也不得与
 * 它们给出相反结论（同一 loader：scripts/lib/preproc-artifacts.ts）。
 *
 * 与既有「能力发现」表格的差别：表格只有 LLM 自己读、与门禁不同源；本命令的清单
 * 可被脚本、门禁与 AppAgent 步骤共同消费，且枚举面取自 ns 的 JSON Schema（不臆造）。
 *
 * Usage:
 *   bun scripts/capability-catalog.ts <ns> [--module <id>] [--block <id>]   # 清单（stdout JSON）
 *   bun scripts/capability-catalog.ts <ns> --check <file.json>...           # 引用 ⊆ 清单 校验
 * Exit: 0 成功/无违规; 1 存在违规; 2 用法或 IO 错误
 *
 * 只读：MUST NOT 写入 Pre-Proc/**（清单权威形态 = stdout JSON）。
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { isRecord } from "./lib/type-guards";
import {
  dbDimensionCodes,
  isArtifactDir,
  listNamespaces,
  unitFiles,
  type Violation,
} from "./lib/preproc-artifacts";

export const REPO_ROOT = join(import.meta.dirname, "..");

/** 清单违规规则码 → 与既有门禁的对应关系（同 loader ⇒ 同结论） */
export const RULE_HINT: Record<string, string> = {
  CC1: "dangling-block：blockAssembly.blocks[].id 不在清单（check-composition C1 同结论）",
  CC2: "dangling-service：services[] 元素不可解析（check-composition C7 同结论）",
  CC3: "dangling-module：sharing.ownerModule / consumers[] 不在清单（check-composition C2 同结论）",
  CC9: "unparsable：目标非合法 JSON",
};

export interface CatalogBlock {
  id: string;
  name?: string;
  description?: string;
  blockType?: string;
  status?: string;
  prototypeVersion?: string;
  /** 是否存在 `llm-tsx` 原型源（可复用性的最低判据） */
  built: boolean;
  sharing?: { mode?: string; ownerModule?: string; consumers?: string[] };
  services: string[];
}

export interface CatalogService {
  id: string;
  version?: string;
  domain?: string;
  layer?: string;
}

export interface CatalogModule {
  id: string;
  name?: string;
  category?: string;
  status?: string;
  /** blockAssembly.blocks[].id（权威组合声明顺序） */
  blocks: string[];
}

export interface CatalogEnums {
  blockSharingMode?: string[];
  blockStatus?: string[];
  moduleCategory?: string[];
  moduleStatus?: string[];
  appStatus?: string[];
}

export interface CapabilityCatalog {
  namespace: string;
  scope: { module?: string; block?: string };
  blocks: CatalogBlock[];
  services: CatalogService[];
  modules: CatalogModule[];
  enums: CatalogEnums;
  /** 已解析的引用规则（供技能解释「为什么这个 id 合法」） */
  referenceRules: Record<string, string>;
  /** 无法判定的面：显式登记，不静默丢弃 */
  unresolved: string[];
}

function readJson(file: string): Record<string, unknown> | null {
  try {
    const v: unknown = JSON.parse(readFileSync(file, "utf-8"));
    return isRecord(v) ? v : null;
  } catch {
    return null;
  }
}

/** 取非空字符串（10+ 调用点须锁步：统一 trim 与空串语义） */
function str(v: unknown): string | undefined {
  return typeof v === "string" && v.trim() ? v.trim() : undefined;
}

/** 取非空字符串数组（5+ 调用点须锁步：统一过滤非字符串与空串） */
function strArray(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && !!x.trim()) : [];
}

function enumAt(root: string, ns: string, file: string, path: string[]): string[] | undefined {
  const j = readJson(join(root, "Pre-Proc", ns, "_schema", file));
  if (!j) return undefined;
  let cur: unknown = j;
  for (const k of path) {
    if (!isRecord(cur)) return undefined;
    cur = cur[k];
  }
  return Array.isArray(cur) ? cur.filter((x): x is string => typeof x === "string") : undefined;
}

function loadBlocks(root: string, ns: string): CatalogBlock[] {
  const out: CatalogBlock[] = [];
  for (const f of unitFiles(root, ns, "Blocks", "block.json")) {
    const id = basename(dirname(f));
    const j = readJson(f);
    if (!j) continue;
    const sharing = isRecord(j.sharing) ? j.sharing : undefined;
    out.push({
      id,
      name: str(j.name),
      description: str(j.description),
      blockType: str(j.blockType),
      status: str(j.status),
      prototypeVersion: str(j.prototypeVersion),
      built: existsSync(join(root, "Pre-Proc", ns, "Prototypes", "Blocks", id, "llm-tsx")),
      sharing: sharing
        ? {
            mode: str(sharing.mode),
            ownerModule: str(sharing.ownerModule),
            consumers: strArray(sharing.consumers),
          }
        : undefined,
      services: strArray(j.services),
    });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

function loadServices(root: string, ns: string): CatalogService[] {
  const appsRoot = join(root, "Pre-Proc", ns, "Sources", "Apps", "Services");
  const servicesRoot = existsSync(appsRoot) ? appsRoot : join(root, "Pre-Proc", ns, "Sources", "Services");
  if (!existsSync(servicesRoot)) return [];
  const out: CatalogService[] = [];
  for (const e of readdirSync(servicesRoot, { withFileTypes: true })) {
    if (!e.isDirectory() || !isArtifactDir(e.name)) continue;
    const j = readJson(join(servicesRoot, e.name, "service.json"));
    out.push({
      id: e.name,
      version: j ? str(j.version) : undefined,
      domain: j ? str(j.domain) : undefined,
      layer: j ? str(j.layer) : undefined,
    });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

function loadModules(root: string, ns: string): CatalogModule[] {
  const out: CatalogModule[] = [];
  for (const f of unitFiles(root, ns, "Modules", "module.json")) {
    const j = readJson(f);
    if (!j) continue;
    const asm = isRecord(j.blockAssembly) ? j.blockAssembly : undefined;
    // legacy 数组形态（check-composition C4 判定为违规）：此处只做可读提取，不重复判定
    const raw = Array.isArray(asm) ? asm : asm && Array.isArray(asm.blocks) ? asm.blocks : [];
    out.push({
      id: basename(dirname(f)),
      name: str(j.name),
      category: str(j.category),
      status: str(j.status),
      blocks: raw
        .map((b) => (isRecord(b) ? str(b.id) : str(b)))
        .filter((x): x is string => !!x),
    });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** 作用域收窄：module / block 视角下的可用能力子集 */
function narrow(
  full: CapabilityCatalog,
  scope: { module?: string; block?: string },
): Pick<CapabilityCatalog, "blocks" | "services" | "modules"> {
  if (!scope.module && !scope.block) {
    return { blocks: full.blocks, services: full.services, modules: full.modules };
  }
  const byId: Record<string, CatalogBlock> = {};
  for (const b of full.blocks) byId[b.id] = b;
  const modById: Record<string, CatalogModule> = {};
  for (const m of full.modules) modById[m.id] = m;
  const moduleIds = new Set<string>();
  const blockIds = new Set<string>();

  if (scope.block) {
    const b = byId[scope.block];
    if (b) blockIds.add(b.id);
    if (b?.sharing?.ownerModule) moduleIds.add(b.sharing.ownerModule.split("/").pop() as string);
    for (const c of b?.sharing?.consumers ?? []) moduleIds.add(c.split("/").pop() as string);
  } else {
    moduleIds.add(scope.module as string);
    for (const id of modById[scope.module as string]?.blocks ?? []) blockIds.add(id);
    for (const b of full.blocks) {
      // 归属该 module 的 block + 声明消费该 module 的 shared block
      if (b.sharing?.ownerModule?.endsWith(`/${scope.module}`)) blockIds.add(b.id);
      if ((b.sharing?.consumers ?? []).some((c) => c.endsWith(`/${scope.module}`))) blockIds.add(b.id);
    }
  }

  const blocks = full.blocks.filter((b) => blockIds.has(b.id));
  const serviceIds = new Set(blocks.flatMap((b) => b.services));
  for (const id of moduleIds) {
    for (const b of modById[id]?.blocks ?? []) {
      for (const s of byId[b]?.services ?? []) serviceIds.add(s);
    }
  }
  return {
    blocks,
    services: full.services.filter((s) => serviceIds.has(s.id)),
    modules: full.modules.filter((m) => moduleIds.has(m.id)),
  };
}

export function buildCatalog(
  root: string,
  ns: string,
  scope: { module?: string; block?: string },
): CapabilityCatalog {
  const unresolved: string[] = [];
  const full: CapabilityCatalog = {
    namespace: ns,
    scope,
    blocks: loadBlocks(root, ns),
    services: loadServices(root, ns),
    modules: loadModules(root, ns),
    enums: {
      blockSharingMode: enumAt(root, ns, "block.schema.json", ["properties", "sharing", "properties", "mode", "enum"]),
      blockStatus: enumAt(root, ns, "block.schema.json", ["properties", "status", "enum"]),
      moduleCategory: enumAt(root, ns, "module.schema.json", ["properties", "category", "enum"]),
      moduleStatus: enumAt(root, ns, "module.schema.json", ["properties", "status", "enum"]),
      appStatus: enumAt(root, ns, "app.schema.json", ["properties", "status", "enum"]),
    },
    referenceRules: {
      "blockAssembly.blocks[].id": "MUST 命中 blocks[].id",
      "block.services[]": "MUST 命中 services[].id（WZ 另允许本 ns factor 码）",
      "sharing.ownerModule": "MUST 命中 modules[].id（格式 {namespace}/{moduleId}）",
      "sharing.consumers[]": "MUST 命中 modules[].id（mode=shared 时必填）",
    },
    unresolved,
  };
  if (!existsSync(join(root, "Pre-Proc", ns, "_schema"))) {
    unresolved.push(`_schema/ 缺失：${ns} 无 JSON Schema，枚举面不可得`);
  }
  if (scope.module && !full.modules.some((m) => m.id === scope.module)) {
    unresolved.push(`作用域 module '${scope.module}' 不在清单`);
  }
  if (scope.block && !full.blocks.some((b) => b.id === scope.block)) {
    unresolved.push(`作用域 block '${scope.block}' 不在清单`);
  }
  return { ...full, ...narrow(full, scope) };
}

/** 引用 ⊆ 清单 校验（投影级；权威门禁为既有 check-*） */
export function checkTargets(
  root: string,
  catalog: CapabilityCatalog,
  files: string[],
  factors: Set<string>,
): Violation[] {
  const ns = catalog.namespace;
  const blockIds = new Set(catalog.blocks.map((b) => b.id));
  const serviceIds = new Set(catalog.services.map((s) => s.id));
  const moduleIds = new Set(catalog.modules.map((m) => m.id));
  const out: Violation[] = [];

  for (const f of files) {
    const rel = relative(root, f);
    const j = readJson(f);
    if (!j) {
      out.push({ ns, file: rel, rule: "CC9", detail: "JSON 解析失败" });
      continue;
    }
    const asmRaw = isRecord(j.blockAssembly) ? j.blockAssembly : undefined;
    if (asmRaw) {
      const arr = Array.isArray(asmRaw) ? asmRaw : Array.isArray(asmRaw.blocks) ? asmRaw.blocks : [];
      for (const b of arr) {
        const id = isRecord(b) ? str(b.id) : str(b);
        if (id && !blockIds.has(id)) {
          out.push({ ns, file: rel, rule: "CC1", detail: `blockAssembly 引用不在清单的 block '${id}'` });
        }
      }
      const bindings = isRecord(asmRaw.serviceBindings) ? asmRaw.serviceBindings : undefined;
      for (const [blockId, entry] of Object.entries(bindings ?? {})) {
        for (const s of isRecord(entry) ? strArray(entry.services) : []) {
          if (!serviceIds.has(s) && !(ns === "WZ" && factors.has(s))) {
            out.push({
              ns,
              file: rel,
              rule: "CC2",
              detail: `serviceBindings['${blockId}'].services 元素 '${s}' 不在清单`,
            });
          }
        }
      }
    }
    // block.json 形态：以 services 数组 + sharing 面判定（module.json 无 services）
    if (Array.isArray(j.services)) {
      for (const s of strArray(j.services)) {
        if (!serviceIds.has(s) && !(ns === "WZ" && factors.has(s))) {
          out.push({ ns, file: rel, rule: "CC2", detail: `services 元素 '${s}' 不在清单` });
        }
      }
      const sharing = isRecord(j.sharing) ? j.sharing : undefined;
      const owner = sharing ? str(sharing.ownerModule) : undefined;
      if (owner && !moduleIds.has(owner.split("/").pop() as string)) {
        out.push({ ns, file: rel, rule: "CC3", detail: `sharing.ownerModule '${owner}' 不在清单` });
      }
      for (const c of sharing ? strArray(sharing.consumers) : []) {
        if (!moduleIds.has(c.split("/").pop() as string)) {
          out.push({ ns, file: rel, rule: "CC3", detail: `sharing.consumers 元素 '${c}' 不在清单` });
        }
      }
    }
  }
  return out;
}

function usage(namespaces: string[]): never {
  console.error(
    [
      "Usage:",
      "  bun scripts/capability-catalog.ts <ns> [--module <id>] [--block <id>]",
      "  bun scripts/capability-catalog.ts <ns> --check <file.json>...",
      `Namespaces: ${namespaces.join(", ")}`,
    ].join("\n"),
  );
  process.exit(2);
}

function main(): void {
  const argv = process.argv.slice(2);
  const namespaces = listNamespaces(REPO_ROOT);
  const ns = argv[0];
  if (!ns || ns.startsWith("--")) usage(namespaces);
  if (!namespaces.includes(ns)) {
    console.error(`未知 namespace: ${ns}（可选 ${namespaces.join(", ")}）`);
    process.exit(2);
  }

  let module: string | undefined;
  let block: string | undefined;
  const checkFiles: string[] = [];
  for (let i = 1; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--module") module = argv[++i];
    else if (a === "--block") block = argv[++i];
    else if (a === "--check") {
      while (i + 1 < argv.length && !argv[i + 1].startsWith("--")) checkFiles.push(argv[++i]);
    } else usage(namespaces);
  }
  if (module && block) usage(namespaces);

  const catalog = buildCatalog(REPO_ROOT, ns, { module, block });
  if (checkFiles.length === 0) {
    console.log(JSON.stringify(catalog, null, 2));
    return;
  }
  if (module || block) usage(namespaces);

  // factor 码兜底仅 WZ 需要（check-composition C7 同口径）——非 WZ 不触库
  const factors = ns === "WZ" ? dbDimensionCodes(REPO_ROOT, "factor") : new Set<string>();
  if (ns === "WZ" && factors.size === 0) {
    catalog.unresolved.push("factor 码不可得：WZ factor 码回退本次跳过（本条即声明）");
    console.error("  ⚠️  factor 码不可得 —— WZ factor 码回退本次跳过");
  }

  const violations = checkTargets(REPO_ROOT, catalog, checkFiles, factors);
  if (violations.length === 0) {
    console.log(`✅ 引用 ⊆ 清单：${checkFiles.length} 个目标通过（${ns}）`);
    return;
  }
  console.error(`❌ 引用 ∉ 清单：${violations.length} 项`);
  for (const v of violations) {
    console.error(`  [${v.rule}] ${v.file} — ${v.detail}`);
    console.error(`         ${RULE_HINT[v.rule] ?? ""}`);
  }
  process.exit(1);
}

if (import.meta.main) main();
