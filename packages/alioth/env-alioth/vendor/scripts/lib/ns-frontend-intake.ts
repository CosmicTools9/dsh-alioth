#!/usr/bin/env node
/**
 * ns-frontend-intake.ts — 「App 前端单元清单」的**单一实现**（change `unify-ns-frontend-intake`，架构候选 C3）。
 *
 * 单一事实：`Pre-Proc/{ns}/Apps/<app>/app.json` 的 `config.{modules,blocks}`（声明面）∩ 磁盘（`frontend/package.json`）。
 * 布局回退链 MUST 复用既有单一源 `scripts/lib/preproc-layout.mjs`（`Sources/Apps` 优先 → 扁平 `Sources`）。
 *
 * 消费者（MUST 同源消费；MUST NOT 各自遍历 app.json / 各自探测 single-spa）：
 *   · 成员生成器 `scripts/ns/ns-frontend-workspace.ts`（layout: 'apps'）
 *   · 隔离判据     `scripts/check/check-app-frontend-isolation.ts`（layout: 'full'）
 *   · 平台 dev 配置 `Gateway/frontend/vite.config.ts`（layout: 'full'）
 *
 * 本 module **只产出事实 + 结构化诊断**，不替消费者决定严宽（gate 可把 `unit-frontend-missing` 升级为违规，
 * 生成器可降级为 warn）——判据留消费者，事实单源。
 *
 * Node-only（`node:fs` / `node:path`）：Gateway 的 vite 配置在 Node 下加载（dev 与 `vite build` 皆然）。
 */
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { nsSourcesDir } from './preproc-layout.mjs';
import { mountedNs } from './ns-mounts';

export type UnitKind = 'Modules' | 'Blocks';

export const UNIT_KINDS: readonly UnitKind[] = ['Modules', 'Blocks'];

export type IntakeCode =
  | 'no-apps-dir'
  | 'app-json-unparsable'
  | 'unit-frontend-missing'
  | 'unit-frontend-no-package-json'
  | 'unit-unassigned'
  | 'unit-shared-across-apps';

export interface IntakeDiag {
  code: IntakeCode;
  /** 声明它的 App（未归属/无 App 语境为 null）。 */
  app: string | null;
  kind: UnitKind | null;
  unit: string | null;
  /** 人类可读补充（消费者可自行改写文案；判据 MUST 基于 code）。 */
  detail: string;
  /** `unit-unassigned` 专用：该未归属前端是否有 `package.json`（成员面消费者按 true 过滤）。 */
  hasPackageJson?: boolean;
}

export interface IntakeUnit {
  /** 声明它的 App（同单元被多 App 声明时取排序后首个；全量归属见 `attributedBy`）。 */
  app: string;
  kind: UnitKind;
  unit: string;
  /** 单元目录（绝对）。 */
  dir: string;
  /** 平台仓相对路径：`Pre-Proc/{ns}/Sources/Apps/{kind}/{unit}/frontend`（成员面用）。 */
  rel: string;
  /** `frontend/src/single-spa.tsx` 绝对路径；缺 ⇒ null（消费方决定处置）。 */
  entry: string | null;
  /** `frontend/package.json` 是否存在——**两个存在性基准的差别在此显式化**：
   *  成员面（ns workspace 生成器）以 package.json 为基准；隔离判据的成员计数以 `frontend/` 目录为基准。 */
  hasPackageJson: boolean;
}

export interface IntakeOptions {
  /** 限定 namespace；null/缺省 = 枚举全部挂载 ns。 */
  ns?: string | null;
  /**
   * 布局回退策略：`'apps'` = 仅 `Sources/Apps/{kind}`（成员面，保持生成产物逐字不变）；
   * `'full'` = 再加 `Sources/Open/{kind}`（dev/判据面）。两者共享 `Sources` 扁平回退（preproc-layout 单一源）。
   */
  layout?: 'apps' | 'full';
}

export interface IntakeAppDeclaration {
  app: string;
  modules: string[];
  blocks: string[];
}

