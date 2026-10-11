#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * derived-artifacts.ts — 「派生新鲜度」族的**唯一成员清单**（single source of truth）
 *
 * 为什么要有这一份：这类门禁的形态同一（`真源 → 派生/生成物`，判据 = 两者一致 / 派生不过期），
 * 但历史上每个族各有自己的登记行与 argv（`gates.ts` 的 `GATES` + `CHECK_ALL` 侧表逐条写脚本路径），
 * 于是「族成员是谁 / 各成员在暂存档与全量档各以什么 argv 跑 / 有没有棘轮基线」只能靠人工比对，
 * 跨处必然漂移。本文件是**族元数据的唯一来源**：`scripts/check/derived-freshness.ts`（统一接口）
 * 只从本表派发子进程，登记面（`GATES` / `CHECK_ALL`）只写「接口 + 族 id」，MUST NOT 再逐族写 probe 路径。
 *
 * 消费方：
 *   · `scripts/check/derived-freshness.ts` —— dispatcher（`--family all|<id> [--staged|--full]`）；
 *   · 门禁引用解析（`gate-refs` 的 `AGGREGATE_FACE_FILES`）—— 本文件即族的**聚合执行面**来源
 *     （probe 脚本经本表被 dispatcher 执行 ⇒ 声明面归属 `aggregate`，见 `run-gates --audit` ③）。
 *
 * 覆盖范围（2026-10-08 裁决：对账机制覆盖全部「真源 → 派生/副本」族）：
 *   纳入判据 = 同一事实存在**产物/副本侧**（由真源经生成器 / 物化 / 发布流程产生、可重生成），
 *   门禁 = 两处不一致即红。DB 依赖族由 gate `needsDb` 承接；双档位（平台 / ns 仓）由脚本自判；
 *   通知 / 人工提醒 MUST NOT 作为任一族的唯一守护（prototype-drift 已批量化转硬门禁）。
 *   不在族（判据 = 无产物/副本侧）：`check-repo-root-single-source`（整仓单源存在性）、
 *   声明↔事实 / 声明↔声明一致性（`check-module-manifest-consistency`、`check-dk-binding-consistency`、
 *   `check-service-coords-consistency`、`check-ns-entity-coordinate-consistency`）与禁重复实现类
 *   single-source 守卫；完整分类表 = change `extend-derived-freshness-coverage` design.md。
 */
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";

/** probe 调用上下文（入口 argv 的两档；无分档者两档同值）。 */
export type DerivedContext = "staged" | "full";

/** 族成员的执行档位（gate 阶段；`check-all` = 仅经 CHECK_ALL 全量面执行）。 */
export type DerivedStage = "commit" | "push" | "check-all";

/** 族成员条目（逐字为唯一真源；dispatcher 与登记面均从本表取值）。 */
export interface DerivedFamily {
  /** 族 id（`--family <id>` 的取值；MUST 唯一、kebab-case）。 */
  id: string;
  /** 判定成员：被 dispatcher 以子进程调用的既有 CLI（仓库相对路径，`scripts/check/` 下）。 */
  probe: string;
  /** 真源（派生产物的上游）。 */
  source: string;
  /** 派生面（被判定的产物 / 闭包）。 */
  derived: string;
  /**
   * 分上下文的 probe argv（**不含**解释器与脚本路径；由 dispatcher 前置 `bun <probe>`）。
   * `staged` = 提交暂存档（如 i18n 生成物的 `--cached`）；`full` = 全量档。
   * 两档同值者照抄（多数族无「只判暂存集」语义）；**空数组 = 裸调用**（成员无任何旗标形态，
   * 如 value-domain-schema 的判定即默认档）。
   */
  argvByContext: Readonly<Record<DerivedContext, readonly string[]>>;
  /** 棘轮基线（仓库相对路径；无基线者 null）。 */
  baseline: string | null;
  /** 执行档（gate stage）。 */
  stage: DerivedStage;
  /** 判据面完全落在 namespace 树（平台档跳过；同 `Gate.nsOwnedContent`）。 */
  nsOwnedContent?: boolean;
  /** 该成员是否支持 `--self-test`（缺省 true；false MUST 在 note 注明原因，dispatcher 代跑自测时跳过）。 */
  selfTest?: boolean;
  /** 单行说明（输出面与 design.md 共用）。 */
  note: string;
}

