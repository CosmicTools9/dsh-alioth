#!/usr/bin/env bun
/**
 * check-app-frontend-isolation.ts — 前端 App 隔离判据：由前端配置文件计算依赖图并判隔离特征
 *
 * 规约/决策来源：ADR D-047（前端隔离粒度 = App，与 namespace 双层；定义面 = 前端配置文件；
 * 判据 = 门禁做依赖图计算）；change replace-ns-overlay-with-framework-extension-seams。
 *
 * 判据（全部正向 MUST 表述；违规即列出条目与建议修法）：
 *   ① 归属完备性：ns 仓磁盘上的每个前端（`Sources/Apps/Modules/<mod>/frontend` 与
 *      `Sources/Apps/Blocks/<block>/frontend`）MUST 归属至少一个 App 声明
 *      （`Pre-Proc/{ns}/Apps/<app>/app.json` 的 `config.modules` / `config.blocks`）
 *      —— 未归属即隔离特征不可计算。
 *   ② 成员声明完备性：每个已归属前端 MUST 作为显式成员出现在**供给通道**的 `pnpm-workspace.yaml` ——
 *      供给通道 = **ns 仓根自持安装面**（`Pre-Proc/{ns}/…` 成员，相对 ns 仓根；change
 *      `move-ns-frontend-assembly-to-ns` / ADR D-046 修订 2026-10-05）；平台侧历史通道
 *      （平台 `../Alioth-NS-{ns}/…` 成员块）已退役，MUST NOT 复活（本判据不再接受）。
 *   ③ 依赖图入账：每个已声明成员 MUST 在**其所属 workspace** 的 `pnpm-lock.yaml` `importers` 有对应条目
 *      （= ns 仓根 lock）。缺失意味着该 App 的依赖须经他处兜底 ⇒ 违反隔离。
 *   ④ 跨 App 兜底检测：若同 ns 内某成员未入账而其同 App/它 App 成员已入账，判为
 *      「跨 App 兜底解析风险」并在报告中点名（缺声明 App MUST fail-loud，不得他 App 兜底）。
 *   ⑤ `@alioth/*` 声明面 ↔ 使用面一致（双向，change prune-ns-frontend-dep-declarations）：
 *      ⑤a 声明的 `@alioth/*` MUST 在单元内至少被引用一次（引用面 = 除自身 `package.json` 外的
 *          全部文本文件；文本命中判据，保守取向）；⑤b 被 import 的 `@alioth/*` MUST 已声明
 *          （import 面经 `Bun.Transpiler.scanImports` 提取，精确取向）。取向差异见 §检测器注释。
 *
 * 用法：
 *   bun scripts/check/check-app-frontend-isolation.ts [--ns NS] [--json] [--fail]
 *     --ns NS   仅检查该 ns（缺省 = 全部已挂载 ns）
 *     --json    机器可读输出
 *     --fail    存在违规时 rc=1（缺省 rc=0 仅报告）
 *
 * 退出码：0 报告完成（无 --fail 恒 0）；1 有 --fail 且存在违规；2 前置缺失（ns 仓/清单不可读）
 *
 * 实现口径：YAML 一律经 yq（仓内规约：YAML → yq；Bun 内建 YAML 对大体量 lock 解析异常，实测返回数组）。
 */
import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { mountedNs, nsRepoOf, portsTableRoot } from '../lib/ns-mounts';
import { intake, listUnitNames, resolveUnitDir } from '../lib/ns-frontend-intake';

import { EXCLUDE_DIRS as CANON_EXCLUDE_DIRS, isExcludedDirName } from './lib/gate-kit';
import { defineCheck } from './lib/check-interface';

const META_CHANNEL = '__meta';

/** 控制流信号 */
class ExitSignal {
  constructor(readonly rc: number) {}
}
/** 输出面 sink（默认真实标准流；状态函数注入缓冲 ⇒ 报告面回放） */
let SINK: { out: (l: string) => void; err: (l: string) => void } = {
  out: (l) => process.stdout.write(`${l}\n`),
  err: (l) => process.stderr.write(`${l}\n`),
};
const exit: (rc: number) => never = (rc) => {
  throw new ExitSignal(rc);
};
/** 旗标注入面（旧自解析 argv；由入口按基座 ctx 注入） */
let onlyNs: string | null = null;
let jsonOut = false;
let failOnViolation = false;

const root = process.env.APP_ISOLATION_ROOT
  ? path.resolve(process.env.APP_ISOLATION_ROOT)
  : path.resolve(import.meta.dirname, '..', '..');