export interface IntakeResult {
  ns: string | null;
  /** per-App 声明摘要（保序）——供需要「按 App 计数/逐 App 走查」的消费者复用，MUST NOT 自行二次解析 app.json。 */
  apps: IntakeAppDeclaration[];
  units: IntakeUnit[];
  warnings: IntakeDiag[];
  errors: IntakeDiag[];
  /** `kind/unit` → 声明它的 App 列表（排序），供「跨 App 共享」判据消费。 */
  attributedBy: Map<string, string[]>;
}

const APP_JSON = 'app.json';
const FE_DIR = 'frontend';
const FE_PKG = 'package.json';
const SPA_ENTRY = path.posix.join('src', 'single-spa.tsx');

function isDir(p: string): boolean {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
}

/**
 * 目录名列表（**跟随符号链接**——`Pre-Proc/{ns}` 是挂载软链，`withFileTypes().isDirectory()` 对其为 false：
 * 本仓 `ns-mounts`/`preproc-layout` 的 lstat 陷阱在同一批；此处以 `statSync` 复核）。
 */
function listDirNames(p: string): string[] {
  let entries: string[];
  try {
    entries = readdirSync(p);
  } catch {
    return [];
  }
  const out: string[] = [];
  for (const name of entries) {
    try {
      if (statSync(path.join(p, name)).isDirectory()) out.push(name);
    } catch {
      continue;
    }
  }
  return out.sort();
}

/** ns 前端单元目录候选（按序）：`<Sources>/<kind>/<unit>`（Apps→扁平由 preproc-layout 决定）+ 可选 `Open`。 */
export function unitDirCandidates(
  nsRoot: string,
  kind: UnitKind,
  unit: string,
  layout: 'apps' | 'full' = 'full',
): string[] {
  const sources = nsSourcesDir(nsRoot);
  const out = [path.join(sources, kind, unit)];
  if (layout === 'full') out.push(path.join(nsRoot, 'Sources', 'Open', kind, unit));
  return out;
}

/** 入口规则**唯一实现**：`<frontendDir>/src/single-spa.tsx` 存在则返回其路径，否则 null。
 *  动态解析方（dev 运行时 resolver / 清单插件）MUST 经本函数探测，MUST NOT 自拼该字面。 */
export function unitEntry(frontendDir: string): string | null {
  const p = path.join(frontendDir, SPA_ENTRY);
  return existsSync(p) ? p : null;
}

/** 某类单元根目录（`Sources/Apps/{kind}` 优先、扁平 `Sources/{kind}` 回退——preproc-layout 单一源）。 */
export function kindDir(nsRoot: string, kind: UnitKind, layout: 'apps' | 'full' = 'apps'): string {
  const sources = nsSourcesDir(nsRoot);
  if (layout === 'full' && !existsSync(path.join(sources, kind))) {
    const openKind = path.join(nsRoot, 'Sources', 'Open', kind);
    if (isDir(openKind)) return openKind;
  }
  return path.join(sources, kind);
}

/** 解析单元目录（首个存在者）；不存在 ⇒ null。 */
export function resolveUnitDir(
  nsRoot: string,
  kind: UnitKind,
  unit: string,
  layout: 'apps' | 'full' = 'full',
): string | null {
  for (const c of unitDirCandidates(nsRoot, kind, unit, layout)) if (isDir(c)) return c;
  return null;
}

/** 枚举某 ns 某类的全部单元名（Apps→扁平→Open，按名去重、先到先得）。 */
export function listUnitNames(
  nsRoot: string,
  kind: UnitKind,
  layout: 'apps' | 'full' = 'full',
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  const roots =
    layout === 'full'
      ? [
          nsSourcesDir(nsRoot),
          path.join(nsRoot, 'Sources', 'Apps'),
          path.join(nsRoot, 'Sources', 'Open'),
        ]
      : [path.join(nsRoot, 'Sources', 'Apps'), nsSourcesDir(nsRoot)];
  for (const root of roots) {
    for (const name of listDirNames(path.join(root, kind))) {
      if (seen.has(name)) continue;
      seen.add(name);
      out.push(name);
    }
  }
  return out;
}

