#!/usr/bin/env bun
/**
 * check-block-json.ts — block.json 形状与引用校验（block-schema capability 的执行基板）
 *
 * 两档：
 *   - 基线档（默认；push 档门禁 `block-json`）：违规与 `scripts/check/baselines/block-json-baseline.json` 比对，
 *     **只阻断基线之外的新增违规**（存量 509 块的键集/引用欠债可平滑接入）。
 *   - 严格档（`--strict`；全量判定）/ 单文件档（`--scope <block.json>`；AppAgent 产物门禁）。
 *
 * 规则：
 *   R1 unknown-key      键不在 canonical 键集内（组合键 module/moduleId/order/group/entry/routePrefix 属此类，
 *                       组合关系 MUST 由 module.json#blockAssembly 持有）
 *   R2 missing-required 缺 REQUIRED 键（id/block/name/version/services/coordinates/aliothVersion/sharing）
 *                       —— 键集与 canonical `scripts/schema/block.schema.json#required` 逐字对齐（运行期断言）
 *   R11 block-code-invalid `block` 存在但非「非空字符串」（空串/对象/数组/数字/布尔的业务码 = 类型不符；
 *                       缺键由 R2 判）
 *   R3 ns-mismatch      `namespace` 存在但与路径不一致
 *   R4 dangling-service `services[]` 元素既非同 ns 服务目录，也非该 ns 约定的 factor 码
 *   R5 bad-coordinate   `coordinates.*.code` 为空/占位符/不在 DB 维度表（DB 不可达 → 该维度显式跳过并打印）
 *   R6 dangling-owner   `sharing.ownerModule` 指向不存在的 `<ns>/<moduleId>`
 *   R10 design-leg-missing `Sources/Apps/Blocks/{id}/block.json` 存在 ⇒ `Prototypes/Blocks/{id}` MUST 有
 *                       `b-v{N}.html` 且构建源 `llm-tsx/block.tsx` 非桩（<200B 恒 null 视为桩；
 *                       block-schema capability `block-design-leg-presence`——存量违例落基线并须
 *                       在 BLOCK_READY_UNWIRED_REGISTRY 有镜像条目）
 *
 * Usage: bun scripts/check/check-block-json.ts [--ns NS] [--strict] [--scope <block.json|blockId>] [--rebuild-baseline]
 * Exit: 0 通过（基线档：无新增违规）; 1 存在新增违规（strict/only：任何违规）; 2 用法/IO 错误或维度码不可得（驳回·人在回路）
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { defineCheck } from "./lib/check-interface";

const META_CHANNEL = "__meta";

class ExitSignal {
  constructor(readonly rc: number) {}
}
let SINK: { out: (l: string) => void; err: (l: string) => void } = {
  out: (l) => process.stdout.write(`${l}\n`),
  err: (l) => process.stderr.write(`${l}\n`),
};
const exit: (rc: number) => never = (rc) => {
  throw new ExitSignal(rc);
};
/** 判定面注入（旧自解析 argv / parseArgs 移除；由基座 ctx 供给） */
let STRICT = false;
/** 基线重建旗标：基座已占 `--rebuild-baseline` ⇒ 本检查自有旗标改名（否则被基座静默吞掉） */
let REBUILD_BASELINE = false;
let NS_FILTER_ARG: string | null = null;
let ONLY_ARG: string | null = null;
import { basename, dirname, join, resolve } from "node:path";
import { isRecord } from "../lib/type-guards";
import { readSchema, schemaNode } from "../schema";
import { parseBigIntSafe, rawIntegerText } from "../lib/json-bigint-safe";
import { callerRepoRoot } from "../lib/caller-root";
import {
  nsFaceSkipNotice,
  nsFaceStatus,
  nsSourcesTracked,
} from "../lib/ns-surface";
import {
  DimensionCodesUnavailableError,
  dbDimensionCodes,
  derivePrototypeVersion,
  fingerprint,
  hasPrototypeTraces,
  listNamespaces,
  loadBaseline,
  relPath,
  summarize,
  unitFiles,
  unitIds,
  writeBaseline,
  type Violation,
} from "../lib/preproc-artifacts";

// 判定面归属（2026-09-25）：ROOT = **调用方仓**（ns 仓内调用判 ns 面；平台仓调用仍落平台仓）。
const ROOT = callerRepoRoot();
const BASELINE_PATH = join(
  ROOT,
  "scripts/check/baselines/block-json-baseline.json",
);

type DimensionInputs = {
  dims: { scenes: Set<string>; factors: Set<string>; functions: Set<string> };
  dimIdOf: (dim: string, code: string) => string | null;
};

/**
 * 维度输入（三族码集 + 维度行 id 解析器）——任一不可得即**驳回至人在回路**
 * （用户裁定 2026-09-23：维度码不能退化；禁以降级跳过 R5/R5b 结论放行）。
 */
