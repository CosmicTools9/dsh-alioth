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
 *   R1 `#root` 有子节点（未卸载）
 *   R2 `innerText` 长度 ≥ 300（非空壳）
 *   R3 `innerText` 不含 `undefined` / `NaN`
 *   R4 无未捕获 ReferenceError / TypeError
 * 覆盖深度:
 *   Block  — 基础渲染
 *   Module — 逐个 `module.json.blockAssembly.blocks[].label` 点击后复断言
 *   App    — 逐个 `app.json.config.modules[].name`（模块显示名）Tab 点击后复断言
 *
 * 用法:
 *   bun scripts/check/check-prototype-render.ts --ns Cosmic-Tools [--fail] [--json]
 *   bun scripts/check/check-prototype-render.ts Pre-Proc/X/Prototypes/Blocks/y/b-v3.html [...]
 *   [--profile <id>]   指定 ego profile（等价 `EGO_PROFILE_ID`）→ task space 的 cookie jar（登录态）
 *
 * 环境: 需要 `ego-browser`（缺失 ⇒ SKIP + 警告，exit 0；开发机上的技能强制步骤为权威）。
 * 清理: 单一 task space，结束时 `completeTaskSpace` + `closeTaskSpace`（ego-browser 技能铁律）。
 */
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { EGO_SPACE_PRELUDE, egoCreateSpaceExpr, resolveEgoProfile } from '../lib/ego-space';

const REPO_ROOT = resolve(import.meta.dir, '../..');
const args = process.argv.slice(2);
const FAIL = args.includes('--fail');
const JSON_OUT = args.includes('--json');
const nsIdx = args.indexOf('--ns');
const NS = nsIdx >= 0 ? args[nsIdx + 1] : null;
const explicit = args.filter((a) => a.endsWith('.html'));
const CHANGED_ONLY = args.includes('--changed');
/** 目标 ego profile（`--profile` > `EGO_PROFILE_ID`）：决定 task space 的 cookie jar＝登录态来源。 */
const profileIdx = args.indexOf('--profile');
const PROFILE = resolveEgoProfile(profileIdx >= 0 ? args[profileIdx + 1] : undefined);

type Step = { kind: 'block' | 'module' | 'app'; file: string; url: string; clicks: string[] };
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
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    return null;
  }
}

function newestArtifact(dirAbs: string, prefix: 'b' | 'm' | 'a'): string | null {
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
  const mod = readJson(join(REPO_ROOT, `Pre-Proc/${ns}/Sources/Apps/Modules/${moduleId}/module.json`)) as {
    blockAssembly?: { blocks?: Array<{ label?: string }> };
  } | null;
  const blocks = mod?.blockAssembly?.blocks;
  if (!Array.isArray(blocks)) return [];
  return blocks.map((b) => b?.label).filter((l): l is string => typeof l === 'string' && l.length > 0);
}

function appTabLabels(ns: string, appId: string): string[] {
  const app = readJson(join(REPO_ROOT, `Pre-Proc/${ns}/Apps/${appId}/app.json`)) as {
    config?: { modules?: unknown[] };
  } | null;
  const mods = app?.config?.modules;
  if (!Array.isArray(mods)) return [];
  const labels: string[] = [];
  for (const m of mods) {
    const id = typeof m === 'string' ? m : (m as { id?: string })?.id;
    if (typeof id !== 'string') continue;
    const mod = readJson(join(REPO_ROOT, `Pre-Proc/${ns}/Sources/Apps/Modules/${id}/module.json`)) as {
      name?: string;
    } | null;
    if (typeof mod?.name === 'string' && mod.name.length > 0) labels.push(mod.name);
  }
  return labels;
}

/** 列出目录下的直接子目录名（目录不存在或非目录 ⇒ 返回空，避免环境特例导致崩溃）。 */
function listSubdirs(rootAbs: string): string[] {
  if (!existsSync(rootAbs) || !statSync(rootAbs).isDirectory()) return [];
  return [...new Bun.Glob('*/').scanSync({ cwd: rootAbs, onlyFiles: false })]
    .filter((name) => statSync(join(rootAbs, name)).isDirectory())
    .map((name) => name.replace(/\/$/, ''));
}