interface AppDeclaration {
  app: string;
  modules: string[];
  blocks: string[];
}

/** 读取 `Apps/<app>/app.json` 的 `config.{modules,blocks}`（保序、去空串）。 */
export function readAppDeclarations(
  appsDir: string,
  ns: string | null,
): { apps: AppDeclaration[]; warnings: IntakeDiag[] } {
  const warnings: IntakeDiag[] = [];
  const apps: AppDeclaration[] = [];
  if (!isDir(appsDir)) {
    warnings.push({
      code: 'no-apps-dir',
      app: null,
      kind: null,
      unit: null,
      detail: `${ns ?? appsDir}: 无 Apps/ 目录（App 声明面缺失）`,
    });
    return { apps, warnings };
  }
  for (const app of listDirNames(appsDir)) {
    const appJson = path.join(appsDir, app, APP_JSON);
    if (!existsSync(appJson)) continue;
    let cfg: unknown;
    try {
      cfg = JSON.parse(readFileSync(appJson, 'utf8'));
    } catch (e) {
      warnings.push({
        code: 'app-json-unparsable',
        app,
        kind: null,
        unit: null,
        detail: `${ns ?? ''}: ${app}/${APP_JSON} 解析失败（${(e as Error).message}）`,
      });
      continue;
    }
    const config =
      cfg !== null && typeof cfg === 'object' ? (cfg as { config?: unknown }).config : null;
    const pick = (k: string): string[] => {
      if (config === null || typeof config !== 'object') return [];
      const v = (config as Record<string, unknown>)[k];
      return Array.isArray(v)
        ? v.filter((x): x is string => typeof x === 'string' && x !== '')
        : [];
    };
    apps.push({ app, modules: pick('modules'), blocks: pick('blocks') });
  }
  return { apps, warnings };
}

/** 挂载的 ns 清单——**委派既有单一源** `scripts/lib/ns-mounts.ts`（挂载判据含 lstat 语义；
 *  MUST NOT 用 `readdir(Pre-Proc)` 自造清单：`Pre-Proc` 下存在测试残留目录（如 `AppAgentUt-*`）。 */
export function mountedNamespaces(platformRoot: string): string[] {
  // `mountedNs` 返回挂载目录的**绝对路径**（与 `check-app-frontend-isolation` 同款归一）
  return mountedNs(platformRoot).map((d) => path.basename(d));
}