/** 族清单（**唯一真源**；增删族只改此处 + 登记面各一条 argv）。 */
export const DERIVED_FAMILIES: readonly DerivedFamily[] = [
  {
    id: "i18n-key-type-integrity",
    probe: "scripts/check/check-i18n-key-type-integrity.ts",
    source: "各前端包 src/locales/zh-CN.json（i18n 字典真源）",
    derived:
      "同包 src/types/i18n-keys.ts（生成物，scripts/gen-module-i18n-keys.ts 唯一通道）",
    argvByContext: { staged: ["--cached", "--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "生成物导出键集 MUST 等于真源键集（缺/多余均阻断）",
  },
  {
    id: "plane-package-dev-face",
    probe: "scripts/check/check-plane-package-dev-face.ts",
    source:
      "Framework/frontend/config/vite/*.ts（tsc 真源）+ 各平面包 exports 面",
    derived:
      "Framework/frontend/config/vite/{index,singletons}.{js,d.ts}（入库 tsc 产物）+ 各包 dev-face 解析",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "dev 面 MUST 指源码；入库生成物 MUST 与 tsc 现产逐字一致",
  },
  {
    id: "module-block-assembly",
    probe: "scripts/check/check-module-block-assembly-drift.ts",
    source:
      "Pre-Proc/{ns}/Sources/Apps/Modules/<id>/module.json#blockAssembly（声明面）",
    derived:
      "Pre-Proc/{ns}/Sources/Apps/Modules/<id>/frontend/src/generated/block-assembly.ts（派生件，禁手改）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "push",
    nsOwnedContent: true,
    note: "派生件 MUST 等于声明面投影（已接入 import 后漂移/缺失 ⇒ 阻断；未迁移模块 = INFO）——`--ns` 缺省 = 全部挂载 ns",
  },
  {
    id: "app-shell-tabs",
    probe: "scripts/check/check-app-shell-tabs-drift.ts",
    source:
      "Pre-Proc/{ns}/Prototypes/Apps/<app>/app.json（模块清单/Tab 序/默认 Tab）",
    derived:
      "Pre-Proc/{ns}/Prototypes/Apps/<app>/llm-tsx/generated/app-tabs.ts（派生件，禁手改）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "push",
    nsOwnedContent: true,
    note: "App 壳 Tab 表（TAB_IDS/MODULE_TABS/DEFAULT_TAB）MUST 派生自 app.json（已接入后漂移/缺失 ⇒ 阻断；未迁移 App = INFO）——`--ns` 缺省 = 全部挂载 ns",
  },
  {
    id: "module-dist-freshness",
    probe: "scripts/check/check-module-dist-freshness.ts",
    source: "Pre-Proc/{ns}/.../frontend/src（模块源码）",
    derived: "Pre-Proc/{ns}/.../frontend/dist（gitignored 独立部署产物）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "push",
    nsOwnedContent: true,
    note: "dist 存在且 mtime 早于 src ⇒ STALE（独立部署前须重建）",
  },
  {
    id: "generated-artifact-freshness",
    probe: "scripts/check/check-generated-artifact-freshness.ts",
    source: "Pre-Proc/{ns}/Apps/{app}/{app}.app.json（现行模块/块集）",
    derived:
      "Pre-Proc/{ns}/Apps/{app}/{flow-plan.json,eval-report.json,pipeline_manifest.json}（AppAgent / alioth-compose 流水线产物）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline:
      "scripts/check/baselines/generated-artifact-freshness.baseline.json",
    stage: "push",
    nsOwnedContent: true,
    note: "流水线产物检出对已退役 id 的引用与陈旧 prototype（只减不增棘轮）",
  },
  {
    id: "dev-freshness-closure",
    probe: "scripts/check/check-dev-freshness-closure.ts",
    source: "scripts/dev/dev-gateway.sh 的构建输入（rust-tree 清单）",
    derived: "dev 后端二进制的启动期新鲜度闭包（WATCH_DIRS 收敛域）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "check-all",
    note: "启动判据 ⊇ 必需树 ∧ WATCH_DIRS 覆盖判据清单（两侧同批）",
  },
  {
    id: "value-domain-schema",
    probe: "scripts/check/check-value-domain-schema.ts",
    source:
      "isahl_meta.meta_fields ∩ 实表列（生成器 scripts/gen-value-domain-schema.ts）",
    derived:
      "Framework/backend/dict/schema/value-domain-columns.tsv（include_str! 编译期输入）",
    argvByContext: { staged: [], full: [] },
    baseline: null,
    stage: "commit",
    note: "逐字节一致性；DB 依赖由 gate needsDb 承接（生成器无法运行 ⇒ 跳过并告警；漂移/缺失 ⇒ 红）",
  },
  {
    id: "check-derived-copies",
    probe: "scripts/check/check-derived-copies.ts",
    source: "平台单源（scripts/lib/derived-copies.ts 登记的各派生族单源面）",
    derived:
      "各 ns / Deploy 目标侧副本（逐字节 / seed-tree / prototype-pair 认证）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "覆盖审计 + 逐字节认证；平台/ns 双档位由脚本自判（平台档遍历全部挂载 ns）",
  },
  {
    id: "stale-embedded-css",
    probe: "scripts/check/check-stale-embedded-css.mjs",
    source:
      ".agents/skills/alioth-design/references/prototype-base.css（框架快照）",
    derived: "各 ns 原型内嵌 prototype-base.css 快照",
    argvByContext: { staged: ["--ci-gate"], full: ["--ci-gate"] },
    baseline: null,
    stage: "push",
    nsOwnedContent: true,
    selfTest: false,
    note: "阻断档 = --ci-gate（修正：原注册 argv 缺该旗标致 blocking 声明与实效不符，恒 rc=0）；无自测（legacy .mjs 纯扫描，判定面为现场文件遍历）",
  },
  {
    id: "ns-derived-consistency",
    probe: "scripts/check/check-ns-derived-consistency.ts",
    source:
      "scripts/gateway/instantiate-ns-host.ts（组合根实例化模板：Cargo.toml / build.rs / Cargo.lock 模板锁）",
    derived: "Pre-Proc/{ns}/gateway-host/ 派生物（各 ns 仓）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "push",
    note: "平台档 = 全部挂载 ns smoke；ns 仓档 = 本仓 ns；未实例化面显式跳过",
  },
  {
    id: "openapi-doc-drift",
    probe: "scripts/check/check-openapi-doc-drift.ts",
    source: "生成器脚本 scripts/lib/openapi-doc.ts（与 CLI 同 import）",
    derived: "Pre-Proc/{ns}/openapi.json（发布包文档产物）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "产物缺失/陈旧/重生成失败 ⇒ 红（fail-closed）；ns 面双档位由 gate-kit.nsScopes 枚举",
  },
  {
    id: "openspec-index-drift",
    probe: "scripts/check/check-openspec-index-drift.ts",
    source: "openspec/specs/**（capability 名 + ## Purpose 首行 + 可达面）",
    derived: "openspec/INDEX.md（生成物）",
    argvByContext: { staged: ["--cached", "--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "双登记：commit 档用 --cached（暂存树快照基准）+ push 档全量",
  },
  {
    id: "prototype-drift",
    probe: "scripts/check/check-prototype-drift.ts",
    source:
      "Pre-Proc/{ns}/Prototypes/{Kind}/{unit}/ 当前版本原型（静态 <label> 表单字段）",
    derived: "同单元生产前端 tsx 字段集（union）",
    argvByContext: {
      staged: ["--batch", "--staged", "--fail"],
      full: ["--batch", "--fail"],
    },
    baseline: null,
    stage: "commit",
    nsOwnedContent: true,
    note: "批量化（原 notify 提醒 → 硬门禁）：逐单元比对；comparable=false（llm-tsx ESM 原型）显式跳过不阻断",
  },
  {
    id: "ns-deploy-frontend-drift",
    probe: "scripts/check/check-ns-deploy-frontend-drift.ts",
    source:
      "平台单源 scripts/gateway/{serve-frontend.sh,frontend-server.js}（manifest scripts/lib/derived-copies.ts）",
    derived:
      "各 ns / Deploy 目标侧副本 Deploy/{ns}/scripts/（cp 语义逐字节；副本缺失打印原因跳过）",
    argvByContext: { staged: ["--fail"], full: ["--fail"] },
    baseline: null,
    stage: "commit",
    note: "平台源零 ns 槽位字面量 + ns 副本逐字节一致；双档位平台/ns 由脚本自判",
  },
  {
    id: "skill-adapter-sync",
    probe: "scripts/check/check-skill-adapter-sync.ts",
    source: ".agents/skills/**/SKILL.md（sha256 真源，source.skill 指向）",
    derived:
      "Meta/backend/app-agent/skill-adapters/*.yaml 的 source.content_sha256 记录",
    argvByContext: { staged: [], full: [] },
    baseline: null,
    stage: "push",
    note: "warn 档（gate blocking:false；合并面 rt-gate 直调强制）；缺哈希/缺 SKILL = warn，漂移 rc1 恒（不依赖 --fail）",
  },
  {
    id: "context-fields-sync",
    probe: "scripts/check/check-context-fields.ts",
    source:
      "DB isahl 元数据（generate-context-fields.ts 读取面：meta_collections / meta_fields / 实表列）",
    derived: "Framework/backend/approval/src/context_meta.rs（整文件生成物）",
    argvByContext: { staged: [], full: [] },
    baseline: null,
    stage: "push",
    note: "DB 依赖（gate needsDb 承接）；通过态缓存（键 = DB 面摘要 + 生成器哈希）；漂移 ⇒ 重建并 rc1（恒）",
  },
  {
    id: "ns-registry-fresh",
    probe: "scripts/check/check-ns-registry-fresh.ts",
    source:
      "各 ns 仓 .alioth/platform.conf（FE_PORT/BE_PORT/SSO_PORT/SSO_MODE 声明正本；经 Pre-Proc/{ns} 符号链接定位）",
    derived: "Deploy/ports.conf + Deploy/sso-mode.conf（派生缓存；行序正本 = 前者，模块端口带共同序）",
    argvByContext: { staged: [], full: [] },
    baseline: null,
    stage: "push",
    note: "判据 = refresh-ns-registry.sh --check 逐字节比对；与 check-ports-conf / check-sso-mode-conf 的缓存⇔声明面 error 判据同源（本族只做台账收敛，自测由 bash lib 自检承接 ⇒ selfTest:false）",
    selfTest: false,
  },
];

/** 仓库根（模块位置推导：`scripts/lib/` → 仓库根；dispatcher 与 integrity 默认值共用）。 */
export const DERIVED_ROOT = resolve(import.meta.dirname, "..", "..");

/**
 * 解析 `--family` 取值 → 族清单（`all` = 全族；具名 id 不存在 ⇒ 抛错，fail-closed）。
 * 未知 id 静默回退「全族/空集」都会让调用方以为跑过 —— 故抛出。
 */
export function resolveFamilies(family: string): DerivedFamily[] {
  if (family === "all") return [...DERIVED_FAMILIES];
  const hit = DERIVED_FAMILIES.find((f) => f.id === family);
  if (hit === undefined) {
    throw new Error(
      `derived-freshness: 未知族 id "${family}"（可选：all | ${DERIVED_FAMILIES.map((f) => f.id).join(" | ")}）`,
    );
  }
  return [hit];
}

/** 族清单的结构完整性（id 唯一 / probe 存在且落在 scripts/check / 两档 argv 非空）。 */
export function integrityProblems(root: string = DERIVED_ROOT): string[] {
  const problems: string[] = [];
  const seen = new Set<string>();
  for (const fam of DERIVED_FAMILIES) {
    if (!/^[a-z0-9-]+$/.test(fam.id))
      problems.push(`族 id 非法（须 kebab-case）：${fam.id}`);
    if (seen.has(fam.id)) problems.push(`族 id 重复：${fam.id}`);
    seen.add(fam.id);
    if (!fam.probe.startsWith("scripts/check/"))
      problems.push(
        `${fam.id}: probe MUST 落在 scripts/check/（${fam.probe}）`,
      );
    if (!existsSync(join(root, fam.probe)))
      problems.push(`${fam.id}: probe 不存在（${fam.probe}）`);
    // argv 两档 MUST 存在（类型强制）且为数组；**空数组 = 裸调用**（无旗标形态的成员）。
    for (const mode of ["staged", "full"] as const) {
      if (!Array.isArray(fam.argvByContext[mode]))
        problems.push(`${fam.id}: ${mode} 档 argv 缺失（MUST 为数组）`);
    }
    if (fam.selfTest === false && fam.note.trim().length === 0) {
      problems.push(`${fam.id}: selfTest=false MUST 在 note 注明原因`);
    }
    if (
      fam.stage !== "commit" &&
      fam.stage !== "push" &&
      fam.stage !== "check-all"
    ) {
      problems.push(`${fam.id}: 执行档非法（${fam.stage}）`);
    }
    if (fam.baseline !== null && !existsSync(join(root, fam.baseline))) {
      problems.push(`${fam.id}: 登记的基线不存在（${fam.baseline}）`);
    }
  }
  return problems;
}

/** 子进程执行结果（dispatcher 聚合面）。 */
export interface FamilyRun {
  id: string;
  probe: string;
  rc: number;
  stdout: string;
  stderr: string;
}

/** 聚合语义（纯函数，供 dispatcher 报告面与自测共用）：失败数 / 最差 rc。 */
export function aggregate(runs: readonly FamilyRun[]): {
  passed: number;
  failed: number;
  worst: number;
} {
  let passed = 0;
  let failed = 0;
  let worst = 0;
  for (const run of runs) {
    if (run.rc === 0) passed += 1;
    else {
      failed += 1;
      if (run.rc > worst) worst = run.rc;
    }
  }
  return { passed, failed, worst };
}
