#!/usr/bin/env bun
/**
 * regen-ns-face.ts — B 类共享可变状态（平台单源、条目归 ns 内容）的**正规重生成入口**
 *
 * 规约来源：openspec/specs/shared-capability-single-source
 *   （capability `shared-capability-single-source`：shared-mutable-state-read-write-invariants）
 *
 * 为什么需要本入口（2026-09-26 实测缺口）：B 类基线的条目来源是 ns 仓内容，但单源文件在平台仓
 * ⇒ 两侧皆不可写：
 *   · ns 仓执行 `--write-baseline` ⇒ INV-1 拒绝（会整文件覆盖其他 ns 的存量登记，且改动在 ns 仓
 *     git status 不可见）；
 *   · 平台仓执行同命令 ⇒ 审计按「本仓不持有 ns 面」跳过（0 单元，非 fail-loud）。
 * 本入口在**平台仓**侧运行：缺口取自 ns 仓内容，写入平台单源（INV-1 满足；条目归属 = 目标 ns，
 * INV-1b 满足）。MUST NOT 在 ns 仓内复刻本入口。
 *
 * 两种形态：
 *   · **per-ns**（`--ns <NS>`）：单文件 `dto-field-mapping-<ns>.baseline.json`；
 *   · **global**（省略 `--ns`）：跨 ns 单文件（`block-json-baseline.json` 条目内嵌 ns 引用），
 *     逐挂载 ns 收集后聚合写入。
 *
 * 用法：
 *   bun scripts/check/baselines/regen-ns-face.ts --list
 *   bun scripts/check/baselines/regen-ns-face.ts --tool dto-field-mapping --ns <NS> [--check]
 *   bun scripts/check/baselines/regen-ns-face.ts --tool block-json [--check]
 */
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { auditNs, fingerprint, type Gap } from "../audit-dto-field-mapping";
import {
  BLOCK_JSON_BASELINE_NOTE,
  collectBlockViolations,
  loadDimensionInputs,
} from "../check-block-json";
import {
  COMPOSITION_BASELINE_NOTE,
  collectCompositionViolations,
} from "../check-composition";
import {
  dbDimensionCodes,
  fingerprint as violationFingerprint,
} from "../../lib/preproc-artifacts";
import { mountedNs, nsRepoOf } from "../../lib/ns-mounts";
import { writeBaseline as guardedWriteBaseline } from "../lib/baseline-store";

const PLATFORM = resolve(import.meta.dirname, "../../..");

interface ToolSpec {
  /** 基线相对平台仓根路径（per-ns 形态用 `__NS__` 占位） */
  baselineRel: string;
  /** per-ns（单 ns 文件）还是 global（跨 ns 单文件） */
  scope: "per-ns" | "global";
  /** 未常态化原因（非 null 表示尚未接入：调用即 fail-loud，禁静默空跑） */
  pendingReason?: string;
  /** 收集器：`targets` = [{ns, repo}]（per-ns 形态恰一项） */
  build?: (targets: Array<{ ns: string; repo: string }>) => {
    count: number;
    content: string;
  };
}

const DTO_NOTE =
  "存量 DTO 字段映射缺口归线（SPEC_ENFORCEMENT_SUBSTRATE §设计原则 1：增量，非全量）。指纹=sha256(ns|判据|服务|实体|列|期望 DTO|实际别名)——未命中指纹的缺口为新增，阻断；缺口清零后应 --write-baseline 收缩（check-gate-hygiene 报 rot）。";