// ── 自测（`--self-test`；gate-hygiene 于每次 push 自省调用）──────────────────────────
// 用 `APP_ISOLATION_ROOT` 夹具（子进程注入根）覆盖两条布局：挂载态可判 + 双源皆空 fail-closed。

function selfTest(): number {
  const { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync } = fs;
  const run = (fixtureRoot: string): { code: number; out: string } => {
    const p = Bun.spawnSync([process.execPath, import.meta.path, '--fail'], {
      env: { ...process.env, APP_ISOLATION_ROOT: fixtureRoot },
    });
    return { code: p.exitCode ?? -1, out: new TextDecoder().decode(p.stdout) + new TextDecoder().decode(p.stderr) };
  };
  let failed = 0;
  let total = 0;
  const check = (name: string, ok: boolean, detail: string) => {
    total++;
    if (!ok) failed++;
    SINK.out(`${ok ? '✔' : '✘'} ${name}${ok ? '' : `（${detail}）`}`);
  };
  const base = mkdtempSync(path.join(tmpdir(), 'app-isolation-selftest-'));
  const ns = 'Selftest';
  try {
    // ① 仅挂载态（同级无 Alioth-NS-*，模拟验证器临时 worktree）：MUST 判出 ns 且零违规
    //    （供给通道 = ns 仓自持安装面：挂载经符号链接指向 ns 仓根 workspace/lock）
    const nsRepo1 = path.join(base, `Alioth-NS-${ns}`);
    const mount = path.join(nsRepo1, 'Pre-Proc', ns);
    mkdirSync(path.join(mount, 'Apps', 'demo'), { recursive: true });
    writeFileSync(path.join(mount, 'Cargo.toml'), '[workspace]\n');
    writeFileSync(path.join(mount, 'Apps/demo/app.json'), JSON.stringify({ config: { modules: ['user'], blocks: [] } }));
    mkdirSync(path.join(mount, 'Sources/Apps/Modules/user/frontend'), { recursive: true });
    mkdirSync(path.join(base, 'Pre-Proc'), { recursive: true });
    symlinkSync(mount, path.join(base, 'Pre-Proc', ns));
    const member1 = `Pre-Proc/${ns}/Sources/Apps/Modules/user/frontend`;
    writeFileSync(path.join(nsRepo1, 'pnpm-workspace.yaml'), `packages:\n  - '${member1}'\n`);
    writeFileSync(path.join(nsRepo1, 'pnpm-lock.yaml'), `lockfileVersion: '9.0'\nimporters:\n  ${member1}: {}\n`);
    const mounted = run(base);
    check('挂载态（同级无 ns 仓）可判且零违规', mounted.code === 0 && mounted.out.includes(`[app-isolation] ${ns}:`), `rc=${mounted.code} ${mounted.out.trim()}`);

    // ② 双源皆空（无挂载、无同级）⇒ MUST fail-closed rc=2（不得静默判通过）
    const empty = mkdtempSync(path.join(tmpdir(), 'app-isolation-empty-'));
    writeFileSync(path.join(empty, 'pnpm-workspace.yaml'), 'packages: []\n');
    writeFileSync(path.join(empty, 'pnpm-lock.yaml'), `lockfileVersion: '9.0'\nimporters: {}\n`);
    const blank = run(empty);
    check('双源皆空 ⇒ fail-closed rc=2', blank.code === 2 && blank.out.includes('判据面失效'), `rc=${blank.code}`);
    rmSync(empty, { recursive: true, force: true });

    // ③ 判据 ⑤ 双向（正例 + 负例；R1 要求负例证明豁免/取向未被放宽）
    const dep = mkdtempSync(path.join(tmpdir(), 'app-isolation-deps-'));
    const dns = 'Deps';
    const dRepo = path.join(dep, `Alioth-NS-${dns}`);
    const dMount = path.join(dRepo, 'Pre-Proc', dns);
    const dUnit = path.join(dMount, 'Sources', 'Apps', 'Modules', 'u1', 'frontend');
    mkdirSync(path.join(dMount, 'Apps', 'demo'), { recursive: true });
    mkdirSync(path.join(dUnit, 'src'), { recursive: true });
    writeFileSync(path.join(dMount, 'Cargo.toml'), '[workspace]\n');
    writeFileSync(path.join(dMount, 'Apps/demo/app.json'), JSON.stringify({ config: { modules: ['u1'], blocks: [] } }));
    writeFileSync(
      path.join(dUnit, 'package.json'),
      JSON.stringify(
        {
          name: '@deps/u1-frontend',
          dependencies: { '@alioth/components': 'workspace:*', '@alioth/hooks': 'workspace:*' },
          devDependencies: { '@alioth/config': 'workspace:*', '@alioth/utils': 'workspace:*' },
        },
        null,
        2,
      ),
    );
    writeFileSync(
      path.join(dUnit, 'src/index.tsx'),
      [
        "import { Button } from '@alioth/components';", // 声明且 import ⇒ 放行
        "import { svc } from '@alioth/api';", // 未声明 import ⇒ ⑤b 命中
        'export const x = Button; export const y = svc;',
      ].join('\n'),
    );
    writeFileSync(
      path.join(dUnit, 'vite.config.ts'),
      [
        "import { defineModuleViteConfig } from '@alioth/config/vite';", // devDeps + 配置面使用 ⇒ 放行
        '// 可参考 @alioth/utils 的写法', // 仅文本命中 ⇒ ⑤a 放行（保护真声明）
        '// 亦可参考 @alioth/i18n 的 t()', // 未声明且仅注释 ⇒ ⑤b MUST NOT 报
        "export default defineModuleViteConfig({ rootDir: import.meta.dirname, moduleName: 'u1', port: 1 });",
      ].join('\n'),
    );
    mkdirSync(path.join(dep, 'Pre-Proc'), { recursive: true });
    symlinkSync(dMount, path.join(dep, 'Pre-Proc', dns));
    const dRel = `Pre-Proc/${dns}/Sources/Apps/Modules/u1/frontend`;
    writeFileSync(path.join(dRepo, 'pnpm-workspace.yaml'), `packages:\n  - '${dRel}'\n`);
    writeFileSync(path.join(dRepo, 'pnpm-lock.yaml'), `lockfileVersion: '9.0'\nimporters:\n  ${dRel}: {}\n`);
    const depRun = run(dep);
    check(
      '⑤a 死声明命中（声明未使用）',
      depRun.code === 1 && depRun.out.includes('死声明') && depRun.out.includes('@alioth/hooks'),
      `rc=${depRun.code} ${depRun.out.trim().slice(0, 200)}`,
    );
    check(
      '⑤b 未声明 import 命中',
      depRun.out.includes('未声明引用') && depRun.out.includes('@alioth/api'),
      `rc=${depRun.code} ${depRun.out.trim().slice(0, 200)}`,
    );
    check('⑤ 仅配置面（vite.config.ts）使用的包放行（负例）', !depRun.out.includes('@alioth/config'), `rc=${depRun.code}`);
    check(
      '⑤ 注释文本既不误报 ⑤b 也不破坏 ⑤a 保护（负例）',
      !depRun.out.includes('@alioth/i18n') && !depRun.out.includes('@alioth/utils'),
      `rc=${depRun.code} ${depRun.out.trim().slice(0, 200)}`,
    );
    rmSync(dep, { recursive: true, force: true });

    // ⑥ target 通道（ns 仓自持安装面）：平台 manifest 无该 ns 成员，ns 仓根自有 workspace+lock ⇒ 零违规；
    //    移除 ns 侧成员行 ⇒ 命中「成员未入供给面」（负例，R1：证明放宽未掩护真缺失）。
    const tgt = mkdtempSync(path.join(tmpdir(), 'app-isolation-nschannel-'));
    const tns = 'Tgt';
    const nsRepo = path.join(tgt, `Alioth-NS-${tns}`);
    const tMountReal = path.join(nsRepo, 'Pre-Proc', tns);
    const tUnit = path.join(tMountReal, 'Sources', 'Apps', 'Modules', 'm1', 'frontend');
    mkdirSync(path.join(tMountReal, 'Apps', 'demo'), { recursive: true });
    mkdirSync(tUnit, { recursive: true });
    writeFileSync(path.join(tMountReal, 'Cargo.toml'), '[workspace]\n');
    writeFileSync(path.join(tMountReal, 'Apps/demo/app.json'), JSON.stringify({ config: { modules: ['m1'], blocks: [] } }));
    writeFileSync(path.join(tUnit, 'package.json'), JSON.stringify({ name: '@tgt/m1-frontend' }));
    // 平台面：收口后形态（无该 ns 成员）；ns 面经符号链接挂载（`nsRepoOf` 形态前置）
    mkdirSync(path.join(tgt, 'Pre-Proc'), { recursive: true });
    symlinkSync(tMountReal, path.join(tgt, 'Pre-Proc', tns));
    writeFileSync(path.join(tgt, 'pnpm-workspace.yaml'), 'packages: []\n');
    writeFileSync(path.join(tgt, 'pnpm-lock.yaml'), "lockfileVersion: '9.0'\nimporters: {}\n");
    const nsMemberRel = `Pre-Proc/${tns}/Sources/Apps/Modules/m1/frontend`;
    writeFileSync(path.join(nsRepo, 'pnpm-workspace.yaml'), `packages:\n  - '${nsMemberRel}'\n`);
    writeFileSync(path.join(nsRepo, 'pnpm-lock.yaml'), `lockfileVersion: '9.0'\nimporters:\n  ${nsMemberRel}: {}\n`);
    const nsCh = run(tgt);
    check(
      '⑥ target 通道（ns 自持安装面）零违规',
      nsCh.code === 0 && nsCh.out.includes('通道 ns'),
      `rc=${nsCh.code} ${nsCh.out.trim().slice(0, 220)}`,
    );
    writeFileSync(path.join(nsRepo, 'pnpm-workspace.yaml'), 'packages: []\n');
    const nsChNeg = run(tgt);
    check(
      '⑥ 负例：ns 侧成员缺失 ⇒ 命中「成员未入供给面」',
      nsChNeg.code === 1 && nsChNeg.out.includes('成员未入供给面'),
      `rc=${nsChNeg.code} ${nsChNeg.out.trim().slice(0, 220)}`,
    );
    rmSync(tgt, { recursive: true, force: true });
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
  SINK.out(failed === 0 ? `✅ check-app-frontend-isolation --self-test 通过（${total} 例）` : `❌ --self-test 失败 ${failed}/${total}`);
  return failed === 0 ? 0 : 1;
}


function yqLines(expr: string, file: string, cwd: string = root): string[] {
  const p = Bun.spawnSync(['yq', '-r', expr, file], { cwd });
  if (p.exitCode !== 0) {
    SINK.err(`[app-isolation] yq 读取失败：${file}（${new TextDecoder().decode(p.stderr).trim()}）`);
    exit(2);
  }
  return new TextDecoder()
    .decode(p.stdout)
    .split('\n')
    .map((s) => s.trim())
    .filter((s) => s.length > 0 && s !== 'null');
}

type Violation = { ns: string; kind: string; item: string; fix: string };
type AppReport = { app: string; modules: number; blocks: number; members: number; inWorkspace: number; inLock: number };
type NsReport = { ns: string; channel: string; apps: AppReport[]; unattributed: string[]; noFrontend: string[]; violations: Violation[] };

/** 跨 ns / 全局判据（包名唯一、成员行唯一）的归集面——判据天然跨 ns，不属任一 ns 的报告行。 */
let globalViolations: Violation[] = [];
/** `packageName → ['ns:kind/unit/frontend']`：包名跨 ns/App 唯一性判据面。 */
let packageNameIndex = new Map<string, string[]>();

interface AppIsolationOutcome {
  out: string[];
  err: string[];
  rc: number;
}

/** 判定面（ns App 前端隔离：workspace 成员单源 / 包名与成员行唯一）；输出经 SINK 记录，报告面回放。 */
export function appFrontendIsolationState(ns: string | null, jsonMode: boolean, failMode: boolean): AppIsolationOutcome {
  const out: string[] = [];
  const err: string[] = [];
  const saved = SINK;
  SINK = { out: (l) => out.push(l), err: (l) => err.push(l) };
  onlyNs = ns;
  jsonOut = jsonMode;
  failOnViolation = failMode;
  globalViolations = [];
  packageNameIndex = new Map<string, string[]>();
  try {
// ── 供给通道（单通道 = ns 仓自持安装面；ADR D-046 修订 2026-10-05）──────────────────────────
//  成员/入账判定 MUST 读 `<nsRepo>/pnpm-workspace.yaml` + `<nsRepo>/pnpm-lock.yaml`；
//  平台侧历史通道（平台 workspace 显式成员块 `../Alioth-NS-{ns}/…`）已退役、MUST NOT 复活
//  （平台清单面零 ns 内容由 `platform-ns-dependency-direction` 守护），本判据不再接受该通道。
const platformRoot = portsTableRoot(root) ?? root;
const hasPlatformManifest = fs.existsSync(path.join(platformRoot, 'pnpm-workspace.yaml'));
const allMembers = hasPlatformManifest ? yqLines('.packages[]', 'pnpm-workspace.yaml', platformRoot) : [];

/** ns 仓自持安装面（唯一供给通道；ADR D-046 修订 2026-10-05）：ns 仓根无 `pnpm-workspace.yaml` ⇒ null（判据面缺失，逐成员报「成员未入供给面」）。 */
function nsOwnedChannel(nsMount: string): { members: Set<string>; importers: Set<string> } | null {
  let repo: string;
  try {
    repo = nsRepoOf(nsMount);
  } catch {
    return null;
  }
  if (!fs.existsSync(path.join(repo, 'pnpm-workspace.yaml'))) return null;
  const members = new Set(yqLines('.packages[]', 'pnpm-workspace.yaml', repo));
  const importers = new Set(
    fs.existsSync(path.join(repo, 'pnpm-lock.yaml')) ? yqLines('.importers | keys[]', 'pnpm-lock.yaml', repo) : [],
  );
  return { members, importers };
}

/**
 * ns 清单**机械派生**（双源并集，两者皆空才 fail-loud）：
 *   ① 同级 `Alioth-NS-*` 仓目录（开发机常态：ns 仓与平台仓同级）；
 *   ② `Pre-Proc/{ns}` 符号链接挂载（平台仓规约的挂载面；`scripts/lib/ns-mounts.ts` 单一实现）。
 *
 * 为什么必须并集（2026-10-04 实测）：验证器 push 档在临时 worktree 内只软链主检出的
 * `Pre-Proc/{ns}`（同级无 `Alioth-NS-*`）⇒ 仅按同级派生会让 ns 清单为空并 rc=2
 * 「判据面失效」，与「平台仓的 ns 身份 = 挂载」这一既有规约相抵。
 * 枚举为空 ⇒ fail-loud（不得静默判 0 App 后「通过」）。
 */
const siblingNs = fs
  .readdirSync(path.join(root, '..'), { withFileTypes: true })
  .filter((e) => (e.isDirectory() || e.isSymbolicLink()) && e.name.startsWith('Alioth-NS-'))
  .map((e) => e.name.slice('Alioth-NS-'.length));
const nsCandidates = [...new Set([...siblingNs, ...mountedNs(root).map((d) => path.basename(d))])].sort();
if (nsCandidates.length === 0 && onlyNs === null) {
  SINK.err(
    '[app-isolation] 判据面失效：同级无 `Alioth-NS-*` 仓目录且 `Pre-Proc/` 无 ns 挂载（ns 清单机械派生为空）——MUST NOT 判通过',
  );
  exit(2);
}
/**
 * 判据 ⑤（声明面 ↔ 使用面一致）的检测器——**双向非对称是刻意的**（change
 * prune-ns-frontend-dep-declarations · design.md §D2）：
 *   ⑤a 声明 → 使用：**文本命中**（单元内除自身 package.json 外全部文本文件；任一处命中即在用）
 *       —— 保守取向：误报的代价是删掉真在用的声明，故有疑即视为在用。
 *   ⑤b 使用 → 声明：**import 提取**（`Bun.Transpiler.scanImports`，按扩展名 ts/tsx/js）
 *       —— 精确取向：文本面会被注释/文档里的包名触发假红。
 */
const UNIT_TEXT_EXT = /\.(tsx?|jsx?|mjs|cjs|json|html|md|css)$/;
/** 单元遍历排除集 = 规范集（`scripts/check/lib/gate-kit.ts` 单一属主）+ 本判据特有附加（显式参数传入派生，MUST NOT 本地副本）。 */
const UNIT_EXCLUDE_DIRS: Record<string, true> = {
  ...CANON_EXCLUDE_DIRS,
  '.turbo': true,
  '.vite': true,
  build: true,
  coverage: true,
};
const ALIOTH_TOKEN = /@alioth\/[a-z0-9-]+/g;
const transpilers = new Map<string, Bun.Transpiler>();

function transpilerFor(loader: 'ts' | 'tsx' | 'js'): Bun.Transpiler {
  let t = transpilers.get(loader);
  if (t === undefined) {
    t = new Bun.Transpiler({ loader });
    transpilers.set(loader, t);
  }
  return t;
}

/** import 说明符 → 包名（`@scope/name` 取前两段；相对/绝对/内建返回 null）。 */
function specifierPackage(spec: string): string | null {
  if (spec === '' || spec.startsWith('.') || spec.startsWith('/') || spec.startsWith('~') || spec.startsWith('node:')) return null;
  const parts = spec.split('/');
  return spec.startsWith('@') ? parts.slice(0, 2).join('/') : parts[0];
}

/** 单次遍历单元：同时收集 ⑤a 的文本命中面与 ⑤b 的 import 面（避免两次 walk）。 */
function scanUnitUsage(unitDir: string, selfPkg: string): { referenced: Set<string>; imported: Map<string, string> } {
  const referenced = new Set<string>();
  const imported = new Map<string, string>();
  const walk = (dir: string): void => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) {
        if (!isExcludedDirName(e.name, UNIT_EXCLUDE_DIRS)) walk(path.join(dir, e.name));
        continue;
      }
      const f = path.join(dir, e.name);
      if (!UNIT_TEXT_EXT.test(e.name) || f === selfPkg) continue;
      const text = fs.readFileSync(f, 'utf8');
      for (const m of text.matchAll(ALIOTH_TOKEN)) referenced.add(m[0]);
      if (!/\.(tsx?|jsx?|mjs|cjs)$/.test(e.name)) continue;
      const loader = e.name.endsWith('.ts') ? 'ts' : e.name.endsWith('.tsx') ? 'tsx' : 'js';
      let recs: { path: string }[];
      try {
        recs = transpilerFor(loader).scanImports(text) as { path: string }[];
      } catch {
        continue; // 解析失败不静默判「无机用」：仅跳过该文件的 import 面（文本面已计入）
      }
      for (const r of recs) {
        const p = specifierPackage(r.path);
        if (p !== null && p.startsWith('@alioth/') && !imported.has(p)) imported.set(p, path.relative(unitDir, f));
      }
    }
  };
  walk(unitDir);
  return { referenced, imported };
}

