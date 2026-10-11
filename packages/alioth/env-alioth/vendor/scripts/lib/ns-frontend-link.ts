#!/usr/bin/env node
/**
 * ns-frontend-link.ts — ns 前端对平台包的 `link:` **唯一实现**（架构候选 C4 · change `unify-ns-frontend-link-resolution`）。
 *
 * 同一事实（`@alioth/<pkg>` 在平台仓的落点 + 「相对于声明目录」的 link 写法）此前有**两份实现**：
 *   · 写侧 `scripts/ns/ns-frontend-workspace.ts`：`platformPackageIndex()` + `link:${relative(fromDir, target)}`
 *   · 验侧 `scripts/check/check-ns-frontend-dep-declarations.ts`：`resolveLinkName()`（解析目标 package.json 的 name）
 * 两侧各自实现相对路径计算 ⇒ 本 module 收口为：索引（name → 平台相对目录）、写规则、验规则，
 * 并以下述**往返不变量**把两侧钉在一起：`resolveLinkName(fromDir, linkSpecFor(fromDir, target))` === target 的包名。
 *
 * 机器无关性：写侧目标经**本 ns 仓的 `Framework` 链接**表达（bootstrap LINKS 自愈）⇒ 不钉平台检出绝对路径。
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

/** 平台前端包索引：包名 → 平台仓相对目录（`Framework/frontend/<dir>`）。 */
export function platformPackageIndex(platformRoot: string): Map<string, string> {
  const idx = new Map<string, string>();
  const dir = join(platformRoot, 'Framework', 'frontend');
  if (!existsSync(dir)) return idx;
  for (const d of readdirSync(dir)) {
    const pj = join(dir, d, 'package.json');
    if (!existsSync(pj)) continue;
    try {
      const parsed: unknown = JSON.parse(readFileSync(pj, 'utf8'));
      if (parsed !== null && typeof parsed === 'object' && 'name' in parsed && typeof parsed.name === 'string') {
        idx.set(parsed.name, `Framework/frontend/${d}`);
      }
    } catch {
      continue;
    }
  }
  return idx;
}

/** 写规则（唯一）：声明目录 → 目标目录 的 `link:` 规格（相对声明目录，pnpm 语义）。 */
export function linkSpecFor(fromDir: string, targetDir: string): string {
  return `link:${relative(fromDir, targetDir)}`;
}

/** 验规则（唯一）：`link:` 目标解析 → 目标目录 `package.json` 的 `name`（解析失败/缺 package.json ⇒ null）。
 *  `link:` 基准：声明所在目录（pnpm 语义）——额外候选根（平台根 / 仓根）用于容忍经链接写出的相对形态。 */
export function resolveLinkName(pkgDir: string, spec: string, extraRoots: readonly string[] = []): string | null {
  if (!spec.startsWith('link:')) return null;
  const rel = spec.slice('link:'.length);
  for (const c of [resolve(pkgDir, rel), ...extraRoots.map((r) => resolve(r, rel))]) {
    try {
      const pj = join(c, 'package.json');
      if (!existsSync(pj)) continue;
      const parsed: unknown = JSON.parse(readFileSync(pj, 'utf8'));
      if (parsed === null || typeof parsed !== 'object' || !('name' in parsed)) continue;
      const name = parsed.name;
      if (typeof name === 'string' && name.length > 0) return name;
    } catch {
      continue;
    }
  }
  return null;
}
