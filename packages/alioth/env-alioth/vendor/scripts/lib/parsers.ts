/**
 * parsers.ts — 集中导出项目已有 parser，供审计脚本引用。
 *
 * 用法（Bun）:
 *   import { html, css, js, cssSelector } from '../lib/parsers.ts';
 *
 * 所有导出模块已在项目 node_modules 中。
 */
import { load } from 'cheerio';
import * as csstree from 'css-tree';
import { parseModule, parseScript } from 'meriyah';
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