export function loadDimensionInputs(): DimensionInputs {
  try {
    return {
      dims: {
        scenes: dbDimensionCodes(ROOT, "scene"),
        factors: dbDimensionCodes(ROOT, "factor"),
        functions: dbDimensionCodes(ROOT, "function"),
      },
      dimIdOf: dbDimensionIdResolver(),
    };
  } catch (e) {
    if (!(e instanceof DimensionCodesUnavailableError)) throw e;
    // 输出经 SINK 记录、由报告面同步写后随 rc 退出（旧实现直接 writeSync(2, …) 是为了绕过
    // `console.error` 的管道异步缓冲 + 紧随其后的 process.exit 丢输出；基座不再有该问题）。
    SINK.err(
      `\n❌ [驳回·人在回路] block.json 校验无法完成：${e.message}\n` +
        "   处置：恢复维度输入（DB 可连、维度表有数据、DATABASE_URL 已设）后重跑；MUST NOT 以降级（跳过 R5/R5b）结论放行。",
    );
    exit(2);
  }
}

/** canonical 键集：block.json 只描述块自身语义 + 文档化扩展（BLOCK_SCHEMA §1.1/§1.2/§1.3） */
const CANONICAL_KEYS: Record<string, true> = {
  id: true,
  block: true,
  name: true,
  version: true,
  prototypeVersion: true,
  services: true,
  coordinates: true,
  aliothVersion: true,
  sharing: true,
  namespace: true,
  flows: true,
  workbenchPosts: true,
  status: true,
  icon: true,
  navIcon: true,
  ontology: true,
  notes: true,
  factors: true,
  blockType: true,
  description: true,
};

/** REQUIRED 键集 —— 与 canonical `scripts/schema/block.schema.json#required` 逐字对齐（含 `block`）；
 *  运行期由 `assertCanonicalRequiredKeys()` 断言（漂移 ⇒ exit 2），注释不再承担判据职责。 */
const REQUIRED_KEYS = [
  "id",
  "block",
  "name",
  "version",
  "services",
  "coordinates",
  "aliothVersion",
  "sharing",
];

/**
 * canonical 单源断言（unify-preproc-schema-source）：本门禁 REQUIRED 键集 MUST 与
 * `scripts/schema/block.schema.json#required` 逐字一致——把「逐字对齐」（旧为注释声称）
 * 升级为运行期判据；漂移 ⇒ exit 2（驳回·人在回路），MUST NOT 静默按陈旧字面量继续。
 */
function assertCanonicalRequiredKeys(): void {
  const read = readSchema(ROOT, "block");
  if (!read.ok) {
    SINK.err(`❌ [驳回] ${read.error}`);
    exit(2);
  }
  const got = schemaNode(read.value, ["required"]);
  if (
    !got.ok ||
    !Array.isArray(got.value) ||
    got.value.some((x) => typeof x !== "string")
  ) {
    SINK.err(
      `❌ [驳回] ${read.rel}#required 非字符串数组（${got.ok ? "类型不符" : got.error}）`,
    );
    exit(2);
  }
  const canonical = got.value as string[];
  const have = new Set(canonical);
  const want = new Set(REQUIRED_KEYS);
  const missing = REQUIRED_KEYS.filter((k) => !have.has(k));
  const extra = canonical.filter((k) => !want.has(k));
  if (missing.length > 0 || extra.length > 0) {
    SINK.err(
      `❌ [驳回] REQUIRED 键集漂移：${read.rel}#required 与门禁字面量不一致`,
    );
    if (missing.length > 0)
      SINK.err(`   canonical 缺（门禁有）：${missing.join(", ")}`);
    if (extra.length > 0)
      SINK.err(`   门禁缺（canonical 有）：${extra.join(", ")}`);
    exit(2);
  }
}

/** 组合键：属 module.json#blockAssembly，出现即 R1（错误信息里点名） */
const COMPOSITION_KEYS: Record<string, true> = {
  module: true,
  moduleId: true,
  order: true,
  group: true,
  entry: true,
  routePrefix: true,
};

const PLACEHOLDER_CODES: Record<string, true> = {
  "↓_XX": true,
  "↑_XX": true,
  "!_XX": true,
  "↓__XX": true,
  "↑__XX": true,
  XX: true,
};

/** R10 桩源阈值（字节）：低于此值不可能承载真实块 UI（技能基线 250+ 行） */
const STUB_SOURCE_MAX_BYTES = 200;

/**
 * R10 语义桩判据：默认导出组件体仅 `return null`（无 JSX／无状态／无副作用）——
 * 带文档注释的声明壳源会超出字节阈值（实测 233–247B）而逃过 size 判据，
 * 但产物仍是空壳（视觉验证 ready_viewports=0），故与阈值判据取或。
 */
