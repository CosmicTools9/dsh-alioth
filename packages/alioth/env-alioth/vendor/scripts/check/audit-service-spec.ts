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
 *
 * 服务发现：`Pre-Proc/{ns}/Sources/Apps/Services` 优先、扁平 `Sources/Services` 回退
 * （布局契约见 scripts/lib/preproc-layout.mjs）。**零发现不得空通过**：候选根下存在
 * service.json 却未发现可审计单元（缺 backend/src）→ 退出码 1。
 *
 * Usage: bun scripts/check/audit-service-spec.ts [--ns NS] [--factor FACTOR]
 * Exit code: 0 = 全部通过（或该范围内确实无 Service 单元）; 1 = 存在违规或发现失败
 */
import { readdirSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { gatewaySourcesKindDir } from "../lib/preproc-layout.mjs";

const ROOT = resolve(import.meta.dirname, "../..");
const FDIR = join(ROOT, "Pre-Proc");

interface Service { ns: string; id: string; dir: string; }

function parseArgs(argv: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith("--") && i + 1 < argv.length && !argv[i + 1].startsWith("--")) {
      out[argv[i].slice(2)] = argv[i + 1];
      i++;
    }
  }
  return out;
}

/** 候选 Service 根目录：Apps 布局优先、扁平回退（同一 ns 只取实际存在的那一个） */
function candidateRoots(nsFilter?: string): Array<{ ns: string; root: string }> {
  const out: Array<{ ns: string; root: string }> = [];
  for (const ns of readdirSync(FDIR).sort()) {
    if (nsFilter && ns !== nsFilter) continue;
    if (!existsSync(join(FDIR, ns, "Sources"))) continue;
    const root = gatewaySourcesKindDir(ROOT, ns, "Services");
    if (existsSync(root)) out.push({ ns, root });
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

function grep(pat: string, dir: string, exclude?: string): string[] {
  const r = spawnSync("grep", ["-rnE", pat, dir], { encoding: "utf-8", timeout: 30000 });
  if (r.status !== 0) return [];
  let lines = r.stdout.trim().split("\n").filter(Boolean);
  if (exclude) lines = lines.filter((l) => !new RegExp(exclude).test(l));
  return lines;
}

const args = parseArgs(process.argv.slice(2));
const services = discoverServices(args.ns, args.factor);

if (services.length === 0) {
  const known = undeclaredServiceJson(args.ns, args.factor);
  if (known.length > 0) {
    console.error(
      `audit-service-spec: FAIL — 发现 0 个可审计单元，但候选根下存在 ${known.length} 个 service.json（缺 backend/src 或布局不符）:`,
    );
    for (const p of known.slice(0, 10)) console.error(`  - ${p}`);
    console.error(`  候选根（Apps 优先/扁平回退）: ${candidateRoots(args.ns).map((c) => c.root.replace(`${ROOT}/`, "")).join(", ") || "（无）"}`);
    process.exit(1);
  }
  console.log(`audit-service-spec: 无可审计 Service 单元（ns=${args.ns ?? "*"} factor=${args.factor ?? "*"}）`);
  process.exit(0);
}

let allPass = true;
for (const svc of services) {
  console.log(`\n== ${svc.ns}/${svc.id} ==`);
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
    console.log(`  ${pass ? "\u2713" : "\u2717"} ${code}: ${name} \u2014 ${detail}`);
    if (!pass) allPass = false;
  }
}
process.exit(allPass ? 0 : 1);