/** App 前端单元 intake（单一事实 + 结构化诊断）。 */
export function intake(platformRoot: string, opts: IntakeOptions = {}): IntakeResult {
  const layout = opts.layout ?? 'full';
  const nsList = opts.ns ? [opts.ns] : mountedNamespaces(platformRoot);
  const units: IntakeUnit[] = [];
  const allApps: IntakeAppDeclaration[] = [];
  const warnings: IntakeDiag[] = [];
  const errors: IntakeDiag[] = [];
  const attributedBy = new Map<string, string[]>();
  const nsSeen: string | null = opts.ns ?? (nsList.length === 1 ? nsList[0] : null);

  for (const ns of nsList) {
    const nsMount = path.join(platformRoot, 'Pre-Proc', ns);
    if (!isDir(nsMount)) continue;
    const appsDir = path.join(nsMount, 'Apps');
    const { apps, warnings: declWarnings } = readAppDeclarations(appsDir, ns);
    warnings.push(...declWarnings);
    allApps.push(...apps);

    const declared = new Set<string>();
    for (const decl of apps) {
      const perKind: Array<[UnitKind, string[]]> = [
        ['Modules', decl.modules],
        ['Blocks', decl.blocks],
      ];
      for (const [kind, list] of perKind) {
        for (const unit of list) {
          const key = `${kind}/${unit}`;
          attributedBy.set(key, [...(attributedBy.get(key) ?? []), decl.app]);
          const dir = resolveUnitDir(nsMount, kind, unit, layout);
          const feDir = dir === null ? null : path.join(dir, FE_DIR);
          if (dir === null || feDir === null || !isDir(feDir)) {
            // 事实：声明了但磁盘无前端**目录**。严宽由消费者定（gate: Module=违规 / 生成器: 非成员）。
            warnings.push({
              code: 'unit-frontend-missing',
              app: decl.app,
              kind,
              unit,
              detail: `${ns}: ${decl.app} 声明 ${kind}/${unit} 但 frontend/ 目录不存在（跳过）`,
            });
            continue;
          }
          const hasPkg = existsSync(path.join(feDir, FE_PKG));
          if (!hasPkg) {
            warnings.push({
              code: 'unit-frontend-no-package-json',
              app: decl.app,
              kind,
              unit,
              detail: `${ns}: ${decl.app} 声明 ${kind}/${unit}，frontend/ 存在但无 package.json（非成员面；判据的成员计数仍计入）`,
            });
          }
          if (declared.has(key)) continue; // 同一 App 内重复声明：只入一次
          declared.add(key);
          const entryAbs = unitEntry(feDir);
          units.push({
            app: decl.app,
            kind,
            unit,
            dir: feDir,
            rel: path.posix.join('Pre-Proc', ns, 'Sources', 'Apps', kind, unit, 'frontend'),
            entry: entryAbs,
            hasPackageJson: hasPkg,
          });
        }
      }
    }

    // 未归属前端（磁盘有 package.json 但无 App 声明）：不在成员面内；gate fail-loud、生成器 warn。
    for (const kind of UNIT_KINDS) {
      for (const unit of listUnitNames(nsMount, kind, layout)) {
        if (declared.has(`${kind}/${unit}`)) continue;
        const dir = resolveUnitDir(nsMount, kind, unit, layout);
        if (dir === null || !isDir(path.join(dir, FE_DIR))) continue;
        const hasPkg = existsSync(path.join(dir, FE_DIR, FE_PKG));
        warnings.push({
          code: 'unit-unassigned',
          app: null,
          kind,
          unit,
          detail: `${ns}: ${kind}/${unit} 有前端但无 App 声明（未入成员面；先补 app.json 声明）`,
          hasPackageJson: hasPkg,
        });
      }
    }
  }

  for (const [key, apps] of attributedBy) {
    const uniq = [...new Set(apps)];
    if (uniq.length > 1) {
      const [kind, unit] = key.split('/') as [UnitKind, string];
      warnings.push({
        code: 'unit-shared-across-apps',
        app: uniq.join(','),
        kind,
        unit,
        detail: `${key} 被多个 App 声明（${uniq.join(', ')}）——按 App 隔离域各装一份`,
      });
    }
  }

  units.sort((a, b) =>
    a.kind === b.kind ? a.unit.localeCompare(b.unit) : a.kind.localeCompare(b.kind),
  );
  return { ns: nsSeen, apps: allApps, units, warnings, errors, attributedBy };
}



/** dev SPA 别名派生（`@alioth/<unit>-frontend/single-spa` → 单元入口文件）。 */
export function spaAliases(
  result: IntakeResult,
  extra: readonly IntakeUnit[] = [],
): Array<{ find: string; replacement: string }> {
  const out: Array<{ find: string; replacement: string }> = [];
  const seen = new Set<string>();
  for (const u of [...result.units, ...extra]) {
    if (u.entry === null) continue;
    const find = `@alioth/${u.unit}-frontend/single-spa`;
    if (seen.has(find)) continue;
    seen.add(find);
    out.push({ find, replacement: u.entry });
  }
  return out.sort((a, b) => a.find.localeCompare(b.find));
}