const reports: NsReport[] = [];

for (const ns of onlyNs ? [onlyNs] : nsCandidates) {
  // ns 内容根 = `Pre-Proc/{ns}` 挂载（双档位一致；同级存在性不再是前置）
  const nsMount = path.join(root, 'Pre-Proc', ns);
  if (!fs.existsSync(nsMount)) continue; // 未挂载：跳过并在报告中缺席
  const appsDir = path.join(nsMount, 'Apps');
  if (!fs.existsSync(appsDir)) continue;

  const violations: Violation[] = [];
  const apps: AppReport[] = [];
  const noFrontend: string[] = [];
  const attributed = new Map<string, string[]>();
  // 供给通道 = ns 仓自持安装面（唯一；平台成员块通道已退役）
  const nsChannel = nsOwnedChannel(nsMount);
  const nsChannelLabel = nsChannel === null ? 'ns-missing' : 'ns';

  // ── App 声明面（单一实现 `scripts/lib/ns-frontend-intake.ts`）────────────────────────────
  // 声明∩磁盘、诊断 code 均来自单一源；本判据只决定**严宽**（Module 缺目录=违规 / Block 无前端=info）。
  const intakeRes = intake(root, { ns, layout: 'full' });
  /** declared ∧ 磁盘存在 的单元面：成员/lock 入账判定 MUST 只覆盖这一面
   *  （原实现对「声明而磁盘缺」提前 return —— 语义保持不变：缺目录由 `unit-frontend-missing` 单独判定）。 */
  const onDiskUnits = new Set(intakeRes.units.map((u) => `${u.kind}/${u.unit}`)); // = frontend/ 目录存在（判据成员计数基准）
  for (const decl of intakeRes.apps) {
    const mods = decl.modules;
    const blks = decl.blocks;
    for (const m of mods) attributed.set(`Modules/${m}`, [...(attributed.get(`Modules/${m}`) ?? []), decl.app]);
    for (const b of blks) attributed.set(`Blocks/${b}`, [...(attributed.get(`Blocks/${b}`) ?? []), decl.app]);

    let members = 0;
    let inWorkspace = 0;
    let inLock = 0;
    const checkMember = (kind: 'Modules' | 'Blocks', name: string) => {
      if (!onDiskUnits.has(`${kind}/${name}`)) return;
      members += 1;
      const nsRel = `Pre-Proc/${ns}/Sources/Apps/${kind}/${name}/frontend`;
      const inWorkspaceNow = nsChannel?.members.has(nsRel) === true || nsChannel?.members.has(`./${nsRel}`) === true;
      const inLockNow = nsChannel?.importers.has(nsRel) === true || nsChannel?.importers.has(`./${nsRel}`) === true;
      if (inWorkspaceNow) inWorkspace += 1;
      else
        violations.push({
          ns,
          kind: '成员未入供给面（ns 仓自持安装面）',
          item: nsRel,
          fix: '接入供给通道：ns 侧生成器（`bun scripts/ns/ns-frontend-workspace.ts sync --write`；ns 仓根 `pnpm-workspace.yaml`）后于 ns 仓根 `pnpm install`；平台成员块通道已退役，MUST NOT 手改 yaml',
        });
      if (inLockNow) inLock += 1;
      else
        violations.push({
          ns,
          kind: '依赖图未入账（跨 App 兜底风险）',
          item: nsRel,
          fix: '在 ns 仓根执行 `pnpm install` 生成 lock importer（ns 自持 store；MUST NOT 由他 App / 平台面兜底解析）',
        });
    };
    for (const m of mods) checkMember('Modules', m);
    for (const b of blks) checkMember('Blocks', b);
    apps.push({ app: decl.app, modules: mods.length, blocks: blks.length, members, inWorkspace, inLock });
  }

  // 声明∧磁盘 差异：Module 声明而无前端=违规；Block 无前端面=info（口径 2026-10-02 实测校正，保持不变）。
  for (const w of intakeRes.warnings) {
    if (w.code !== 'unit-frontend-missing') continue;
    if (w.kind === 'Blocks') noFrontend.push(`${w.app} → ${w.kind}/${w.unit}`);
    else
      violations.push({
        ns,
        kind: '声明的前端不存在',
        item: `${w.app} → ${w.kind}/${w.unit}`,
        fix: '修正 app.json 声明或补齐前端目录',
      });
  }

  const unattributed: string[] = intakeRes.warnings
    .filter((w) => w.code === 'unit-unassigned')
    .map((w) => `${w.kind}/${w.unit}`)
    .sort();
  for (const u of unattributed) {
    violations.push({
      ns,
      kind: '前端未归属任何 App（隔离特征不可计算）',
      item: u,
      fix: '在拥有该前端的 App 的 app.json config.modules/config.blocks 中声明（或确认该前端退役后删除）',
    });
  }

  // 包名索引（跨 ns/App 唯一性判据面）：只看有 `package.json` 的前端单元（有无前端面已由上面判）。
  // 判据 ⑤（声明面 ↔ 使用面一致）复用同一遍历——同面判据 MUST 共用单元枚举，不另建实现。
  for (const kind of ['Modules', 'Blocks'] as const) {
    // ⑤/包名判据面 MUST 覆盖**磁盘上全部**有 package.json 的前端单元（含未声明者）——
    // 与成员计数面（declared ∧ 目录存在）刻意不同：未归属单元同样要判死声明/未声明引用/包名唯一性。
    for (const unitName of listUnitNames(nsMount, kind, 'apps')) {
      const unitRoot = resolveUnitDir(nsMount, kind, unitName, 'apps');
      if (unitRoot === null) continue;
      const pkg = path.join(unitRoot, 'frontend', 'package.json');
      if (!fs.existsSync(pkg)) continue;
      const d = { name: unitName };
      let parsed: unknown;
      try {
        parsed = JSON.parse(fs.readFileSync(pkg, 'utf8'));
      } catch {
        continue;
      }
      if (typeof parsed !== 'object' || parsed === null) continue;

      // ── 判据 ⑤：`@alioth/*` 声明面 MUST 与使用面一致（双向）────────────────────
      const obj = parsed as { dependencies?: Record<string, string>; devDependencies?: Record<string, string> };
      const declaredProd = Object.keys(obj.dependencies ?? {});
      const declaredDev = Object.keys(obj.devDependencies ?? {});
      const declaredAll = new Set([...declaredProd, ...declaredDev]);
      const usage = scanUnitUsage(path.join(unitRoot, 'frontend'), pkg);
      for (const dep of [...declaredProd, ...declaredDev]) {
        if (!dep.startsWith('@alioth/') || usage.referenced.has(dep)) continue;
        violations.push({
          ns,
          kind: '死声明（声明未使用）',
          item: `${kind}/${d.name} → ${dep}`,
          fix: '删除该声明：单元内除 package.json 外无任何引用；平台共享包 MUST 按使用面声明，MUST NOT 以「预留」为由保留',
        });
      }
      for (const [dep, where] of usage.imported) {
        if (declaredAll.has(dep)) continue;
        violations.push({
          ns,
          kind: '未声明引用（import 但未声明）',
          item: `${kind}/${d.name} → ${dep}（${where}）`,
          fix: '在 package.json 补声明（构建期依赖入 devDependencies）；MUST NOT 依赖提升（hoisting）侥幸解析',
        });
      }

      if (!('name' in parsed)) continue;
      const name: unknown = parsed.name;
      if (typeof name !== 'string' || name.length === 0) continue;
      packageNameIndex.set(name, [...(packageNameIndex.get(name) ?? []), `${ns}:${kind}/${d.name}/frontend`]);
    }
  }

  reports.push({ ns, channel: nsChannelLabel, apps, unattributed, noFrontend, violations });
}