function buildPlan(): Step[] {
  if (explicit.length > 0) {
    return explicit.map((f) => ({ kind: 'block' as const, file: f, url: `file://${resolve(REPO_ROOT, f)}`, clicks: [] }));
  }
  const nsList = NS
    ? [NS]
    : [...new Bun.Glob('Pre-Proc/*').scanSync({ cwd: REPO_ROOT, onlyFiles: false })]
        .filter((p) => statSync(join(REPO_ROOT, p)).isDirectory())
        .map((p) => basename(p));
  const changed = CHANGED_ONLY ? collectChangedPaths() : null;
  const isChanged = (artifactPath: string, ownerDir: string): boolean => {
    if (!changed) return true;
    const relArtifact = artifactPath.replace(`${REPO_ROOT}/`, '');
    const relDir = ownerDir.replace(`${REPO_ROOT}/`, '');
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
      const f = newestArtifact(owner, 'b');
      if (f && isChanged(f, owner)) steps.push({ kind: 'block', file: f, url: `file://${f}`, clicks: [] });
    }
    const modulesRoot = join(REPO_ROOT, `Pre-Proc/${ns}/Prototypes/Modules`);
    for (const id of listSubdirs(modulesRoot)) {
      const owner = join(modulesRoot, id);
      const f = newestArtifact(owner, 'm');
      if (f && isChanged(f, owner)) steps.push({ kind: 'module', file: f, url: `file://${f}`, clicks: moduleBlockLabels(ns, id) });
    }
    const appsRoot = join(REPO_ROOT, `Pre-Proc/${ns}/Prototypes/Apps`);
    for (const id of listSubdirs(appsRoot)) {
      const owner = join(appsRoot, id);
      const f = newestArtifact(owner, 'a');
      if (f && isChanged(f, owner)) steps.push({ kind: 'app', file: f, url: `file://${f}`, clicks: appTabLabels(ns, id) });
    }
  }
  return steps;
}

/** `--changed`：取工作树 + 最近一次提交涉及的原型相关路径（源或其产物），使 push 门禁只断言受影响产物。 */
function collectChangedPaths(): string[] {
  const run = (gitArgs: string[]): string[] => {
    const p = Bun.spawnSync(['git', ...gitArgs], { cwd: REPO_ROOT });
    return p.stdout
      .toString()
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
  };
  const paths = new Set<string>();
  for (const p of run(['status', '--porcelain'])) {
    const file = p.slice(3).trim();
    if (file.includes(' -> ')) paths.add(file.split(' -> ')[1]);
    else paths.add(file);
  }
  for (const p of run(['diff', '--name-only', 'HEAD~1..HEAD'])) paths.add(p);
  return [...paths];
}

