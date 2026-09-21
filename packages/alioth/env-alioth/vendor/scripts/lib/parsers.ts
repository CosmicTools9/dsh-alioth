/**
 * parsers.ts — 集中导出项目已有 parser，供审计脚本引用。
 *
 * 用法（Bun）:
 *   import { html, css, js, cssSelector, markdown, toml } from '../lib/parsers.ts';
 *
 * 所有导出模块已在项目 node_modules 中（`toml` 例外：用 Bun 内建 TOML 解析器）。
 */
import { load } from 'cheerio';
import * as csstree from 'css-tree';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfmTableFromMarkdown } from 'mdast-util-gfm-table';
import { parseModule, parseScript } from 'meriyah';
import { gfmTable } from 'micromark-extension-gfm-table';
import parser from 'postcss-selector-parser';

/** HTML DOM 操作 — cheerio */
export const html = { load };

/** CSS AST 解析 — css-tree */
export const css = csstree;

/** JS 源码 AST 解析 — meriyah（parse=script, parseModule=ESM） */
export const js = { parse: parseScript, parseModule };

// 注：typescript@7（原生编译器）主入口不再提供 TS5 编译器 API（createSourceFile/
// forEachChild/isXxx 谓词）。需要解析 TS/JS 时用 Bun.Transpiler 去类型 + meriyah，
// 或从 'typescript/unstable/*' 新 API 接入；此处不再导出不可用的 ts。

/** CSS 选择器解析 — postcss-selector-parser */
export const cssSelector = parser;

/**
 * Markdown 文档 AST 解析 — mdast（CommonMark + GFM 表格扩展）。
 *
 * 表格属 GFM 扩展（CommonMark 不含），而「按表格抽取」是本仓库规约文档的常规判据
 * （如 REPO_LAYOUT_SPEC 条目表）——扩展集中内置于此，调用方不得各自拼装 parser，
 * 更 MUST NOT 按 `|` 拆行或用正则解析结构化文档（NO_REGEX_FOR_PARSING.md:120）。
 */
export const markdown = {
  fromMarkdown: (doc: string) =>
    fromMarkdown(doc, { extensions: [gfmTable()], mdastExtensions: [gfmTableFromMarkdown()] }),
};

/**
 * TOML 解析 — Bun 内建解析器（`Bun.TOML`）。
 *
 * 面向 Cargo.toml / Cargo.lock 等 TOML 结构化容器；调用方 MUST NOT 用正则或按行拆文本
 * 解析（NO_REGEX_FOR_PARSING.md）——例如从 `Cargo.lock` 取依赖版本、从根 `Cargo.toml`
 * 取 `[workspace.dependencies]` / `[patch.crates-io]` 条目。
 */
export const toml = {
  parse: (doc: string): Record<string, unknown> => Bun.TOML.parse(doc) as Record<string, unknown>,
};
