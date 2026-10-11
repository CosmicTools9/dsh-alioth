#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * check-prototype-render.ts — 原型产物**运行期渲染断言**（真实浏览器）
 *
 * 为什么需要: 2026-09-17 `ct-bv-local` 战役实测——原型产物可通过 `evaluate-prototype-reference`
 * 静态评分（96.8）、`audit-css-framework`（0 错）、`check-visual-verify` 冒烟（90/100）与
 * `check-e2e-app`（47/47，但只驱动默认模块），却在点击对应 Tab 时抛 `ReferenceError`
 * 致使 React 整树卸载（空白页）；另有 mock↔列键错配渲染出 `undefined`/`—`。
 * 本文脚本是该漏洞类的**运行期判据**：逐产物、逐导航项点击并断言。
 *
 * 判据（对每个目标）:
 *   R1 `#root` 有子节点（未卸载 ⇒ 页面崩/挂载前即抛错）
 *   R2 `#root` 已挂载且 `#boot-skeleton` 已移除/隐藏（仅隐藏也算）
 *   R3 `innerText` 长度 ≥ `MIN_TEXT`（非空壳）
 *   R4 `innerText` 不含 `undefined` / `NaN`
 *   R5 boot 占位文案（`该模块原型未生成`）未展示
 *   R6 无未捕获 `error` / `unhandledrejection`（含跨域壳文案 `Script error.`）
 * 覆盖深度:
 *   Block  — 基础渲染
 *   Module — 逐个 `module.json.blockAssembly.blocks[].label` 点击后复断言
 *   App    — 逐个 `app.json.config.modules[].name`（模块显示名）Tab 点击后复断言
 *
 * 2026-10-08 修复（本门禁此前是**静默绿**的死判据）：原实现把 `snapshot()` 当作
 * `{rc,len,bad,err}` 对象读——而 ego 0.5.1 实测 `snapshot()` 返回 `{content, refs}` ⇒ 四个判据
 * 全部恒真（崩溃产物照报「R1-R4 全部通过」；同期 `visual-verify capture` 正确失败）。现在：
 *   · 事实面 = 页内 `js()` 探针（`#root` 子节点数 / innerText / boot 骨架 / 未捕获错误），
 *     不再消费 `snapshot()` 的未声明形态；
 *   · 判定面 = 宿主侧 `probeFacts`/`baseProblems`/`clickProblems`（**唯一实现**，自测可直接喂夹具）；
 *   · 断言前**等待 React 挂载**（轮询 `#root` 子节点；未捕获异常即提前收敛，不等满窗口）；
 *   · 未捕获错误（含 `Script error.`）计硬错；
 *   · 棘轮基线 `scripts/check/.prototype-render-baseline.txt`（存量豁免，只拦新增；`--update-baseline` 登记）。
 *
 * 用法:
 *   bun scripts/check/check-prototype-render.ts --ns Cosmic-Tools [--fail] [--json]
 *   bun scripts/check/check-prototype-render.ts Pre-Proc/X/Prototypes/Blocks/y/b-v3.html [...]
 *   [--profile <id>]       指定 ego profile（等价 `EGO_PROFILE_ID`）→ task space 的 cookie jar（登录态）
 *   [--update-baseline]    以本仓当前产物为存量重写棘轮基线
 *   [--self-test]          判据自证：判定面夹具（必崩/空壳/健康/undefined/探针失败）+ ego 可用时的端到端必崩夹具
 *
 * 环境: 需要 `ego-browser`（缺失 ⇒ SKIP + 警告，exit 0；开发机上的技能强制步骤为权威）。
 * 清理: 单一 task space，结束时 `completeTaskSpace` + `closeTaskSpace`（ego-browser 技能铁律）。
 */
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, relative, resolve } from "node:path";
import {
  EGO_SPACE_PRELUDE,
  egoCreateSpaceExpr,
  resolveEgoProfile,
} from "../lib/ego-space";
import { egoCompatPrelude, egoSpawnArgs } from "../lib/ego-cli";
import { repoRootOf } from "../lib/repo-role";
import {
  defineCheck,
  substrateRoot,
  type Violation,
} from "./lib/check-interface";
import * as baseline from "./lib/baseline";

const REPO_ROOT = resolve(import.meta.dir, "../..");

/**
 * `--changed` 的 git 视界 = **调用方所在仓**（不是 REPO_ROOT）：ns 仓 push 判定的是该 ns 仓内的
 * 原型改动（内容经 `Pre-Proc/{ns}` 挂载可见，但 `git status/diff` 必须问 ns 仓）；平台仓调用时
 * 两者同仓，行为同旧。旧实现恒用 REPO_ROOT（`import.meta.dir` 经 realpath 落在平台仓）⇒ 从 ns 仓
 * 调用时该 flag 永远匹配不到任何改动 ⇒ plan 空 ⇒ 静默跳过（同一门禁的另一处死判据）。
 */
