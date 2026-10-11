/**
 * icon-pool.ts — 原型图标池单一读取/判定入口
 *
 * 池 = `.agents/skills/alioth-design/references/icon-pool.js` 的 `T_ICONS`（`key: '<path d>'`，键为
 * **camelCase**；`prototype-tool build` 注入产物，三套 shell 经 `ICON_POOL_CONTENT` 引用）。
 *
 * 本模块是声明面图标规则的**唯一实现**（`gen-app-shell-tabs.ts` / `gen-module-block-assembly.ts` 共用）：
 *   · 池键集读取（`readIconPool`）；
 *   · 声明原值归一（`normalizeIcon`：首字母小写 + kebab→camel）；
 *   · 判定（`iconProblem`：非 camelCase（原值 ≠ 归一值）或归一后不在池 ⇒ 文案；合规 ⇒ null）。
 *
 * 规约：`openspec/specs/block-schema/spec.md`（`block-assembly-gap-filling`；回退 `clipboard`）
 * + 归档 change `2026-10-08-align-module-icon-naming`（声明面图标 MUST camelCase、MUST 命中原型图标池）。
 */

import fs from "node:fs";
import path from "node:path";

/** 池文件相对仓根路径。 */
export const ICON_POOL_REL =
  ".agents/skills/alioth-design/references/icon-pool.js";

/** 缺省回退图标（池内键）：声明缺 `icon` 或图标缺位时的渲染回退。 */
export const FALLBACK_ICON = "layout";

/** 读池键集（只取 `T_ICONS` 对象体内的 `key:` —— 与缩进无关，避免误收 `SvgIcon` 内属性等非图标键）。 */
export function readIconPool(root: string): Set<string> {
  const keys = new Set<string>();
  const file = path.join(root, ICON_POOL_REL);
  if (!fs.existsSync(file)) return keys;
  const src = fs.readFileSync(file, "utf8");
  const start = src.indexOf("T_ICONS");
  const end = start >= 0 ? src.indexOf("};", start) : -1;
  // 锚点缺失（池形态被改坏）⇒ 返回空集：调用方会把所有图标判「不在池」= fail-loud，
  // 绝不退回「扫全文」（那正是把 `SvgIcon` 属性等非图标键收进来的口子）。
  if (start < 0 || end <= start) return keys;
  const body = src.slice(start, end);
  for (const m of body.matchAll(/^[ \t]*([A-Za-z][A-Za-z0-9]*)[ \t]*:/gm)) {
    if (m[1]) keys.add(m[1]);
  }
  return keys;
}

/** 声明原值 → 池键口径（首字母小写；kebab→camel）。 */
export function normalizeIcon(raw: string): string {
  if (raw.includes("-")) {
    const parts = raw.split("-");
    return (
      parts[0]! +
      parts
        .slice(1)
        .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
        .join("")
    );
  }
  return raw.charAt(0).toLowerCase() + raw.slice(1);
}

/**
 * 图标判定（合规 ⇒ null）。`where` 用于文案定位（如 ``模块 `x`的 icon`` / `navigation.groups[0].icon`）。
 */
export function iconProblem(
  raw: string | null,
  pool: Set<string>,
  where: string,
): string | null {
  if (raw === null) return null;
  const norm = normalizeIcon(raw);
  if (norm !== raw) {
    return `${where} \`${raw}\` 非 camelCase（声明面图标 MUST camelCase，对齐原型图标池键；应写 \`${norm}\`）`;
  }
  if (!pool.has(norm)) {
    return `${where} \`${norm}\` 不在原型图标池（渲染期回退「•」）`;
  }
  return null;
}