/**
 * 模块名 → `{ ns, rel }`（仅入口存在的模块；`rel` = 平台仓相对的前端目录，
 * 形如 `Pre-Proc/{ns}/Sources/{布局}/{kind}/{unit}/frontend`）。
 * **单一实现**：平台壳（Gateway/Meta/OpenActivity）的模块解析 MUST 消费本函数，
 * MUST NOT 各自 `readdirSync('Pre-Proc')` 拼 `<ns>/Sources/Modules`
 * （该扁平形态对已迁移 ns 恒不存在 ⇒ 映射静默为空、模块解析死掉）。
 * 消费方按 `rel` 做 importer 子串匹配（对 `Pre-Proc/{ns}` 软链的两种路径形态均成立）。
 */
export function moduleFrontends(result: IntakeResult): Record<string, { ns: string; rel: string }> {
  const map: Record<string, { ns: string; rel: string }> = {};
  for (const u of result.units) {
    if (u.kind !== 'Modules' || u.entry === null) continue;
    const ns = u.rel.split('/')[1];
    if (ns && !(u.unit in map)) map[u.unit] = { ns, rel: u.rel };
  }
  return map;
}

/** live index fact：模块前端的磁盘事实（入口/主题 CSS 探测规则 = 本模块唯一实现）。 */
export interface ModuleFrontendLiveFact {
  ns: string;
  /** 模块单元目录（经挂载软链的绝对路径）。 */
  dir: string;
  /** `frontend/src/single-spa.tsx` 入口（探测 = `unitEntry`）。 */
  entry: string;
  /** `frontend/src/theme.css`（存在才非 null；dist 产物面由消费方按需回退探测）。 */
  themeCss: string | null;
  /** 平台仓相对前端目录（importer 子串匹配口径，同 `moduleFrontends().rel`）。 */
  rel: string;
}

export interface ModuleFrontendsLive {
  /** miss 即重建一次再探——「启动后新增模块有路由、无 alias ⇒ SPA 白屏」不变量（2026-09-13）的唯一实现点。 */
  get(name: string): ModuleFrontendLiveFact | null;
  /** 当前快照模块名（不触发重建）。 */
  names(): string[];
  /** 强制重建（alias 表等静态面在配置装载点主动刷新）。 */
  refresh(): void;
}

/**
 * 磁盘全域模块前端 live index（含未声明模块）。平台壳（Gateway/Meta）的模块解析 MUST 消费本接口：
 * 壳内 resolver / 路由生成 / import 改写 / 别名表 MUST NOT 各自扫树建静态映射（静态映射覆盖不了
 * 启动后新增/补齐的模块）。`ns` 传入时只扫该 ns，否则扫挂载全集（`mountedNamespaces` 单一源）。
 */
export function moduleFrontendsLive(
  projectRoot: string,
  opts: { ns?: string | null } = {},
): ModuleFrontendsLive {
  const preproc = path.join(projectRoot, 'Pre-Proc');
  const namespaces = opts.ns ? [opts.ns] : mountedNamespaces(projectRoot);
  let cache = new Map<string, ModuleFrontendLiveFact>();
  const rebuild = (): void => {
    const next = new Map<string, ModuleFrontendLiveFact>();
    for (const ns of namespaces) {
      const nsRoot = path.join(preproc, ns);
      for (const name of listUnitNames(nsRoot, 'Modules', 'full')) {
        const dir = resolveUnitDir(nsRoot, 'Modules', name, 'full');
        if (dir === null) continue;
        const entry = unitEntry(path.join(dir, 'frontend'));
        if (entry === null) continue;
        const themeCss = path.join(dir, 'frontend', 'src', 'theme.css');
        const rel = path
          .relative(projectRoot, path.join(dir, 'frontend'))
          .split(path.sep)
          .join('/');
        next.set(name, { ns, dir, entry, themeCss: existsSync(themeCss) ? themeCss : null, rel });
      }
    }
    cache = next;
  };
  rebuild();
  return {
    get(name: string): ModuleFrontendLiveFact | null {
      const hit = cache.get(name);
      if (hit !== undefined) return hit;
      rebuild();
      return cache.get(name) ?? null;
    },
    names(): string[] {
      return [...cache.keys()];
    },
    refresh: rebuild,
  };
}