const CHANGE_ROOT = repoRootOf(process.cwd()) ?? REPO_ROOT;

/** 仓库相对路径（`Pre-Proc/**` 口径；仓外路径（自测夹具等）原样保留——显式目标不参与基线豁免）。 */
function repoRel(p: string): string {
  const rel = relative(REPO_ROOT, p);
  return rel.startsWith("..") ? p : rel;
}

/** 棘轮基线（存量豁免；只拦新增）。基线文件路径同时被 `--update-baseline`（基座）与报告面消费。 */
const BASELINE_FILE = "scripts/check/.prototype-render-baseline.txt";

/**
 * R3 文本下限（非空壳）。阈值依据（2026-10-08 实测，见 change 记录）：
 *   · 合法块产物 `body.innerText` ≈154–169（如 Blocks/account-futures 154）；
 *   · 合法 Module/App ≈863–1469；
 *   · 崩溃产物 `#root` 未挂载 ⇒ 文本 ≈0（R1/R2/R6 才是硬信号，本阈值只兜底「挂载了但空」）。
 */
const MIN_TEXT = 50;

/** boot 占位文案（`#boot-skeleton` 之外的另一处「未生成」占位）。 */
const PLACEHOLDER_TEXT = "该模块原型未生成";

/** lease CLI 绝对路径（cwd 无关）。 */
const LEASE_CLI = join(REPO_ROOT, "scripts/ego/lease.ts");

/** 登记/刷新本次运行的 task space lease（含运行身份）；失败不影响门禁。 */
function leaseCli(cliArgs: string[]): void {
  try {
    spawnSync("bun", [LEASE_CLI, ...cliArgs], {
      timeout: 5000,
      stdio: "ignore",
    });
  } catch {
    /* lease 失败不影响主流程 */
  }
}

let SINK: { out: (l: string) => void; err: (l: string) => void } = {
  out: (l) => process.stdout.write(`${l}\n`),
  err: (l) => process.stderr.write(`${l}\n`),
};

type Step = {
  kind: "block" | "module" | "app";
  file: string;
  url: string;
  clicks: string[];
};
type Result = {
  file: string;
  kind: string;
  ok: boolean;
  detail: string;
  clicksTotal?: number;
  clicksAttempted?: number;
  clicksSkipped?: number;
};