function isNullReturnStub(source: string): boolean {
  return /export\s+default\s+function\s+\w*\s*\([^)]*\)\s*\{\s*return\s+null\s*;?\s*\}/.test(
    source,
  );
}

/**
 * 声明式共享入口（非桩源）：Block 原型单元被 ≥2 个 namespace 引用时，实现 MUST 单源于
 * 平台共享面 `.agents/skills/alioth-design/references/blocks/<unit>/`
 * （openspec/specs/ns-isolation :: shared-prototype-single-source），ns 侧
 * `llm-tsx/block.tsx` 只保留 re-export 入口——它承载真实实现（渲染非空壳），
 * 故 MUST NOT 按「桩源」判据拦截。
 */
function isSharedBlockEntry(source: string): boolean {
  return /export\s*\{\s*default\s*\}\s*from\s*['"][^'"]*\.agents\/skills\/alioth-design\/references\/blocks\//.test(
    source,
  );
}

/** 违规详情里的实测值渲染（类型 + 截断后的字面量；循环引用等不可序列化值退化为 String） */
function describeValue(v: unknown): string {
  const type = Array.isArray(v) ? "array" : v === null ? "null" : typeof v;
  let text: string;
  try {
    text = JSON.stringify(v) ?? String(v);
  } catch {
    text = String(v);
  }
  if (text.length > 60) text = `${text.slice(0, 57)}…`;
  return `类型=${type} 值=${text}`;
}

interface Ctx {
  services: Set<string>;
  modules: Set<string>;
  /** 模块 id → 该模块 `blockAssembly.blocks[]`（兼容顶层 `blocks[]`）登记的块 id 集合（R9 判据） */
  moduleBlocks: Map<string, Set<string>>;
  scenes: Set<string>;
  factors: Set<string>;
  functions: Set<string>;
  factorIsServiceAlias: boolean;
  /** 维度行 id 解析（code → `isahl.zc_id_{dim}.id` 文本）；构建期不可得即驳回（R5b MUST NOT 跳过） */
  dimIdOf: (dim: string, code: string) => string | null;
  /** 扫描根（`v.file` 相对它取路径）：main = ROOT、导入方 = 其传入 root（结论与 cwd 无关） */
  scanRoot: string;
}

/**
 * 维度行 id（文本）解析 —— 经 `DATABASE_URL` + psql（与 `check-dk-binding-consistency.ts` 同通道；
 * `mise run schema-info` 二进制未构建时 R5 的码存在性会跳过，本通道独立可用）。
 * 返回 `id::text` 以免 17 位 id 经 Number 舍入。
 *
 * `DATABASE_URL` 缺失时回落到仓内惯例 dev 库（与 `db-ready.sh` / `check-leaf-insert.ts` /
 * `check-embedded-id-class.ts` 同字面量）：门禁运行器（`scripts/pre/gates.ts`）不导出该变量，
 * 若无默认值则任何触及 block.json 的提交/推送都被「未设置 DATABASE_URL」驳回（实测自伤）。
 * 不可达时不降级：维度码通道（`dbDimensionCodes` 经 `mise run schema-info`）先抛
 * DimensionCodesUnavailableError ⇒ 退出码 2（`check-dimension-codes-fail-closed.test.ts` 覆盖）。
 */
function dbDimensionIdResolver(): (dim: string, code: string) => string | null {
  const dbUrl =
    process.env.DATABASE_URL ??
    "postgres://isahl@localhost:5432/aliothstudio_dev";
  const cache = new Map<string, string | null>();
  return (dim: string, code: string): string | null => {
    const key = `${dim}|${code}`;
    if (cache.has(key)) return cache.get(key) ?? null;
    const r = spawnSync(
      "psql",
      [
        dbUrl,
        "-tA",
        "-c",
        `SELECT id::text FROM isahl.zc_id_${dim} WHERE code = '${code.replace(/'/g, "''")}' AND deleted_at IS NULL LIMIT 1`,
      ],
      { encoding: "utf-8", timeout: 15000 },
    );
    const id = r.status === 0 ? r.stdout.trim() || null : null;
    cache.set(key, id);
    return id;
  };
}

/**
 * `--scope` 参数解析：接受 block.json 文件路径，或块 id / 块目录（`--scope block-theme` 直觉用法），
 * 亦接受 `<ns>/<blockId>` 限定形。仅在参数**不是现有文件**时才按 id 检索
 * （AppAgent 产物门禁传显式路径，行为不变）；同 id 跨 ns 命中多个且未加限定/`--ns` 时驳回（退出码 2），
 * MUST NOT 静默取首个。
 */
