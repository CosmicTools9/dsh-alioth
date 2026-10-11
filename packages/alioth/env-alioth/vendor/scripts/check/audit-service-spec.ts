#!/usr/bin/env bun
/**
 * audit-service-spec.ts — Service 后端架构规约审计（F1–F7）
 *
 * 本脚本是规则集与用法的**唯一正本**；技能文档
 * `.agents/skills/alioth-service/references/phase-4-compliance-audit.md` 与之逐条一致。
 *
 * 规则:
 *   F1  backend/Cargo.toml 存在
 *   F2  handlers/mod.rs 存在，或 lib.rs 委托外部 crate（聚合服务壳模式）
 *   F3  services/ 层无直接 SQL（sqlx::query|execute|fetch|begin）
 *   F4  handlers/ 不引用 models 实体（DTO Request/Response/Query/Record/Row/Dto/Payload/Event 除外）
 *   F5  自定义错误类型使用 thiserror/anyhow（无自定义错误类型 → OK）
 *   F6  业务代码无 println!（src/bin/ 入口输出除外）
 *   F7  handlers/ 无直接 SQL（分层：handlers → services → repositories）
 *   F8  声明↔实况：service.json `hasBackend: true` 而本 ns 无 backend/src → 违规
 *       （跨 ns 复用共享内核 crate 的形态 MUST 声明 `hasBackend: false` + `backendCrate`
 *        记录复用来源；此前该形态仅在「零发现」分支才被发现，同 ns 有其他单元时静默通过）
 *
 * 服务发现：`Pre-Proc/{ns}/Sources/Apps/Services` 优先、扁平 `Sources/Services` 回退
 * （布局契约见 scripts/lib/preproc-layout.mjs）。**零发现不得空通过**：候选根下存在
 * service.json 却未发现可审计单元（缺 backend/src）→ 退出码 1。
 *
 * Usage: bun scripts/check/audit-service-spec.ts [--ns NS] [--factor FACTOR]
 * Exit code: 0 = 全部通过（或该范围内确实无 Service 单元）; 1 = 存在违规或发现失败
 */