function readJson(path: string): unknown {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

function newestArtifact(
  dirAbs: string,
  prefix: "b" | "m" | "a",
): string | null {
  const g = new Bun.Glob(`${prefix}-v*.html`);
  const found = [...g.scanSync({ cwd: dirAbs, onlyFiles: true })];
  if (found.length === 0) return null;
  found.sort((x, y) => {
    const nx = Number(/(\d+)/.exec(x)?.[1] ?? 0);
    const ny = Number(/(\d+)/.exec(y)?.[1] ?? 0);
    return nx - ny;
  });
  return join(dirAbs, found[found.length - 1]);
}

function moduleBlockLabels(ns: string, moduleId: string): string[] {
  const mod = readJson(
    join(
      REPO_ROOT,
      `Pre-Proc/${ns}/Sources/Apps/Modules/${moduleId}/module.json`,
    ),
  ) as {
    blockAssembly?: { blocks?: Array<{ label?: string }> };
  } | null;
  const blocks = mod?.blockAssembly?.blocks;
  if (!Array.isArray(blocks)) return [];
  return blocks
    .map((b) => b?.label)
    .filter((l): l is string => typeof l === "string" && l.length > 0);
}

function appTabLabels(ns: string, appId: string): string[] {
  const app = readJson(
    join(REPO_ROOT, `Pre-Proc/${ns}/Apps/${appId}/app.json`),
  ) as {
    config?: { modules?: unknown[] };
  } | null;
  const mods = app?.config?.modules;
  if (!Array.isArray(mods)) return [];
  const labels: string[] = [];
  for (const m of mods) {
    const id = typeof m === "string" ? m : (m as { id?: string })?.id;
    if (typeof id !== "string") continue;
    const mod = readJson(
      join(REPO_ROOT, `Pre-Proc/${ns}/Sources/Apps/Modules/${id}/module.json`),
    ) as {
      name?: string;
    } | null;
    if (typeof mod?.name === "string" && mod.name.length > 0)
      labels.push(mod.name);
  }
  return labels;
}

/** 列出目录下的直接子目录名（目录不存在或非目录 ⇒ 返回空，避免环境特例导致崩溃）。 */
function listSubdirs(rootAbs: string): string[] {
  if (!existsSync(rootAbs) || !statSync(rootAbs).isDirectory()) return [];
  return [...new Bun.Glob("*/").scanSync({ cwd: rootAbs, onlyFiles: false })]
    .filter((name) => statSync(join(rootAbs, name)).isDirectory())
    .map((name) => name.replace(/\/$/, ""));
}

function buildPlan(opts: {
  ns: string | null;
  explicit: readonly string[];
  changed: boolean;
}): Step[] {
  if (opts.explicit.length > 0) {
    return opts.explicit.map((f) => ({
      kind: "block" as const,
      file: f,
      url: `file://${resolve(REPO_ROOT, f)}`,
      clicks: [],
    }));
  }
  const nsList = opts.ns
    ? [opts.ns]
    : [
        ...new Bun.Glob("Pre-Proc/*").scanSync({
          cwd: REPO_ROOT,
          onlyFiles: false,
        }),
      ]
        .filter((p) => statSync(join(REPO_ROOT, p)).isDirectory())
        .map((p) => basename(p));
  const changed = opts.changed ? collectChangedPaths() : null;
  const isChanged = (artifactPath: string, ownerDir: string): boolean => {
    if (!changed) return true;
    const relArtifact = artifactPath.replace(`${REPO_ROOT}/`, "");
    const relDir = ownerDir.replace(`${REPO_ROOT}/`, "");
    for (const p of changed) {
      if (p === relArtifact || p.startsWith(`${relDir}/`)) return true;
    }
    return false;
  };
  const steps: Step[] = [];
  for (const ns of nsList) {
    const blocksRoot = join(REPO_ROOT, `Pre-Proc/${ns}/Prototypes/Blocks`);
    for (const id of listSubdirs(blocksRoot)) {
      const owner = join(blocksRoot, id);
      const f = newestArtifact(owner, "b");
      if (f && isChanged(f, owner))
        steps.push({ kind: "block", file: f, url: `file://${f}`, clicks: [] });
    }
    const modulesRoot = join(REPO_ROOT, `Pre-Proc/${ns}/Prototypes/Modules`);
    for (const id of listSubdirs(modulesRoot)) {
      const owner = join(modulesRoot, id);
      const f = newestArtifact(owner, "m");
      if (f && isChanged(f, owner))
        steps.push({
          kind: "module",
          file: f,
          url: `file://${f}`,
          clicks: moduleBlockLabels(ns, id),
        });
    }
    const appsRoot = join(REPO_ROOT, `Pre-Proc/${ns}/Prototypes/Apps`);
    for (const id of listSubdirs(appsRoot)) {
      const owner = join(appsRoot, id);
      const f = newestArtifact(owner, "a");
      if (f && isChanged(f, owner))
        steps.push({
          kind: "app",
          file: f,
          url: `file://${f}`,
          clicks: appTabLabels(ns, id),
        });
    }
  }
  return steps;
}

/** `--changed`：取工作树 + 最近一次提交涉及的原型相关路径（源或其产物），使 push 门禁只断言受影响产物。 */
function collectChangedPaths(): string[] {
  const run = (gitArgs: string[]): string[] => {
    const p = Bun.spawnSync(["git", ...gitArgs], { cwd: CHANGE_ROOT });
    return p.stdout
      .toString()
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
  };
  const paths = new Set<string>();
  for (const p of run(["status", "--porcelain"])) {
    const file = p.slice(3).trim();
    if (file.includes(" -> ")) paths.add(file.split(" -> ")[1]);
    else paths.add(file);
  }
  for (const p of run(["diff", "--name-only", "HEAD~1..HEAD"])) paths.add(p);
  return [...paths];
}

const BROWSER_SCRIPT = (
  planPath: string,
  outPath: string,
  spaceName: string,
  profile: string | undefined,
) => `
${egoCompatPrelude()}
${profile ? EGO_SPACE_PRELUDE : ""}// 静态 import 在此不可用：本段是**经 stdin 交给 ego 求值的 heredoc 脚本**（非模块文件），
// 且 ego 的 eval 上下文不提供 CJS require（Linux 线实测 ReferenceError: require is not defined）
// ⇒ 只能运行期取 fs（specifier 字面量、非 TS 源里的模块依赖）。
const fs = await import('node:fs');
const plan = JSON.parse(fs.readFileSync(${JSON.stringify(planPath)}, 'utf8'));
const outPath = ${JSON.stringify(outPath)};
const PLACEHOLDER = ${JSON.stringify(PLACEHOLDER_TEXT)};
const res = { results: [], fatal: null };
let task = null;
const guard = (p, ms, w) => { const { promise, reject } = Promise.withResolvers(); const t = setTimeout(() => reject(new Error('wd ' + ms + ' ' + w)), ms); return Promise.race([p, promise]).finally(() => clearTimeout(t)); };
const sleep = (ms) => { const { promise, resolve } = Promise.withResolvers(); setTimeout(resolve, ms); return promise; };
async function js2(e) { try { return await guard(js(e), 12000, 'js'); } catch (x) { return 'ERR:' + x.message; } }
// 页内**探针**：一次 evaluate 取全量事实。本脚本只取事实（判定在宿主侧 probeFacts/baseProblems/clickProblems，
// 唯一实现——这样自测可直接喂夹具，且不会出现「页内一套判据、宿主另一套」的漂移）。
const PROBE_EXPR = "(() => { const r = document.getElementById('root'); const b = document.getElementById('boot-skeleton'); const boot = !!(b && !(b.classList.contains('removed') || getComputedStyle(b).display === 'none')); const t = document.body ? document.body.innerText : ''; return { rc: r ? r.children.length : 0, len: t.length, bad: /undefined|NaN/.test(t), boot: boot, ph: t.indexOf('" + PLACEHOLDER + "') >= 0, errs: (window.__protoErrs || []).slice(0, 4) }; })()";
async function probe() { const v = await js2(PROBE_EXPR); return (v && typeof v === 'object') ? v : { probeError: String(v) }; }
// waitRender: 断言前**等待 React 挂载**（R1/R2 谓词）；未捕获异常即提前收敛（再等也不会自愈），超时返回最后一次观测。
async function waitRender(ms) {
  const dl = Date.now() + ms; let st = null;
  while (Date.now() < dl) {
    st = await probe();
    if (st.probeError) return st;
    if ((st.rc > 0 && !st.boot) || (st.errs && st.errs.length > 0)) return st;
    await sleep(300);
  }
  return st;
}
async function click(label) {
  return js2('(() => { const sel = "button, a, [role=\\"button\\"], [role=\\"tab\\"], [role=\\"menuitem\\"]"; const els=[...document.querySelectorAll(sel)]; const b=els.find(x=>x.textContent.trim()===' + JSON.stringify(label) + '); if(!b) return "NO_BUTTON"; b.click(); return "CLICKED"; })()');
}
await (async () => {
  try {
    task = await ${egoCreateSpaceExpr(JSON.stringify(spaceName), profile)};
    await guard(openOrReuseTab('about:blank', { wait: true, timeout: 15 }), 20000, 'openTab');
    await guard(cdp('Page.addScriptToEvaluateOnNewDocument', { source: '(() => { window.__protoErrs = []; window.addEventListener("error", (e) => { try { window.__protoErrs.push("uncaught: " + String((e.error && (e.error.stack || e.error.message)) || e.message)); } catch (x) { window.__protoErrs.push("uncaught: <unstringifiable>"); } }); window.addEventListener("unhandledrejection", (e) => { try { window.__protoErrs.push("rejection: " + String((e.reason && (e.reason.stack || e.reason.message)) || e.reason)); } catch (x) { window.__protoErrs.push("rejection: <unstringifiable>"); } }); })()' }), 15000, 'hook');
    for (const step of plan) {
      const raw = { file: step.file, kind: step.kind, clicksTotal: (step.clicks || []).length, clicksSkipped: 0, base: null, clicks: [] };
      await guard(gotoUrl(step.url), 45000, 'goto ' + step.file);
      await sleep(1500);
      raw.base = await waitRender(15000);
      for (const label of (step.clicks || [])) {
        const c = await click(label);
        // 导航项标签在部分原型中为图标型（无文本）或需展开后才渲染 ⇒ 未命中记为 skipped（不判失败），
        // 命中则必须渲染通过（该路径正是「点击即崩」的捕获点）。
        if (c !== 'CLICKED') { raw.clicksSkipped += 1; continue; }
        await sleep(900);
        raw.clicks.push({ label: label, probe: await waitRender(12000) });
      }
      res.results.push(raw);
    }
  } catch (e) { res.fatal = String(e && e.message); }
  finally {
    try { fs.writeFileSync(outPath, JSON.stringify(res, null, 2)); } catch (e) {}
    cliLog(JSON.stringify(res));
    if (task) {
      let closedDone = false;
      try { const r = await completeTaskSpace(task.id || task.name, { keep: false }); closedDone = !!(r && r.done); } catch (e) {}
      try { await ego.closeTaskSpace(); } catch (e) {}
      // 关闭确证回传（父进程据此注销 lease）；close 挂住则无此行 ⇒ 父进程保留 lease，
      // 运行进程死后由判据回收（保住僵尸兜底）。
      cliLog('SPACE_CLOSED ' + JSON.stringify({ done: closedDone }));
    }
  }
})();
`;

// ═══════════════════════════════════════════════════════════
// 判定面（宿主侧唯一实现；自测直接喂夹具）
// ═══════════════════════════════════════════════════════════

/** 页内探针的观测事实（判定输入）。 */
interface ProbeFacts {
  ok: boolean;
  probeError?: string;
  rc: number;
  len: number;
  bad: boolean;
  boot: boolean;
  ph: boolean;
  errs: string[];
}

/** 探针原始观测 → 事实（形态守卫在此收口：缺失/畸形**不得**静默当作通过）。 */
function probeFacts(p: unknown): ProbeFacts {
  // 入参是浏览器侧 evaluate 的 JSON 边界：无 schema 可依，逐字段守卫即本函数的职责；
  // `o` 只作字段读取的载体，取值一律经 typeof/=== 校验后才落到 ProbeFacts。
  const o: Record<string, unknown> | null =
    p && typeof p === "object" ? (p as Record<string, unknown>) : null;
  if (!o || typeof o.probeError === "string" || typeof o.rc !== "number") {
    return {
      ok: false,
      probeError: String(o?.probeError ?? "probe-missing"),
      rc: -1,
      len: -1,
      bad: false,
      boot: false,
      ph: false,
      errs: [],
    };
  }
  return {
    ok: true,
    rc: o.rc,
    len: typeof o.len === "number" ? o.len : -1,
    bad: o.bad === true,
    boot: o.boot === true,
    ph: o.ph === true,
    errs: Array.isArray(o.errs) ? o.errs.map(String).slice(0, 4) : [],
  };
}

function firstLine(s: string): string {
  return s.split("\n")[0].slice(0, 200);
}

/** 基础观测 → 问题清单（R1–R6）。空数组 = 通过。 */
function baseProblems(f: ProbeFacts): string[] {
  if (!f.ok) return [`probe-failed: ${f.probeError}`];
  const p: string[] = [];
  if (f.rc <= 0) p.push("root-unmounted");
  if (f.boot) p.push("boot-skeleton-still-shown");
  if (f.len < MIN_TEXT) p.push(`content-too-short(${f.len})`);
  if (f.bad) p.push("undefined/NaN-in-text");
  if (f.ph) p.push("placeholder-still-shown");
  if (f.errs.length > 0) p.push(`runtime-error: ${firstLine(f.errs[0])}`);
  return p;
}

/** 点击后观测 → 问题清单（同源事实，标签化以定位导航项）。 */
function clickProblems(label: string, f: ProbeFacts): string[] {
  if (!f.ok) return [`click-probe-failed:${label}: ${f.probeError}`];
  const p: string[] = [];
  if (f.rc <= 0 || f.len < MIN_TEXT) p.push(`click-blank:${label}(${f.len})`);
  if (f.boot) p.push(`click-boot-skeleton:${label}`);
  if (f.bad) p.push(`click-undefined:${label}`);
  if (f.ph) p.push(`click-placeholder:${label}`);
  if (f.errs.length > 0)
    p.push(`click-runtime-error:${label}: ${firstLine(f.errs[0])}`);
  return p;
}

interface RawClick {
  label: string;
  probe: unknown;
}
interface RawStep {
  file: string;
  kind: string;
  clicksTotal?: number;
  clicksSkipped?: number;
  base?: unknown;
  clicks?: RawClick[];
}
interface RenderOutcome {
  checked: number;
  results: Result[];
  /** 违规（未过棘轮过滤；基线分族在报告面用 `baseline.partition` 做）。 */
  violations: Violation[];
  /** 执行面失败（浏览器脚本未产出结果/超时）——不可基线豁免。 */
  fatal: string | null;
  /** 计划为空 / 无 ego ⇒ 跳过（rc 0，但不伪装成「断言通过」）。 */
  skipped: boolean;
}

function toResult(raw: RawStep): { result: Result; problems: string[] } {
  const problems = baseProblems(probeFacts(raw.base));
  for (const c of raw.clicks ?? [])
    problems.push(
      ...clickProblems(String(c?.label ?? "?"), probeFacts(c?.probe)),
    );
  const total = raw.clicksTotal ?? (raw.clicks ?? []).length;
  const skipped = raw.clicksSkipped ?? 0;
  return {
    result: {
      file: String(raw.file),
      kind: String(raw.kind),
      ok: problems.length === 0,
      detail: problems.join(", "),
      clicksTotal: total,
      clicksAttempted: total - skipped,
      clicksSkipped: skipped,
    },
    problems,
  };
}

const EMPTY_OUTCOME: RenderOutcome = {
  checked: 0,
  results: [],
  violations: [],
  fatal: null,
  skipped: true,
};

/** 跑一次浏览器断言（计划构造 → ego heredoc → 宿主判定）。 */
function runRenderAssertions(opts: {
  ns: string | null;
  explicit: readonly string[];
  changed: boolean;
  profile: string | undefined;
}): RenderOutcome {
  const plan = buildPlan(opts);
  if (plan.length === 0) {
    SINK.out("[check-prototype-render] 未发现目标原型产物（跳过）");
    return EMPTY_OUTCOME;
  }

  const ego = Bun.which("ego-browser");
  if (!ego) {
    SINK.err(
      "[check-prototype-render] ⚠ SKIP：未找到 ego-browser（本环境无法做运行期判断；开发机技能流程为权威）",
    );
    return EMPTY_OUTCOME;
  }

  const dir = mkdtempSync(join(tmpdir(), "proto-render-"));
  const planPath = join(dir, "plan.json");
  const outPath = join(dir, "out.json");
  const spaceName = `alioth-proto-render-${Date.now()}`;
  writeFileSync(planPath, JSON.stringify(plan), "utf8");
  if (process.env.VV_DEBUG_SCRIPT === "1") {
    const dump = join(tmpdir(), "proto-render-script.js");
    writeFileSync(
      dump,
      BROWSER_SCRIPT(planPath, outPath, spaceName, opts.profile),
      "utf8",
    );
    SINK.err(`[check-prototype-render] 脚本已转储: ${dump}`);
  }
  SINK.err(
    `[check-prototype-render] 浏览器断言启动（${plan.length} 个产物）...`,
  );
  leaseCli([
    "touch",
    spaceName,
    "--state",
    "starting",
    "--run-pid",
    String(process.pid),
  ]);
  const proc = Bun.spawnSync([ego, ...egoSpawnArgs("", ego)], {
    stdin: Buffer.from(
      BROWSER_SCRIPT(planPath, outPath, spaceName, opts.profile),
    ),
    cwd: REPO_ROOT,
  });
  const spawnOut = `${proc.stdout ? Buffer.from(proc.stdout).toString("utf8") : ""}${proc.stderr ? Buffer.from(proc.stderr).toString("utf8") : ""}`;

  let raws: RawStep[] = [];
  let fatal: string | null = null;
  if (existsSync(outPath)) {
    const parsed = JSON.parse(readFileSync(outPath, "utf8")) as {
      results?: RawStep[];
      fatal?: string | null;
    };
    raws = parsed.results ?? [];
    fatal = parsed.fatal ?? null;
  } else {
    // 无 out.json ⇒ 浏览器脚本异常退出；**必须**带子进程 stderr，否则不可诊断（实测：Linux 线 compat 缺失
    // 时只报 exit 码，根因（ReferenceError/语法错）被吞）
    const childErr = (
      proc.stderr ? Buffer.from(proc.stderr).toString("utf8") : ""
    ).trim();
    fatal = `浏览器脚本未产出结果（exit=${proc.exitCode}）${childErr ? `\n${childErr.slice(-800)}` : ""}`;
  }
  rmSync(dir, { recursive: true, force: true });

  // 关闭确证回传（脚本在 close 之后打的 `SPACE_CLOSED {done}`）⇒ 注销 lease；
  // 未确证（close 挂住 / 脚本被 SIGKILL）⇒ 保留 lease，运行进程死后由判据回收。
  leaseCli([
    "touch",
    spaceName,
    "--state",
    fatal ? "error" : "running",
    "--run-pid",
    String(process.pid),
  ]);
  const closedMatch = spawnOut.match(/SPACE_CLOSED (\{.*\})/);
  if (closedMatch) {
    try {
      const parsed: unknown = JSON.parse(closedMatch[1]);
      if (
        parsed &&
        typeof parsed === "object" &&
        "done" in parsed &&
        parsed.done === true
      ) {
        leaseCli(["unregister", spaceName]);
      }
    } catch {
      /* 标记畸形 ⇒ 保留 lease（保守） */
    }
  }

  const results: Result[] = [];
  const violations: Violation[] = [];
  for (const raw of raws) {
    const { result, problems } = toResult({
      ...raw,
      file: repoRel(String(raw.file)),
    });
    results.push(result);
    if (problems.length > 0)
      violations.push({
        path: result.file,
        message: result.detail,
        detail: result.detail,
      });
  }
  return {
    checked: results.length,
    results,
    violations,
    fatal,
    skipped: false,
  };
}

/** 判定面缓存：`predicate` 跑一次浏览器，报告面复用（绝不复跑浏览器）。 */
let CACHE: RenderOutcome | null = null;

// ═══════════════════════════════════════════════════════════
// 自证夹具
// ═══════════════════════════════════════════════════════════

/** 必崩夹具：`#root` 空 + `#boot-skeleton` 常驻 + 挂载期抛 ReferenceError（等价「点击即崩」类产物）。 */
const CRASH_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>prototype-render 必崩夹具</title></head>
<body>
<div id="root"></div>
<div id="boot-skeleton"><div class="boot-skeleton-sidebar"></div><div class="boot-skeleton-main"></div><div class="boot-skeleton-loader"></div></div>
<script>
  // 模拟「React 挂载期抛错」：脚本自身抛 ReferenceError ⇒ #root 保持空、boot 骨架不被移除
  (function () { window.__fixtureThrown = true; throw new ReferenceError('fixture: 必崩产物（模拟挂载期 ReferenceError）'); })();
</script>
</body></html>
`;

const gate = defineCheck({
  id: "prototype-render",
  label: "check-prototype-render",
  flags: ["--changed"],
  valueFlags: ["--ns", "--profile"],
  baseline: {
    file: BASELINE_FILE,
    ruleReference:
      "docs/specs/MODULE_SPEC.md（原型运行期渲染断言；判据正本 = scripts/check/check-prototype-render.ts 头注 R1-R6）",
    // 棘轮键 = 产物路径（同一产物的问题集合变化不改变键 ⇒ 修好一半也仍是存量，直到整件转绿）
    keyOf: (v) => v.path,
  },
  predicate: (ctx) => {
    CACHE = runRenderAssertions({
      ns: ctx.valueFlags.get("--ns")?.[0] ?? null,
      explicit: ctx.argv.filter((a) => a.endsWith(".html")),
      changed: ctx.flags.has("--changed"),
      profile:
        ctx.valueFlags.get("--profile")?.[0] ?? resolveEgoProfile(undefined),
    });
    return CACHE.violations;
  },
  report: (ctx, res, io) => {
    // 判定核全同步（Bun.spawnSync）⇒ predicate 已跑完；此处只做基线分族与输出
    const o =
      CACHE ??
      runRenderAssertions({
        ns: ctx.valueFlags.get("--ns")?.[0] ?? null,
        explicit: ctx.argv.filter((a) => a.endsWith(".html")),
        changed: ctx.flags.has("--changed"),
        profile:
          ctx.valueFlags.get("--profile")?.[0] ?? resolveEgoProfile(undefined),
      });
    // 分族输入 = **本次全部违规**（不复用基座的 tracked 过滤）：显式指定的目标（含未入库的新原型 /
    // `--self-test` 夹具）是**有意判定**，MUST 计作新增——否则门禁会对「还没 commit 的新产物」静默放行。
    const keys = baseline.loadKeys(substrateRoot(), BASELINE_FILE);
    const { fresh, grandfathered } = baseline.partition(
      o.violations,
      keys,
      (v) => v.path,
    );
    const kindOf = new Map(o.results.map((r) => [r.file, r.kind]));

    if (res.json) {
      io.out(
        `${JSON.stringify({ checked: o.checked, failed: o.violations.length, fresh: fresh.length, grandfathered: grandfathered.length, results: o.results, fatal: o.fatal }, null, 2)}\n`,
      );
    } else {
      io.out(
        `[check-prototype-render] 断言 ${o.checked} 个产物（kind: block/module/app）\n`,
      );
      if (grandfathered.length > 0) {
        io.out(
          `[BASELINE] prototype-render: ${grandfathered.length} 个产物存量已基线豁免（ratchet，只拦新增）\n`,
        );
      }
      for (const v of fresh)
        io.err(
          `❌ ${kindOf.get(v.path) ?? "?"} ${v.path}\n    ${v.detail ?? v.message}\n`,
        );
      if (o.fatal) io.err(`❌ 执行失败: ${o.fatal}\n`);
      if (fresh.length === 0 && !o.fatal)
        io.out("[check-prototype-render] ✅ R1-R6 全部通过\n");
    }

    if (!res.fail) return 0; // 审计模式（无 --fail）：只报告不阻断（既有语义）
    return fresh.length > 0 || o.fatal !== null ? 1 : 0;
  },
  selfTest() {
    const failures: string[] = [];

    // ① 判定面夹具（不依赖浏览器）：含**必崩负例**与实测的健康样本（防误报）
    const healthyBlock = {
      rc: 1,
      len: 154,
      bad: false,
      boot: false,
      ph: false,
      errs: [],
    }; // 实测 Blocks/account-futures
    const healthyModule = {
      rc: 1,
      len: 1469,
      bad: false,
      boot: false,
      ph: false,
      errs: [],
    }; // 实测 Modules/biz-analysis
    const cases: Array<{ name: string; probe: unknown; expect: string[] }> = [
      {
        name: "必崩产物（#root 未挂载 + Script error. + boot 常驻）",
        probe: {
          rc: 0,
          len: 0,
          bad: false,
          boot: true,
          ph: false,
          errs: ["uncaught: Script error."],
        },
        expect: [
          "root-unmounted",
          "boot-skeleton-still-shown",
          "content-too-short(0)",
          "runtime-error: uncaught: Script error.",
        ],
      },
      {
        name: "健康块产物（实测 rc=1/len=154）",
        probe: healthyBlock,
        expect: [],
      },
      {
        name: "健康模块产物（实测 rc=1/len=1469）",
        probe: healthyModule,
        expect: [],
      },
      {
        name: "undefined/NaN 文本",
        probe: { ...healthyBlock, bad: true },
        expect: ["undefined/NaN-in-text"],
      },
      {
        name: "boot 占位文案仍在",
        probe: { ...healthyBlock, ph: true },
        expect: ["placeholder-still-shown"],
      },
      {
        name: "探针失败（evaluate 挂/超时）",
        probe: "ERR:js watchdog timeout",
        expect: ["probe-failed: "],
      },
      {
        name: "探针返回畸形对象",
        probe: { unexpected: 1 },
        expect: ["probe-failed: "],
      },
    ];
    for (const c of cases) {
      const got = baseProblems(probeFacts(c.probe));
      const missing = c.expect.filter((e) => !got.some((g) => g.startsWith(e)));
      if (missing.length > 0)
        failures.push(
          `${c.name}：未报 ${JSON.stringify(missing)}（实得 ${JSON.stringify(got)}）`,
        );
      if (c.expect.length === 0 && got.length > 0)
        failures.push(`${c.name}：误报 ${JSON.stringify(got)}`);
    }
    // 点击路径（同源事实，标签化）
    const clickCodes = clickProblems(
      "工作台",
      probeFacts({
        rc: 0,
        len: 0,
        bad: false,
        boot: true,
        ph: false,
        errs: ["uncaught: boom"],
      }),
    );
    for (const e of [
      "click-blank:工作台(0)",
      "click-boot-skeleton:工作台",
      "click-runtime-error:工作台: uncaught: boom",
    ]) {
      if (!clickCodes.includes(e))
        failures.push(
          `点击夹具未报 ${JSON.stringify(e)}（实得 ${JSON.stringify(clickCodes)}）`,
        );
    }

    // ② 端到端必崩夹具（真实浏览器；ego 缺失 ⇒ 告警跳过，MUST NOT 静默当作通过）
    const ego = Bun.which("ego-browser");
    if (!ego) {
      process.stderr.write(
        "⚠ prototype-render self-test：未找到 ego-browser ⇒ 端到端必崩夹具未执行（判定面夹具已执行）\n",
      );
    } else {
      const dir = mkdtempSync(join(tmpdir(), "proto-render-selftest-"));
      const fixture = join(dir, "crash-fixture.html");
      writeFileSync(fixture, CRASH_FIXTURE_HTML, "utf8");
      const savedSink = SINK;
      const buf: string[] = [];
      SINK = { out: (l) => buf.push(l), err: (l) => buf.push(l) };
      let o: RenderOutcome;
      try {
        o = runRenderAssertions({
          ns: null,
          explicit: [fixture],
          changed: false,
          profile: undefined,
        });
      } finally {
        SINK = savedSink;
        rmSync(dir, { recursive: true, force: true });
      }
      if (o.fatal !== null)
        failures.push(`端到端夹具执行失败：${firstLine(o.fatal)}`);
      else if (o.checked !== 1)
        failures.push(`端到端夹具未被执行（checked=${o.checked}）`);
      else if (o.violations.length === 0)
        failures.push("必崩夹具被判通过（R1-R6 死判据复活）");
    }

    if (failures.length > 0) {
      for (const f of failures)
        process.stderr.write(`❌ prototype-render self-test 失败：${f}\n`);
      return 1;
    }
    process.stdout.write(
      "✅ prototype-render self-test 通过（判定面夹具 7 例 + 点击夹具 + 端到端必崩夹具）\n",
    );
    return 0;
  },
});

if (import.meta.main) process.exit(await gate.main());