function resolveBlockJsonArg(arg: string, nsHint: string | null): string {
  const direct = resolve(ROOT, arg);
  if (existsSync(direct) && statSync(direct).isFile()) return direct;
  const all = listNamespaces(ROOT);
  const segs = arg.replace(/\/+$/, "").split("/").filter(Boolean);
  let hint = nsHint;
  let id = basename(direct);
  if (!hint && segs.length >= 2 && all.includes(segs[segs.length - 2])) {
    hint = segs[segs.length - 2];
    id = segs[segs.length - 1];
  }
  const scoped = hint ? all.filter((ns) => ns === hint) : all;
  if (hint && scoped.length === 0) {
    SINK.err(
      `\n❌ --scope '${arg}' 的 ns 限定 '${hint}' 不是已知 ns（${all.join(", ")}）`,
    );
    exit(2);
  }
  const hits = scoped.flatMap((ns) =>
    unitFiles(ROOT, ns, "Blocks", "block.json").filter(
      (f) => basename(dirname(f)) === id,
    ),
  );
  if (hits.length > 1) {
    SINK.err(
      `\n❌ --scope '${arg}' 在多个 ns 命中块 id '${id}'：\n   ${hits.map((h) => relPath(ROOT, h)).join("\n   ")}`,
    );
    SINK.err(
      "   处置：改传显式 block.json 路径，或加 ns 限定（`--scope <ns>/<blockId>` / `--ns <ns>`）。",
    );
    exit(2);
  }
  return hits[0] ?? direct; // 零命中：保持原路径（下游以 IO 错误显式报错）
}

function contextFor(
  root: string,
  ns: string,
  dims: { scenes: Set<string>; factors: Set<string>; functions: Set<string> },
  dimIdOf: (dim: string, code: string) => string | null,
): Ctx {
  return {
    services: unitIds(root, ns, "Services"),
    modules: unitIds(root, ns, "Modules"),
    moduleBlocks: moduleRegistrationIndex(root, ns),
    scenes: dims.scenes,
    factors: dims.factors,
    functions: dims.functions,
    dimIdOf,
    scanRoot: root,
    // WZ 口径：block.services[] 可为 factor 码（BLOCK_SCHEMA §1.1 `services` 说明）
    factorIsServiceAlias: ns === "WZ",
  };
}

/**
 * 模块 id（目录名）→ 该模块登记的块 id 集合（R9 判据）。
 * 登记形态取 `blockAssembly.blocks[]`（对象 `{id}` 或字符串）∪ 顶层 `blocks[]`——
 * 后者为 `reconcile-module-declaration-drift` 记录的两种历史形态并存期兼容面。
 */
/**
 * 单元所属**面**（`Apps` / `Open`；遗留扁平面 → `legacy`）。
 *
 * 为什么需要（2026-10-05 实测）：`Apps` 与 `Open` 两面可存在**同名模块**（实测 Cosmic-Tools：
 * `Sources/Apps/Modules/system-settings` 注册 `block-manage-settings`，而
 * `Sources/Open/Modules/system-settings` 注册 theme/language）——`moduleRegistrationIndex` 原以
 * `basename(dirname(file))` 为键 ⇒ 两面互相覆盖（后写者胜）⇒ R9 对 Apps 面的块**假阳性**
 * （「该块在 Gateway Navigator 不可达」的结论不成立）。键面限定后，R9 按**块自身所在面**解析登记集。
 */
function faceOf(rel: string): string {
  const m = /^Pre-Proc\/[^/]+\/Sources\/(Apps|Open)\//.exec(rel);
  return m ? m[1]! : "legacy";
}

function moduleRegistrationIndex(
  root: string,
  ns: string,
): Map<string, Set<string>> {
  const index = new Map<string, Set<string>>();
  for (const file of unitFiles(root, ns, "Modules", "module.json")) {
    const ids = new Set<string>();
    const collect = (entries: unknown): void => {
      if (!Array.isArray(entries)) return;
      for (const e of entries) {
        if (typeof e === "string" && e.trim()) ids.add(e.trim());
        else if (isRecord(e) && typeof e.id === "string" && e.id.trim())
          ids.add(e.id.trim());
      }
    };
    try {
      const mod = parseBigIntSafe(readFileSync(file, "utf-8"));
      if (isRecord(mod)) {
        const ba = mod.blockAssembly;
        if (isRecord(ba)) collect(ba.blocks);
        else collect(ba);
        collect(mod.blocks);
      }
    } catch {
      // module.json 不可解析：该模块登记集为空 = 零登记（R9 对声明归属该模块的块判失败）
    }
    // 键 = `<面>/<moduleId>`（消同名冲突）+ 裸 `<moduleId>`（兼容既有解析点；两面同 id 时并集）
    const id = basename(dirname(file));
    const faceKey = `${faceOf(relPath(root, file))}/${id}`;
    for (const key of new Set([faceKey, id])) {
      const cur = index.get(key) ?? new Set<string>();
      for (const v of ids) cur.add(v);
      index.set(key, cur);
    }
  }
  return index;
}