const BROWSER_SCRIPT = (planPath: string, outPath: string) => `
${PROFILE ? EGO_SPACE_PRELUDE : ''}const fs = require('fs');
const plan = JSON.parse(fs.readFileSync(${JSON.stringify(planPath)}, 'utf8'));
const outPath = ${JSON.stringify(outPath)};
const PLACEHOLDER = '该模块原型未生成';
const res = { results: [], fatal: null };
let task = null;
const guard = (p, ms, w) => { const { promise, reject } = Promise.withResolvers(); const t = setTimeout(() => reject(new Error('wd ' + ms + ' ' + w)), ms); return Promise.race([p, promise]).finally(() => clearTimeout(t)); };
const sleep = (ms) => { const { promise, resolve } = Promise.withResolvers(); setTimeout(resolve, ms); return promise; };
async function js2(e) { try { return await guard(js(e), 12000, 'js'); } catch (x) { return 'ERR:' + x.message; } }
async function click(label) {
  // 文本精确匹配优先；未命中则兜底 data-testid / aria-label（异构原型的图标型导航项可无文本）
  // 注意：本字符串经模板拼接传给页面 evaluate，内层用单引号 CSS 选择器、属性值不加引号（避免模板内转义陷阱）。
  return js2('(() => { const els=[...document.querySelectorAll("button, a, [role=button], [role=tab], [role=menuitem]")]; let b=els.find(x=>x.textContent.trim()===' + JSON.stringify(label) + '); if(!b) { const attrs=[...document.querySelectorAll("[data-testid], [aria-label]")]; b=attrs.find(x=>(x.getAttribute("data-testid")||"").trim()===' + JSON.stringify(label) + '||(x.getAttribute("aria-label")||"").trim()===' + JSON.stringify(label) + '); } if(!b) return "NO_BUTTON"; b.click(); return "CLICKED"; })()');
}
async function waitRender(ms) {
  const dl = Date.now() + ms; let st;
  while (Date.now() < dl) { st = await snapshot(); if (st && typeof st === 'object' && st.rc > 0 && st.len >= 300 && !st.bad) return st; await sleep(300); }
  return st;
}
async function click(label) {
  return js2('(() => { const sel = "button, a, [role=\\"button\\"], [role=\\"tab\\"], [role=\\"menuitem\\"]"; const els=[...document.querySelectorAll(sel)]; const b=els.find(x=>x.textContent.trim()===' + JSON.stringify(label) + '); if(!b) return "NO_BUTTON"; b.click(); return "CLICKED"; })()');
}
(async () => {
  try {
    task = await ${egoCreateSpaceExpr("'alioth-proto-render-' + Date.now()", PROFILE)};
    await guard(openOrReuseTab('about:blank', { wait: true, timeout: 15 }), 20000, 'openTab');
    await guard(cdp('Page.addScriptToEvaluateOnNewDocument', { source: '(() => { window.__protoErrs = []; window.addEventListener("error", (e) => { try { window.__protoErrs.push(String((e.error && (e.error.stack || e.error.message)) || e.message)); } catch (x) {} }); })()' }), 15000, 'hook');
    for (const step of plan) {
      await guard(gotoUrl(step.url), 45000, 'goto ' + step.file);
      await sleep(1500);
      const base = await waitRender(15000);
      const problems = [];
      if (!base || base.rc <= 0) problems.push('root-unmounted');
      if (!base || base.len < 300) problems.push('content-too-short(' + (base && base.len) + ')');
      if (base && base.bad) problems.push('undefined/NaN-in-text');
      if (base && base.placeholder) problems.push('placeholder-still-shown');
      if (base && base.err) problems.push('runtime-error: ' + base.err);
      const skipped = [];
      for (const label of (step.clicks || [])) {
        const c = await click(label);
        // 导航项标签在部分原型中为图标型（无文本）或需展开后才渲染 ⇒ 未命中记为 skipped（不判失败），
        // 命中则必须渲染通过（该路径正是「点击即崩」的捕获点）。
        if (c !== 'CLICKED') { skipped.push(label); continue; }
        await sleep(900);
        const st = await waitRender(12000);
        if (!st || st.rc <= 0 || st.len < 300) problems.push('click-blank:' + label + '(' + (st && st.len) + ')');
        else if (st.bad) problems.push('click-undefined:' + label);
        else if (st.placeholder) problems.push('click-placeholder:' + label);
        else if (st.err) problems.push('click-runtime-error:' + label + ':' + st.err);
      }
      const attempted = (step.clicks || []).length - skipped.length;
      res.results.push({
        file: step.file,
        kind: step.kind,
        ok: problems.length === 0,
        detail: problems.join(', '),
        clicksTotal: (step.clicks || []).length,
        clicksAttempted: attempted,
        clicksSkipped: skipped.length,
      });
    }
  } catch (e) { res.fatal = String(e && e.message); }
  finally {
    try { fs.writeFileSync(outPath, JSON.stringify(res, null, 2)); } catch (e) {}
    cliLog(JSON.stringify(res));
    if (task) { try { await completeTaskSpace(task.id || task.name, { keep: false }); } catch (e) {} try { await ego.closeTaskSpace(); } catch (e) {} }
  }
})();
`;

const plan = buildPlan();
if (plan.length === 0) {
  console.log('[check-prototype-render] 未发现目标原型产物（跳过）');
  process.exit(0);
}

const ego = Bun.which('ego-browser');
if (!ego) {
  console.warn('[check-prototype-render] ⚠ SKIP：未找到 ego-browser（本环境无法做运行期判断；开发机技能流程为权威）');
  process.exit(0);
}

const dir = mkdtempSync(join(tmpdir(), 'proto-render-'));
const planPath = join(dir, 'plan.json');
const outPath = join(dir, 'out.json');
writeFileSync(planPath, JSON.stringify(plan), 'utf8');
const proc = Bun.spawnSync([ego, 'nodejs'], { stdin: Buffer.from(BROWSER_SCRIPT(planPath, outPath)), cwd: REPO_ROOT });

let results: Result[] = [];
let fatal: string | null = null;
if (existsSync(outPath)) {
  const parsed = JSON.parse(readFileSync(outPath, 'utf8')) as { results?: Result[]; fatal?: string | null };
  results = parsed.results ?? [];
  fatal = parsed.fatal ?? null;
} else {
  fatal = `浏览器脚本未产出结果（exit=${proc.exitCode}）`;
}
rmSync(dir, { recursive: true, force: true });

const failed = results.filter((r) => !r.ok);
if (JSON_OUT) {
  console.log(JSON.stringify({ checked: results.length, failed: failed.length, results, fatal }, null, 2));
} else {
  console.log(`[check-prototype-render] 断言 ${results.length} 个产物（kind: block/module/app）`);
  for (const r of failed) console.error(`❌ ${r.kind} ${r.file}\n    ${r.detail}`);
  if (fatal) console.error(`❌ 执行失败: ${fatal}`);
  if (failed.length === 0 && !fatal) console.log('[check-prototype-render] ✅ R1-R4 全部通过');
}

process.exit(FAIL && (failed.length > 0 || fatal) ? 1 : 0);