import { readdirSync, existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { gatewaySourcesKindDir } from "../lib/preproc-layout.mjs";

import { defineCheck } from './lib/check-interface';

const META_CHANNEL = '__meta';

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
/** 判定面注入的取值旗标（`--ns` / `--factor`，均缺省 = 不限） */
let ARGS: Record<string, string> = {};

const ROOT = resolve(import.meta.dirname, "../..");
const FDIR = join(ROOT, "Pre-Proc");

interface Service { ns: string; id: string; dir: string; }

/** 候选 Service 根目录：Apps 布局优先、扁平回退（同一 ns 只取实际存在的那一个），
 *  **另含 OpenActivity 层根 `Sources/Open/Services`** —— 层级契约 `preproc-layout.mjs` 定
 *  `Sources/Apps` = Gateway 单元、`Sources/Open` = OpenActivity 单元；缺该根则 Open 层服务
 *  不被枚举 = 审计假绿（2026-10-04 实测：`Sources/Open/Services/system-settings` 整节缺失）。 */
function candidateRoots(nsFilter?: string): Array<{ ns: string; root: string }> {
  const out: Array<{ ns: string; root: string }> = [];
  for (const ns of readdirSync(FDIR).sort()) {
    if (nsFilter && ns !== nsFilter) continue;
    if (!existsSync(join(FDIR, ns, "Sources"))) continue;
    const appsRoot = gatewaySourcesKindDir(ROOT, ns, "Services");
    if (existsSync(appsRoot)) out.push({ ns, root: appsRoot });
    const openRoot = join(FDIR, ns, "Sources", "Open", "Services");
    if (existsSync(openRoot)) out.push({ ns, root: openRoot });
  }
  return out;
}

function discoverServices(nsFilter?: string, factorFilter?: string): Service[] {
  const found: Service[] = [];
  for (const { ns, root } of candidateRoots(nsFilter)) {
    for (const unit of readdirSync(root, { withFileTypes: true })) {
      if (!unit.isDirectory()) continue;
      if (factorFilter && unit.name !== factorFilter) continue;
      const dir = join(root, unit.name);
      if (existsSync(join(dir, "service.json")) && existsSync(join(dir, "backend", "src"))) {
        found.push({ ns, id: unit.name, dir });
      }
    }
  }
  return found;
}

/** 零发现归因：候选根下存在 service.json（说明有单元但缺 backend/src 或布局不符） */
function undeclaredServiceJson(nsFilter?: string, factorFilter?: string): string[] {
  const hits: string[] = [];
  for (const { root } of candidateRoots(nsFilter)) {
    for (const unit of readdirSync(root, { withFileTypes: true })) {
      if (!unit.isDirectory()) continue;
      if (factorFilter && unit.name !== factorFilter) continue;
      const sj = join(root, unit.name, "service.json");
      if (existsSync(sj)) hits.push(sj.replace(`${ROOT}/`, ""));
    }
  }
  return hits;
}

/** 声明↔实况：service.json 声明 hasBackend=true 但本 ns 无 backend/src。
 *  跨 ns 复用共享内核 crate 的形态 MUST 声明 hasBackend=false + backendCrate 记录复用来源。 */
function declaredBackendWithoutDir(nsFilter?: string, factorFilter?: string): string[] {
  const hits: string[] = [];
  for (const { root } of candidateRoots(nsFilter)) {
    for (const unit of readdirSync(root, { withFileTypes: true })) {
      if (!unit.isDirectory()) continue;
      if (factorFilter && unit.name !== factorFilter) continue;
      const dir = join(root, unit.name);
      const sj = join(dir, "service.json");
      if (!existsSync(sj) || existsSync(join(dir, "backend", "src"))) continue;
      let declared = false;
      try {
        declared = JSON.parse(readFileSync(sj, "utf-8"))?.hasBackend === true;
      } catch {
        continue; // JSON 合法性由 check-config-json.mjs 负责
      }
      if (declared) hits.push(sj.replace(`${ROOT}/`, ""));
    }
  }
  return hits;
}

function grep(pat: string, dir: string, exclude?: string): string[] {
  const r = spawnSync("grep", ["-rnE", pat, dir], { encoding: "utf-8", timeout: 30000 });
  if (r.status !== 0) return [];
  let lines = r.stdout.trim().split("\n").filter(Boolean);
  if (exclude) lines = lines.filter((l) => !new RegExp(exclude).test(l));
  return lines;
}

interface AuditServiceSpecOutcome {
  out: string[];
  err: string[];
  rc: number;
}

/** 判定面（Service 规格审计：目录契约/backend 存在性/交付面）；输出经 SINK 记录，报告面回放。 */
export function auditServiceSpecState(nsFilter: string | null, factorFilter: string | null): AuditServiceSpecOutcome {
  const out: string[] = [];
  const err: string[] = [];
  const saved = SINK;
  SINK = { out: (l) => out.push(l), err: (l) => err.push(l) };
  ARGS = {};
  if (nsFilter !== null) ARGS.ns = nsFilter;
  if (factorFilter !== null) ARGS.factor = factorFilter;
  const args = ARGS;
  try {
const services = discoverServices(args.ns, args.factor);

if (services.length === 0) {
  const known = undeclaredServiceJson(args.ns, args.factor);
  if (known.length > 0) {
    SINK.err(
      `audit-service-spec: FAIL — 发现 0 个可审计单元，但候选根下存在 ${known.length} 个 service.json（缺 backend/src 或布局不符）:`,
    );
    for (const p of known.slice(0, 10)) SINK.err(`  - ${p}`);
    SINK.err(`  候选根（Apps 优先/扁平回退）: ${candidateRoots(args.ns).map((c) => c.root.replace(`${ROOT}/`, "")).join(", ") || "（无）"}`);
    exit(1);
  }
  SINK.out(`audit-service-spec: 无可审计 Service 单元（ns=${args.ns ?? "*"} factor=${args.factor ?? "*"}）`);
  exit(0);
}

let allPass = true;
const phantomBackends = declaredBackendWithoutDir(args.ns, args.factor);
if (phantomBackends.length > 0) {
  SINK.err(
    `audit-service-spec: FAIL \u2014 ${phantomBackends.length} 个 Service 声明 hasBackend=true 但本 ns 无 backend/src（跨 ns 复用内核 crate 的形态 MUST 声明 hasBackend=false + backendCrate 记录复用来源）:`,
  );
  for (const p of phantomBackends) SINK.err(`  - ${p}`);
  allPass = false;
}
for (const svc of services) {
  SINK.out(`\n== ${svc.ns}/${svc.id} ==`);
  const checks: [string, string, boolean, string][] = [
    ["F1", "Cargo.toml", existsSync(join(svc.dir, "backend", "Cargo.toml")),
      existsSync(join(svc.dir, "backend", "Cargo.toml")) ? "OK" : "MISSING"],
    // F2：本地 handlers/mod.rs 或 lib.rs 委托外部 crate（聚合服务 isahl-db/measurement/status/approval/authority 模式）
    ["F2", "handlers/mod.rs", existsSync(join(svc.dir, "backend", "src", "handlers", "mod.rs"))
        || grep("\\.configure\\(|::\\w+\\(cfg|::register_service_routes", join(svc.dir, "backend", "src", "lib.rs")).length > 0,
      existsSync(join(svc.dir, "backend", "src", "handlers", "mod.rs"))
        ? "OK"
        : grep("\\.configure\\(|::\\w+\\(cfg|::register_service_routes", join(svc.dir, "backend", "src", "lib.rs")).length > 0
          ? "OK (lib.rs 委托外部 crate)"
          : "MISSING"],
    // F3：services 层禁止直接执行 SQL（依赖注入类型签名 sqlx::PgPool/Transaction 合规）
    ["F3", "no sqlx queries in services", (h => h.length === 0)(grep("sqlx::(query|execute|fetch|begin)", join(svc.dir, "backend", "src", "services"))),
      (h => h.length === 0 ? "OK" : `sqlx queries in ${h.length} lines`)(grep("sqlx::(query|execute|fetch|begin)", join(svc.dir, "backend", "src", "services")))],
    // F4：handlers 禁止引用 models 实体（DTO Request/Response 与跨 crate 业务模型除外——
    // DTO 是 BACKEND_FRAMEWORK crud 样板的标准输入形态）
    ["F4", "no entity models in handlers", (h => h.length === 0)(grep("use.*models::", join(svc.dir, "backend", "src", "handlers"), "(Request|Response|Query|Record|Row|Dto|Payload|Event|\\{$)" )),
      (h => h.length === 0 ? "OK" : `entity models in ${h.length} handlers`)(grep("use.*models::", join(svc.dir, "backend", "src", "handlers"), "(Request|Response|Query|Record|Row|Dto|Payload|Event|\\{$)"))],
    // F5：存在自定义错误类型时必须用 thiserror/anyhow（CODE_STYLE_SPEC §错误处理）；
    // 无自定义错误（统一 common::AliothError）→ OK
    ["F5", "custom errors use thiserror/anyhow", (h => h.length === 0 || grep("thiserror|anyhow", join(svc.dir, "backend", "src")).length > 0)(grep("enum \\w*(Error|Err)\\b|struct \\w*Error\\b", join(svc.dir, "backend", "src"))),
      (h => h.length === 0 ? "OK" : "custom errors require thiserror/anyhow")(grep("enum \\w*(Error|Err)\\b|struct \\w*Error\\b", join(svc.dir, "backend", "src")))],
    // F6：业务代码禁 println（CLI 工具 src/bin/ 入口输出合规——CODE_STYLE CLI/main 例外）
    ["F6", "no println", (h => h.length === 0)(grep("println!", join(svc.dir, "backend", "src"), "src/bin/")),
      (h => h.length === 0 ? "OK" : `println! in ${h.length} lines`)(grep("println!", join(svc.dir, "backend", "src"), "src/bin/"))],
    // F7：handlers 禁止直接执行 SQL（分层：handlers → services → repositories；审计缺口补全）
    ["F7", "no sqlx queries in handlers", (h => h.length === 0)(grep("sqlx::(query|execute|fetch|begin)", join(svc.dir, "backend", "src", "handlers"))),
      (h => h.length === 0 ? "OK" : `sqlx queries in ${h.length} lines`)(grep("sqlx::(query|execute|fetch|begin)", join(svc.dir, "backend", "src", "handlers")))],
  ];
  for (const [code, name, pass, detail] of checks) {
    SINK.out(`  ${pass ? "\u2713" : "\u2717"} ${code}: ${name} \u2014 ${detail}`);
    if (!pass) allPass = false;
  }
}
exit(allPass ? 0 : 1);
  } catch (e) {
    if (!(e instanceof ExitSignal)) throw e;
    return { out, err, rc: e.rc };
  } finally {
    SINK = saved;
  }
  return { out, err, rc: 0 };
}

const gate = defineCheck({
  id: 'audit-service-spec',
  label: 'audit-service-spec',
  valueFlags: ['--ns', '--factor'],
  predicate: () => [],
  report: (ctx, _res, io) => {
    const s = auditServiceSpecState(
      ctx.valueFlags.get('--ns')?.[0] ?? null,
      ctx.valueFlags.get('--factor')?.[0] ?? null,
    );
    for (const line of s.out) io.out(`${line}\n`);
    for (const line of s.err) io.err(`${line}\n`);
    return s.rc;
  },
  selfTest() {
    // 判据自证：同参两次求值等价（ARGS/SINK 复位生效，枚举/判定幂等）
    const a = auditServiceSpecState(null, null);
    const b = auditServiceSpecState(null, null);
    if (a.rc !== b.rc || a.out.join('') !== b.out.join('') || a.err.join('') !== b.err.join('')) {
      process.stderr.write(`❌ audit-service-spec self-test 失败：两次求值不等价（rc=${a.rc}/${b.rc}）\n`);
      return 1;
    }
    process.stdout.write(`✅ audit-service-spec self-test 通过（rc=${a.rc}）\n`);
    return 0;
  },
});

if (import.meta.main) process.exit(await gate.main());