const TOOLS: Record<string, ToolSpec> = {
  "dto-field-mapping": {
    baselineRel:
      "scripts/check/baselines/dto-field-mapping-__NS__.baseline.json",
    scope: "per-ns",
    build: ([target]) => {
      const audit = auditNs(target.repo, target.ns);
      if (audit.fatal) throw new Error(`audit fatal: ${audit.fatal}`);
      if (audit.skipReason)
        throw new Error(`audit skipped: ${audit.skipReason}`);
      const canonical = audit.nsCanonical ?? target.ns;
      const seen = new Set<string>();
      const entries: Array<Record<string, string>> = [];
      for (const g of (audit.gaps ?? []) as Gap[]) {
        const fp = fingerprint(canonical, g);
        if (seen.has(fp)) continue;
        seen.add(fp);
        entries.push({
          fp,
          ns: canonical,
          service: g.service,
          entity: g.entity,
          rule: g.kind,
          column: g.column,
        });
      }
      entries.sort((a, b) =>
        `${a.rule}\u0000${a.service}\u0000${a.entity}\u0000${a.column}`.localeCompare(
          `${b.rule}\u0000${b.service}\u0000${b.entity}\u0000${b.column}`,
        ),
      );
      const doc = {
        version: "1.0.0",
        rule_reference:
          "docs/specs/SERVICE_SPEC.md §4.1（DB 物理列 → DTO 字段命名分层）＋ §3.1（field_mappings 必填）",
        note: DTO_NOTE,
        generatedBy: `bun scripts/check/baselines/regen-ns-face.ts --tool dto-field-mapping --ns ${canonical.toLowerCase()}`,
        violations: entries,
      };
      return {
        count: entries.length,
        content: `${JSON.stringify(doc, null, 2)}\n`,
      };
    },
  },
  // 信封解包单源门禁（check-sources-envelope-unwrap.ts）：判定面 = ns Sources TS/TSX 内的重定义
  // （条目归 ns 内容、基线单源在平台仓）。ns 仓被 INV-1 拒绝、平台仓对 ns 面判据按归属跳过 ⇒ 两侧
  // 都不产基线 ⇒ 本工具自带正规维护入口，不在本入口复制采集逻辑（复制会绕过 ns 仓自拒守卫）。
  "sources-envelope-unwrap": {
    baselineRel:
      "scripts/check/baselines/ns-sources-envelope-unwrap.baseline.tsv",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-sources-envelope-unwrap.ts --init`，**仅平台仓**；ns 仓内以 `.alioth/platform.conf` 自拒）：由 AST 扫描重生成存量登记（当前 111 条），写入经唯一入口 `writeBaseline(ROOT, …)`（INV-1）。前置 = 平台仓根且 5 个 ns 挂载可达；MUST NOT 由本入口复制该逻辑（复制会绕过 ns 仓自拒/归属守卫）。",
  },
  // 本我声明待办（add-ns-self-subject-binding-contract）：条目 = 各 ns 的「system 哨兵 → 业务主体（本我）」
  // 声明状态（人工维护——完成声明式绑定的 ns 由 owner 删除条目；断言 5 对「达成仍留册」判红强制销账）。
  // 消费端 = `scripts/check/check-ns-subject-bootstrap.sh`（push 档审计）；无自动采集器（状态由 ns 仓
  // 种子声明决定，非可扫描产物），故以 pendingReason 指向自带维护入口。
  "ns-self-subject-pending": {
    baselineRel: "scripts/check/baselines/ns-self-subject-pending.txt",
    scope: "global",
    pendingReason:
      "人工维护的「本我声明待办」登记（无自动采集器；条目 = ns 名 + 状态说明，完成声明的 ns 由 owner 从本文件删除——门禁对达成后留册判红，只减不增）。消费端 = scripts/check/check-ns-subject-bootstrap.sh 断言 5（非占位本我绑定）；写入门禁 = scripts/check/lib/baseline-store.ts（平台仓单源，INV-1）。",
  },
  // 原型卫生三判据（check-prototype-hygiene.ts，2026-10-09 立）：条目 = `<ns>|<path>|<message>`（跨 ns 单文件）。
  // 判据 = ① i18n 字典覆盖（裸 t() 回显裸键）② 页脚品牌串味 ③ 声明版视觉证据（html+report 双在 ∧ report mtime ≥ html）。
  // 本工具自带正规维护入口，不在本入口复制采集逻辑（复制会绕过 ns 仓自拒/归属守卫）。
  "prototype-hygiene": {
    baselineRel: "scripts/check/baselines/prototype-hygiene.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-prototype-hygiene.ts --update-baseline`，**仅平台仓根**）：判定面 = 原型卫生三判据；写入经唯一入口 `writeBaseline`（INV-1）。存量 = 他 ns 的 footerBrand 串味与声明版缺视觉证据（归属方修后 `--update-baseline` 收缩，只减不增）。前置 = 平台仓根且 5 个 ns 挂载可达；MUST NOT 由本入口复制该逻辑。",
  },
  // 三层 ESM 集成契约门禁（check-prototype-contract.sh）存量归线（2026-10-08 立）：
  // 条目 = `<ns>|<unit>|<判据原文>`（跨 ns 单文件）。本工具自带正规维护入口，不在本入口复制采集逻辑
  // （复制会绕过 ns 仓自拒/归属守卫，与 sources-envelope-unwrap 同口径）。
  "prototype-contract": {
    baselineRel: "scripts/check/baselines/prototype-contract.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-prototype-contract.sh --ci --ns all --layer all --baseline scripts/check/baselines/prototype-contract.baseline.txt --update-baseline`，**仅平台仓根**；ns 仓内以 `.alioth/platform.conf` 自拒）：D 段判定 + `--ci` 严格档 + 基线棘轮由本次复活（AST 依赖根解析 / ns 档 N/A / `--ci` 与 `--baseline` 透传 / 基线棘轮）实装；写入经唯一入口 `writeBaseline`（INV-1）。前置 = 平台仓根且 5 个 ns 挂载可达；MUST NOT 由本入口复制该逻辑。存量 = 各 ns ESM 集成契约违规（App 壳 navGroups/TopBar-only 标志、少数模块/块 displayName 与 id 不一致等），只减不增。",
  },
  "block-json": {
    baselineRel: "scripts/check/baselines/block-json-baseline.json",
    scope: "global",
    build: (targets) => {
      const { dims, dimIdOf } = loadDimensionInputs(); // DB 不可用 ⇒ 抛错 fail-closed（与门禁同源）
      const lines: string[] = [];
      const seen = new Set<string>();
      let files = 0;
      for (const t of targets) {
        const { violations, files: n } = collectBlockViolations(
          t.repo,
          t.ns,
          dims,
          dimIdOf,
        );
        files += n;
        for (const v of violations) {
          const fp = violationFingerprint(v);
          if (seen.has(fp)) continue;
          seen.add(fp);
          lines.push(fp);
        }
      }
      if (files === 0)
        throw new Error(
          "收集到 0 个 block.json 单元（ns 面挂载/解析失效）——拒绝写出空基线",
        );
      lines.sort();
      const doc = {
        updated_at: new Date().toISOString(),
        note: BLOCK_JSON_BASELINE_NOTE,
        violations: lines,
      };
      return {
        count: lines.length,
        content: `${JSON.stringify(doc, null, 2)}\n`,
      };
    },
  },
  // 组合契约（`check-composition.ts`）：判定面跨 ns（module.json / app.json 引用），单文件跨 ns 聚合
  // ⇒ 由本入口采集（门禁自身在平台仓按归属跳过、在 ns 仓无写权 ⇒ 两侧都不产基线，必须在此产）。
  composition: {
    baselineRel: "scripts/check/baselines/composition-baseline.json",
    scope: "global",
    build: (targets) => {
      const factors = dbDimensionCodes(PLATFORM, "factor"); // DB 不可得 ⇒ 抛错 fail-closed（与门禁同源）
      const entries: string[] = [];
      const seen = new Set<string>();
      let units = 0;
      for (const t of targets) {
        const scan = collectCompositionViolations(PLATFORM, t.ns, factors);
        units += scan.modules + scan.apps;
        for (const v of scan.violations) {
          const fp = violationFingerprint(v);
          if (seen.has(fp)) continue;
          seen.add(fp);
          entries.push(fp);
        }
      }
      if (units === 0) {
        throw new Error(
          "收集到 0 个 module/app 单元（ns 面挂载/解析失效）——拒绝写出空基线",
        );
      }
      entries.sort();
      const doc = {
        updated_at: new Date().toISOString(),
        note: COMPOSITION_BASELINE_NOTE,
        violations: entries,
      };
      return {
        count: entries.length,
        content: `${JSON.stringify(doc, null, 2)}\n`,
      };
    },
  },
  "dynamic-table-name": {
    baselineRel: "scripts/check/baselines/dynamic-table-name.baseline.json",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-dynamic-table-name.ts --refresh-lines`，平台仓执行）：行号再锚定 + 组内 1:1 保序 + **孤儿/枚举外 source 拒写**（禁臆造 source/reason），写入经归属守卫（`writeBaseline(ROOT,…)`）；扫描面经 `statSync` 穿透 `Pre-Proc/*` 符号链接挂载（2026-09-28 实测平台仓扫到 ns 内容：2240 文件 / 373 站点，基线内命中 0 新）。MUST NOT 由本入口复制该逻辑（会绕过拒写护栏）",
  },
  // 种子边界（`check-seed-layer-ownership.ts`）：自带 `--init`；扫描面按名枚举 `Pre-Proc`/`Deploy`
  // 下的 ns 目录（符号链接按名可见，2026-09-28 实测平台仓扫到 Pre-Proc+Deploy 147 文件）。
  "seed-layer-ownership": {
    baselineRel: "scripts/check/baselines/ns-seed-dml.baseline.tsv",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-seed-layer-ownership.ts --init`，平台仓执行）：按名枚举挂载 ns 面，平台仓可完整采集所有 ns 的 DML 命中；写入经归属守卫。单 ns 形态（`--ns`）MUST NOT 用于重生成（其余 ns 条目不可见 ⇒ 会整文件覆盖）",
  },
  // T1 全量运行快照（冻结结果日志，条目 = 各检查的结论行，非可归属违规）——
  // 其 `Pre-Proc/...` 出现位于摘要文本内，不构成 ns 条目型基线。
  "check-all-snapshot": {
    baselineRel: "scripts/check/baselines/check-all-baseline.json",
    scope: "global",
    pendingReason:
      "运行快照（`bash scripts/check/check-all.sh --init` 冻结的 T1 结论），非 ns 条目型基线：其中的 `Pre-Proc/...` 出现位于检查摘要文本内，不承载 ns 归属；重生成即整文件重跑快照（平台仓执行）",
  },
  // 资源注册覆盖率（`check-resource-registry-coverage.ts`）：自带 `--write-baseline`；扫描面按名枚举
  // `Pre-Proc/*/Sources/{Apps/}Services/*/service.json`（符号链接挂载按名可见，平台仓可完整采集）。
  "resource-registry-coverage": {
    baselineRel:
      "scripts/check/baselines/resource-registry-coverage.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-resource-registry-coverage.ts --write-baseline`，平台仓执行）：按名枚举挂载 ns 面的服务声明面，条目标识 = `<service.json 相对路径>::<未注册实体>`（实体为语义键、不含行号 ⇒ 无再锚定需求）；写入经归属守卫（`writeBaseline(ROOT,…)`）。MUST NOT 由本入口复制该逻辑",
  },
  // Service 隔离粒度（`check-service-granularity.ts`，ADR D-048）：自带 `--write-baseline`；扫描面按名枚举
  // `Pre-Proc/*/Sources/Apps/Services/*/backend/src`（符号链接挂载按名可见，平台仓可完整采集）。
  "service-granularity": {
    baselineRel: "scripts/check/baselines/service-granularity.baseline.tsv",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-service-granularity.ts --write-baseline`，平台仓执行）：按名枚举挂载 ns 面的服务后端源码，条目 = 违规文件相对路径（无行号 ⇒ 无再锚定需求）；写入经归属守卫（`writeBaseline(ROOT,…)`）；扫描到 0 个 service.json 时拒绝写出空基线。MUST NOT 由本入口复制该逻辑",
  },
  // 抽屉容器合规（`check-drawer-container.ts`，openspec `form-drawer` §drawer-docks-left-of-right-sidebar）：
  // 自带 `--write-baseline --all-ns`（平台仓执行）；扫描面 = `Pre-Proc/*` 挂载 ns 面（模块 frontend/src）
  // + 平台 FE（Framework/Gateway/Meta/SSO frontend/<pkg>/src）；ns 仓内运行会被归属守卫拒绝。
  "drawer-container": {
    baselineRel: "scripts/check/baselines/drawer-container.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-drawer-container.ts --write-baseline --all-ns`，平台仓执行）：条目 = `<path> <sha256[:16]>`（无行号 ⇒ 无再锚定需求），被修改即重检（只减不增）；写入经归属守卫（`writeBaseline(ROOT,…)`），ns 仓内运行被拒（防幽灵覆盖其他 ns 存量登记）。MUST NOT 由本入口复制该逻辑",
  },
  // 清单 Schema 单源（`check-schema-single-source.ts`，unify-preproc-schema-source）：条目 = ns 面
  // `_schema/<f>.schema.json` 相对路径（形态判据：MUST 为指向 `scripts/schema/` 的符号链接）。
  "schema-single-source": {
    baselineRel: "scripts/check/baselines/schema-single-source.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-schema-single-source.ts --write-baseline`，平台仓执行）：条目 = 违规面相对路径（无行号 ⇒ 无再锚定需求）；存量 20 实体副本的收敛（实体 → 符号链接）归 ns 侧 mission，收敛后 MUST 经该入口收缩至 0 条；写入经归属守卫（`writeBaseline(ROOT,…)`），ns 仓内运行被拒。MUST NOT 由本入口复制该逻辑",
  },
  // 平台 ↔ ns 依赖方向（`check-platform-ns-dependency-direction.ts`）：条目 = **平台受版本控制清单的
  // 内容**（pnpm workspace 成员 / lock importer 键 / cargo 清单 path 值）；命中本登记仅因键文本出现
  // `Pre-Proc/` 命名（ns 面命名词汇），**不承载 ns 仓内容归属**（不读 ns 仓内容即可完整采集）。
  "platform-ns-dependency-direction": {
    baselineRel:
      "scripts/check/baselines/platform-ns-dependency-direction.baseline.txt",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-platform-ns-dependency-direction.ts --update-baseline`，平台仓执行；前置 = 平台仓根即满足，无需 ns 内容）：条目 = 平台清单内容快照，迁移（change move-ns-frontend-assembly-to-ns）收口后应为 0 条并 MUST 经该入口重写；写入经唯一入口 baseline-store（INV-1）。MUST NOT 由本入口复制该逻辑（会引入第二实现）",
  },
  // 生成产物陈旧（`check-generated-artifact-freshness.ts`）：条目归 ns 内容（`Pre-Proc/{ns}/Apps/**`），
  // 单源在平台仓；ns 仓被 INV-1 拒绝 ⇒ 由门禁自带入口在平台仓重写。
  "generated-artifact-freshness": {
    baselineRel:
      "scripts/check/baselines/generated-artifact-freshness.baseline.json",
    scope: "global",
    pendingReason:
      "本工具自带正规维护入口（`bun scripts/check/check-generated-artifact-freshness.ts --refresh-baseline`，**仅平台仓**执行；前置 = 平台仓根 + 5 ns 挂载可达）：条目 = `Pre-Proc/{ns}/Apps/**` 三件产物的越集模块/块 id 与陈旧 prototype（键=rule|file|subject，逐条带 reason），经唯一入口 `writeBaseline(ROOT,…)`（INV-1）重写，存量条目按 key 保留原 reason。MUST NOT 由本入口复制该逻辑（复制会绕过 ns 仓归属守卫）。产物本身的出口 = 重跑 AppAgent 流水线 / alioth-compose（非本基线职责）。",
  },
};

/** 已登记的基线 rel（`__NS__` = per-ns 占位）；供「无写入方的 ns 条目型基线」审计消费。 */
export function registeredBaselineRels(): string[] {
  return Object.values(TOOLS).map((spec) => spec.baselineRel);
}

function arg(name: string): string | null {
  const i = process.argv.indexOf(name);
  return i >= 0 ? (process.argv[i + 1] ?? null) : null;
}

function nsTarget(ns: string): { ns: string; repo: string } {
  const face = join(PLATFORM, "Pre-Proc", ns);
  if (!existsSync(face)) throw new Error(`ns 面不存在：${face}`);
  return { ns, repo: nsRepoOf(face) };
}

function allTargets(): Array<{ ns: string; repo: string }> {
  return mountedNs(PLATFORM).map((dir) => ({
    ns: dir.split("/").pop() as string,
    repo: nsRepoOf(dir),
  }));
}

function main(): void {
  if (process.argv.includes("--list")) {
    for (const [name, spec] of Object.entries(TOOLS)) {
      console.log(
        `${spec.pendingReason ? "⏳" : "✅"} ${name}（${spec.scope}）→ ${spec.baselineRel}` +
          (spec.pendingReason ? `\n    未接入：${spec.pendingReason}` : ""),
      );
    }
    return;
  }
  const tool = arg("--tool");
  const ns = arg("--ns");
  const checkOnly = process.argv.includes("--check");
  if (!tool) {
    console.error(
      "用法：bun scripts/check/baselines/regen-ns-face.ts --tool <name> [--ns <NS>] [--check]｜--list",
    );
    process.exit(2);
  }
  const spec = TOOLS[tool];
  if (!spec) {
    console.error(
      `未知 tool「${tool}」；已登记：${Object.keys(TOOLS).join("、")}（--list 查看就绪态）`,
    );
    process.exit(2);
  }
  if (spec.pendingReason || !spec.build) {
    console.error(
      `❌ tool「${tool}」不由本入口承接（禁止静默空跑）：\n    ${spec.pendingReason}`,
    );
    process.exit(2);
  }
  if (spec.scope === "per-ns" && !ns) {
    console.error(`❌ tool「${tool}」为 per-ns 形态，MUST 传 --ns <NS>`);
    process.exit(2);
  }
  if (spec.scope === "global" && ns) {
    console.error(
      `❌ tool「${tool}」为 global 形态（跨 ns 单文件），MUST NOT 传 --ns`,
    );
    process.exit(2);
  }

  const targets =
    spec.scope === "per-ns" ? [nsTarget(ns as string)] : allTargets();
  if (targets.length === 0) {
    console.error("❌ 未发现任何已挂载 ns——扫描面失效，MUST NOT 静默通过");
    process.exit(1);
  }
  const { count, content } = spec.build(targets);
  const baselineRel = spec.baselineRel.replace(
    "__NS__",
    (ns ?? "").toLowerCase(),
  );
  const baselineAbs = join(PLATFORM, baselineRel);

  if (checkOnly) {
    // 语义比对：只比 violations 条目集合（generatedBy/updated_at 等元数据库不属判据）
    const cur = existsSync(baselineAbs)
      ? readFileSync(baselineAbs, "utf8")
      : null;
    const keyOf = (text: string): string | null => {
      try {
        const list = (JSON.parse(text).violations ?? []).map((v: unknown) =>
          JSON.stringify(v),
        );
        return [...new Set(list)].sort().join("\n");
      } catch {
        return null;
      }
    };
    const want = keyOf(content);
    const got = cur === null ? null : keyOf(cur);
    if (got !== null && got === want) {
      console.log(
        `✅ ${tool}${ns ? ` / ${ns}` : ""}：基线条目集合与 ns 面一致（${count} 条）`,
      );
      return;
    }
    console.error(
      `✗ ${tool}${ns ? ` / ${ns}` : ""}：基线需要重生成（计算 ${count} 条；现文件${cur === null ? "缺失" : got === null ? "不可解析" : "条目集合不一致"}）——去掉 --check 执行`,
    );
    process.exit(1);
  }
  guardedWriteBaseline(PLATFORM, baselineAbs, content);
  console.log(
    `✅ ${tool}${ns ? ` / ${ns}` : ""}：已写入 ${baselineRel}（${count} 条，内容取自 ${targets.length} 个 ns 面）`,
  );
}

// 可导入模块：仅直接执行时跑 main（`check-baseline-ownership.ts` 复用 `registeredBaselineRels`）。
if (import.meta.main) main();