function checkBlock(
  ns: string,
  file: string,
  ctx: Ctx,
  out: Violation[],
): void {
  const rel = relPath(ctx.scanRoot, file);
  let data: unknown;
  try {
    data = parseBigIntSafe(readFileSync(file, "utf-8"));
  } catch (e) {
    out.push({ ns, file: rel, rule: "R2", detail: `JSON 解析失败: ${e}` });
    return;
  }
  if (!isRecord(data)) {
    out.push({ ns, file: rel, rule: "R2", detail: "顶层非对象" });
    return;
  }

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
    if (!(key in data))
      out.push({
        ns,
        file: rel,
        rule: "R2",
        detail: `缺 REQUIRED 键 '${key}'`,
      });
  }

  // R11 block-code-invalid：`block` 是块业务码（block.schema.json#properties.block.type = string）——
  // 存在即 MUST 为非空字符串；空串/对象/数组/数字/布尔 皆判失败（缺键由 R2 判，二者不重叠）
  if ("block" in data) {
    const code = data.block;
    if (typeof code !== "string" || !code.trim()) {
      out.push({
        ns,
        file: rel,
        rule: "R11",
        detail: `block 字段 MUST 为非空字符串业务码（block.schema.json#properties.block），实测 ${describeValue(code)}`,
      });
    }
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

  // R10 design-leg-missing：block.json 存在 ⇒ Prototypes 侧有 b-v{N}.html 且源非桩
  {
    const blockId =
      typeof data.id === "string" && data.id.trim()
        ? data.id.trim()
        : basename(dirname(file));
    // 设计腿候选根（布局契约 preproc-layout.mjs）：Gateway = Prototypes/Blocks、OpenActivity = Prototypes/Open/Blocks
    const protoDir =
      [
        join(ROOT, "Pre-Proc", ns, "Prototypes", "Blocks", blockId),
        join(ROOT, "Pre-Proc", ns, "Prototypes", "Open", "Blocks", blockId),
      ].find(
        (d) =>
          existsSync(d) && readdirSync(d).some((f) => /^b-v\d+\.html$/.test(f)),
      ) ?? join(ROOT, "Pre-Proc", ns, "Prototypes", "Blocks", blockId);
    const legs = existsSync(protoDir)
      ? readdirSync(protoDir).filter((f) => /^b-v\d+\.html$/.test(f))
      : [];
    if (legs.length === 0) {
      out.push({
        ns,
        file: rel,
        rule: "R10",
        detail: `Prototypes/Blocks/${blockId} 缺 b-v{N}.html 设计腿（目录或产物缺失）`,
      });
    } else {
      const srcPath = join(protoDir, "llm-tsx", "block.tsx");
      if (!existsSync(srcPath)) {
        out.push({
          ns,
          file: rel,
          rule: "R10",
          detail: `Prototypes/Blocks/${blockId} 有产物但缺 llm-tsx/block.tsx 构建源（无法重建）`,
        });
      } else {
        const source = readFileSync(srcPath, "utf8");
        const tooSmall = statSync(srcPath).size < STUB_SOURCE_MAX_BYTES;
        if (
          !isSharedBlockEntry(source) &&
          (tooSmall || isNullReturnStub(source))
        ) {
          out.push({
            ns,
            file: rel,
            rule: "R10",
            detail: tooSmall
              ? `Prototypes/Blocks/${blockId}/llm-tsx/block.tsx 为桩源（<${STUB_SOURCE_MAX_BYTES}B，产物空壳）`
              : `Prototypes/Blocks/${blockId}/llm-tsx/block.tsx 为空渲染声明壳（默认导出仅 return null，产物空壳）`,
          });
        }
      }
    }
  }
  // R3 ns-mismatch：`namespace` 可选，存在则 MUST 与路径一致（AGENTS.md：路径为权威）
  const nsField = data.namespace;
  if (typeof nsField === "string" && nsField !== ns) {
    out.push({
      ns,
      file: rel,
      rule: "R3",
      detail: `namespace='${nsField}' 与路径 ns='${ns}' 不一致`,
    });
  }

  // R4 services 解析（空数组合规：BLOCK_SCHEMA/check-config-json 明示 services 可为空数组）
  const services = data.services;
  if (Array.isArray(services)) {
    for (const s of services) {
      if (typeof s !== "string" || !s.trim()) continue;
      const id = s.trim();
      if (ctx.services.has(id)) continue;
      if (ctx.factorIsServiceAlias && ctx.factors.has(id)) continue;
      out.push({
        ns,
        file: rel,
        rule: "R4",
        detail: `services[] 元素 '${id}' 非本 ns 服务目录，也不是该 ns 约定的 factor 码`,
      });
    }
  }

  // R5 坐标码
  const coords = data.coordinates;
  if (isRecord(coords)) {
    const dims: Array<[string, Set<string>]> = [
      ["scene", ctx.scenes],
      ["factor", ctx.factors],
      ["function", ctx.functions],
    ];
    for (const [dim, valid] of dims) {
      const dimObj = coords[dim];
      if (!isRecord(dimObj)) continue;
      const code = typeof dimObj.code === "string" ? dimObj.code.trim() : "";
      if (!code) {
        out.push({
          ns,
          file: rel,
          rule: "R5",
          detail: `coordinates.${dim}.code 为空`,
        });
        continue;
      }
      if (PLACEHOLDER_CODES[code]) {
        out.push({
          ns,
          file: rel,
          rule: "R5",
          detail: `coordinates.${dim}.code='${code}' 为占位符`,
        });
        continue;
      }
      if (!valid.has(code)) {
        out.push({
          ns,
          file: rel,
          rule: "R5",
          detail: `coordinates.${dim}.code='${code}' 不在 isahl.zc_id_${dim}`,
        });
        continue;
      }
      // R5b：声明 id 与库中该 code 的维度行 id 逐位一致（防 >2^53 舍入 / 手抄错位；
      // 经 rawIntegerText 取原始数字文本，绝不走 Number）
      const declared =
        rawIntegerText(dimObj.id) ??
        (typeof dimObj.id === "number" ? String(dimObj.id) : "");
      if (!declared) {
        out.push({
          ns,
          file: rel,
          rule: "R5b",
          detail: `coordinates.${dim}.id 缺失或非整数（code='${code}'）`,
        });
        continue;
      }
      const dbId = ctx.dimIdOf(dim, code);
      if (dbId !== null && dbId !== declared) {
        out.push({
          ns,
          file: rel,
          rule: "R5b",
          detail: `coordinates.${dim}.id=${declared} 与库中 ${code} 的 id=${dbId} 不一致（精度丢位或抄错）`,
        });
      }
    }
  }

  // R6 sharing.ownerModule + R7 可复用声明（mode/consumers）
  const sharing = data.sharing;
  if (isRecord(sharing)) {
    if (typeof sharing.ownerModule === "string" && sharing.ownerModule.trim()) {
      const om = sharing.ownerModule.trim();
      const [mNs, mId] = om.split("/");
      if (!mNs || !mId)
        out.push({
          ns,
          file: rel,
          rule: "R6",
          detail: `ownerModule='${om}' 非法（期望 <ns>/<moduleId>）`,
        });
      else if (mNs !== ns)
        out.push({
          ns,
          file: rel,
          rule: "R6",
          detail: `ownerModule 跨 ns（'${om}' vs 文件 ns '${ns}'）`,
        });
      else if (!ctx.modules.has(mId))
        out.push({
          ns,
          file: rel,
          rule: "R6",
          detail: `ownerModule='${om}' 模块目录不存在`,
        });
      else {
        // R9：ownerModule 指向的模块 MUST 在 blockAssembly.blocks[]（或顶层 blocks[]）登记本块
        const blockId =
          typeof data.id === "string" && data.id.trim()
            ? data.id.trim()
            : basename(dirname(file));
        // 按**块自身所在面**解析（同名模块跨 Apps/Open 时避免互覆盖导致的假阳性；裸键为遗留面兼容）
        const registered =
          ctx.moduleBlocks.get(`${faceOf(rel)}/${mId}`) ??
          ctx.moduleBlocks.get(mId);
        if (registered && !registered.has(blockId)) {
          out.push({
            ns,
            file: rel,
            rule: "R9",
            detail: `ownerModule='${om}' 未在 blockAssembly.blocks[] 登记 '${blockId}'（该块在 Gateway Navigator 不可达；登记面 = 实现模块）。注：块合并/改指的过渡期内，旧块目录尚在而模块已改指新块时本规则会集中出现，属**预期中间态**；新块原型 build 完成并删除退役目录后 MUST 归零（顺序：先 build 消 R10/T3 → 再删目录消 R9；见 docs/specs/BLOCK_SCHEMA.md §7.4）`,
          });
        }
      }
    }

    const mode = typeof sharing.mode === "string" ? sharing.mode.trim() : "";
    if (mode && mode !== "single" && mode !== "shared") {
      out.push({
        ns,
        file: rel,
        rule: "R7",
        detail: `sharing.mode='${mode}' 非法（single|shared）`,
      });
    }
    const consumers = sharing.consumers;
    if (Array.isArray(consumers)) {
      if (mode === "shared" && consumers.length < 2) {
        out.push({
          ns,
          file: rel,
          rule: "R7",
          detail: `sharing.mode='shared' 时 consumers MUST ≥ 2（当前 ${consumers.length}）—— 否则该块非可复用单元`,
        });
      }
      for (const c of consumers) {
        if (typeof c !== "string" || !c.trim()) continue;
        const [cNs, cId] = c.trim().split("/");
        if (!cNs || !cId) {
          out.push({
            ns,
            file: rel,
            rule: "R7",
            detail: `consumers 元素 '${c}' 非法（期望 <ns>/<moduleId>）`,
          });
          continue;
        }
        if (cNs !== ns) {
          out.push({
            ns,
            file: rel,
            rule: "R7",
            detail: `consumers 元素 '${c}' 跨 ns（文件 ns='${ns}'）`,
          });
          continue;
        }
        if (!ctx.modules.has(cId)) {
          out.push({
            ns,
            file: rel,
            rule: "R7",
            detail: `consumers 元素 '${c}' 模块目录不存在`,
          });
        }
      }
    } else if (mode === "shared") {
      out.push({
        ns,
        file: rel,
        rule: "R7",
        detail: "sharing.mode='shared' 时 MUST 声明 consumers[]（≥ 2）",
      });
    }
  }
}