// 全局判据（不属任一 ns 的报告行）：① 包名跨 ns/App 唯一；② 平台 workspace 成员行唯一。
for (const [name, places] of packageNameIndex) {
  if (places.length < 2) continue;
  globalViolations.push({
    ns: '*',
    kind: '包名跨 ns/App 冲突',
    item: `${name} → ${places.join(' , ')}`,
    fix: '包名 MUST 携带归属 ns 前缀（`@<ns-lower>/…`）——跨 ns 同名**模块目录**合法，包名撞名不合法',
  });
}
const memberCounts = new Map<string, number>();
for (const m of allMembers) memberCounts.set(m, (memberCounts.get(m) ?? 0) + 1);
for (const [m, n] of memberCounts) {
  if (n < 2) continue;
  globalViolations.push({
    ns: '*',
    kind: 'workspace 成员行重复',
    item: `${m}（×${n}）`,
    fix: '成员行 MUST 唯一：ns 成员由 `bun scripts/ns/ns-frontend-workspace.ts sync --write` 生成；平台清单 MUST NOT 记录 ns 成员（此处重复行 = 手改痕迹，直接删除）',
  });
}

if (jsonOut) {
  SINK.out(JSON.stringify({ reports, global: globalViolations }, null, 2));
} else {
  for (const r of reports) {
    const members = r.apps.reduce((n, a) => n + a.members, 0);
    const wsOk = r.apps.reduce((n, a) => n + a.inWorkspace, 0);
    const lockOk = r.apps.reduce((n, a) => n + a.inLock, 0);
    SINK.out(
      `[app-isolation] ${r.ns}: ${r.apps.length} App / 成员 ${members}（workspace ${wsOk} / lock ${lockOk}）` +
        ` | 通道 ${r.channel} | 未归属 ${r.unattributed.length} | 无前端面(Block) ${r.noFrontend.length} | 违规 ${r.violations.length}`,
    );
    for (const v of r.violations.slice(0, 40)) SINK.out(`  ✗ [${v.kind}] ${v.item}\n    → ${v.fix}`);
    if (r.violations.length > 40) SINK.out(`  … 其余 ${r.violations.length - 40} 条见 --json`);
  }
  if (globalViolations.length > 0) {
    SINK.out(`[app-isolation] 全局（跨 ns / 平台清单）：${globalViolations.length} 处`);
    for (const v of globalViolations.slice(0, 40)) SINK.out(`  ✗ [${v.kind}] ${v.item}\n    → ${v.fix}`);
  } else {
    SINK.out('[app-isolation] 全局（跨 ns / 平台清单）：无违规');
  }
}

const total =
  reports.reduce((n, r) => n + r.violations.length, 0) + globalViolations.length;
if (failOnViolation && total > 0) {
  SINK.err(`[app-isolation] 违规合计 ${total} 条（--fail）`);
  exit(1);
}
  } catch (e) {
    if (!(e instanceof ExitSignal)) throw e;
    return { out, err, rc: e.rc };
  } finally {
    SINK = saved;
  }
  return { out, err, rc: 0 };
}

const gate = defineCheck({
  id: 'app-frontend-isolation',
  label: 'app-frontend-isolation',
  valueFlags: ['--ns'],
  predicate: () => [],
  report: (ctx, { fail, json }, io) => {
    const s = appFrontendIsolationState(ctx.valueFlags.get('--ns')?.[0] ?? null, json, fail);
    for (const line of s.out) io.out(`${line}\n`);
    for (const line of s.err) io.err(`${line}\n`);
    return s.rc;
  },
  selfTest() {
    return selfTest(); // 既有自检（子进程注入 APP_ISOLATION_ROOT 夹具；输出经默认 SINK 直出）
  },
});

if (import.meta.main) process.exit(await gate.main());