/** 基线 note 唯一来源（main 的 `--rebuild-baseline` 与 `regen-ns-face.ts` 共用）。 */
export const BLOCK_JSON_BASELINE_NOTE =
  "block.json 形状/引用存量欠债基线（check-block-json.ts）；新增违规不得落入此文件。R10 类（unwired 块）基线项 MUST 在 docs/specs/BLOCK_READY_UNWIRED_REGISTRY.md 有对应条目（基线 = 登记册镜像，非逃生舱）；R2/R11 中 `block` 业务码缺失/类型不符类按**类级**登记于该册 §3.6（逐 id 见本文件 violations[]；出口 = 命名权威给出各域码表后按域回填，MUST NOT 臆造）。";

/**
 * 缺口收集器（单一实现：main 与 `scripts/check/baselines/regen-ns-face.ts` 共用）。
 * `root` 显式传入 ⇒ 结论与 cwd 无关、权威判定面 = 该仓（ns 仓根传 ns 仓，平台仓传平台仓）。
 */
export function collectBlockViolations(
  root: string,
  nsFilter: string | null,
  dims: DimensionInputs["dims"],
  dimIdOf: DimensionInputs["dimIdOf"],
): { violations: Violation[]; files: number } {
  const violations: Violation[] = [];
  let files = 0;
  for (const ns of listNamespaces(root)) {
    if (nsFilter && ns !== nsFilter) continue;
    const ctx = contextFor(root, ns, dims, dimIdOf);
    for (const f of unitFiles(root, ns, "Blocks", "block.json")) {
      files++;
      checkBlock(ns, f, ctx, violations);
    }
  }
  return { violations, files };
}

function runBody(): number {
  const strict = STRICT;
  const update = REBUILD_BASELINE;
  const nsFilter = NS_FILTER_ARG;
  const onlyArg = ONLY_ARG;
  const only = onlyArg ? resolveBlockJsonArg(onlyArg, nsFilter) : null;

  assertCanonicalRequiredKeys();
  const { dims, dimIdOf } = loadDimensionInputs();

  // 单文件档：AppAgent 产物门禁（严格判失败，不比对基线）
  if (only) {
    const ns = only.split("/Pre-Proc/")[1]?.split("/")[0] ?? "";
    const violations: Violation[] = [];
    checkBlock(ns, only, contextFor(ROOT, ns, dims, dimIdOf), violations);
    SINK.out(`\n📋 block.json 校验 — ${relPath(ROOT, only)}（单文件/严格档）`);
    SINK.out(`   违规统计: ${summarize(violations)}`);
    for (const v of violations.slice(0, 20))
      SINK.out(`   [${v.rule}] ${v.detail}`);
    SINK.out(
      violations.length === 0
        ? "\n✅ block.json 校验通过"
        : "\n❌ block.json 校验未通过",
    );
    exit(violations.length === 0 ? 0 : 1);
  }

  const { violations, files } = collectBlockViolations(
    ROOT,
    nsFilter,
    dims,
    dimIdOf,
  );
  // 判定面归属（2026-09-25）：0 单元必须区分「本仓不持有 ns 面（显式跳过）」与「持有却解析失效（fail-loud）」
  // —— 原实现两者都打印「0 个文件」+ ✅（假绿）。
  if (files === 0) {
    if (!nsSourcesTracked(ROOT)) {
      nsFaceSkipNotice("check-block-json", ROOT);
      exit(0);
    }
    const nsLabel = nsFilter ?? nsFaceStatus(ROOT).realDirs.join(", ");
    SINK.err(
      `❌ check-block-json: ns「${nsLabel}」被本仓跟踪但解析出 0 个 block.json 单元（ROOT=${ROOT}）—— 扫描面失效，MUST NOT 静默通过`,
    );
    exit(1);
  }

  const baseline = loadBaseline(BASELINE_PATH);
  const fresh = violations.filter((v) => !baseline.entries.has(fingerprint(v)));

  SINK.out(
    `\n📋 block.json 校验 — ${files} 个文件（${nsFilter ?? "全 ns"}，${strict ? "严格档" : "基线档"}）`,
  );
  SINK.out(`   违规统计: ${summarize(violations)}`);
  if (baseline.entries.size > 0)
    SINK.out(
      `   基线已登记: ${baseline.entries.size} 条；本次新增: ${fresh.length} 条`,
    );
  else SINK.out(`   （基线不存在：执行 --rebuild-baseline 生成）`);

  const show = strict ? violations : fresh;
  for (const v of show.slice(0, 15))
    SINK.out(`   [${v.rule}] ${v.file}: ${v.detail}`);
  if (show.length > 15) SINK.out(`   … 其余 ${show.length - 15} 条省略`);

  if (update) {
    writeBaseline(BASELINE_PATH, violations, BLOCK_JSON_BASELINE_NOTE);
    SINK.out(
      `\n✅ 基线已写入 ${relPath(ROOT, BASELINE_PATH)}（${violations.length} 条）`,
    );
    exit(0);
  }

  const failed = strict ? violations.length > 0 : fresh.length > 0;
  SINK.out(failed ? "\n❌ block.json 校验未通过" : "\n✅ block.json 校验通过");
  exit(failed ? 1 : 0);
}

interface BlockJsonOutcome {
  out: string[];
  err: string[];
  rc: number;
  blocked: boolean;
}

/** 判定面（block.json 校验：R1–R9 规则族 + 基线档/严格档/单文件档）；输出经 SINK 记录，报告面回放。 */
export function blockJsonState(opts: {
  strict: boolean;
  rebuildBaseline: boolean;
  nsFilter: string | null;
  onlyArg: string | null;
}): BlockJsonOutcome {
  const OUT: string[] = [];
  const ERR: string[] = [];
  const saved = SINK;
  SINK = { out: (l) => OUT.push(l), err: (l) => ERR.push(l) };
  STRICT = opts.strict;
  REBUILD_BASELINE = opts.rebuildBaseline;
  NS_FILTER_ARG = opts.nsFilter;
  ONLY_ARG = opts.onlyArg;
  let rc = 0;
  try {
    rc = runBody();
  } catch (e) {
    if (!(e instanceof ExitSignal)) throw e;
    rc = e.rc;
  } finally {
    SINK = saved;
  }
  return { out: OUT, err: ERR, rc, blocked: false };
}

/** 最近一次判定（同步判定核 ⇒ 同步报告面复用） */
let LAST: BlockJsonOutcome = { out: [], err: [], rc: 0, blocked: false };

const gate = defineCheck({
  id: "block-json",
  label: "check-block-json",
  flags: ["--strict", "--rebuild-baseline"],
  valueFlags: ["--ns"],
  // 检查层单输入档旗标：基座保留 `--only`（收敛为 `--scope`）⇒ 按 scopeValue 接线
  scopeValue: "block-json",
  predicate: (ctx) => {
    LAST = blockJsonState({
      strict: ctx.flags.has("--strict"),
      rebuildBaseline: ctx.flags.has("--rebuild-baseline"),
      nsFilter: ctx.valueFlags.get("--ns")?.[0] ?? null,
      onlyArg: ctx.scopeValue,
    });
    return [];
  },
  report: (_ctx, _res, io) => {
    const st = LAST;
    for (const line of st.out) io.out(`${line}\n`);
    for (const line of st.err) io.err(`${line}\n`);
    return st.rc; // 违例/驳回恒阻断（旧实现不看 --fail）
  },
  selfTest() {
    // 判据自证（R1，正负例）：`faceOf` 面判定——Apps/Open 两面同名模块必须区分（R9 假阳性回归的守门判据）
    const cases: Array<[string, string]> = [
      ["Pre-Proc/WZ/Sources/Apps/Modules/system-settings/block.json", "Apps"],
      ["Pre-Proc/WZ/Sources/Open/Modules/system-settings/block.json", "Open"],
      ["Pre-Proc/WZ/Preproc/Modules/x/block.json", "legacy"],
    ];
    const bad = cases.filter(([rel, want]) => faceOf(rel) !== want);
    if (bad.length > 0)
      throw new Error(`faceOf 自测失败：${JSON.stringify(bad)}`);
    return 0;
  },
});

// 可导入模块：仅直接执行时跑 gate（regen-ns-face.ts 复用 collectBlockViolations）。
if (import.meta.main) process.exit(await gate.main());
