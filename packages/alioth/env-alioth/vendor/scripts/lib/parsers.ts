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

/**
 * PostgreSQL 默认值表达式解析 — 本仓最小实现（**手写词法 + 形态校验**，非正则）。
 *
 * 用途：从列默认值取 `gen_next_uid(<code>)` 的表码（`isahl` 面 id 生成链的唯一入口形态）。
 * 依据：`ALIOTH_ONTOLOGY_SPEC`（id 口径：高 16 位表码 + 低 48 位 `isahl.uid_seq`）与
 * `NO_REGEX_FOR_PARSING.md`（结构化数据 MUST NOT 用正则模拟解析）——故此处实现真正的词法/结构解析，
 * 且**形态不符即返回 null**（调用方据此显式报错，绝不猜）。
 *
 * 接受形态：`gen_next_uid(<int>)` / `gen_next_uid(<int>::bigint)` / `gen_next_uid((<int>)::bigint)`
 * 拒绝形态：拼算式（`gen_next_uid(1) | 5`）、多参、其他函数名、非字面量参数、括号不配 → `null`
 */

export const pgDefault = {
  /** 取 `gen_next_uid(<int>)` 的字面量参数；形态不符返回 null。 */
  genNextUidCode(expr: string): number | null {
    const toks: { k: 'ident' | 'int' | 'punct' | 'other'; v: string }[] = [];
    for (let i = 0; i < expr.length; ) {
      const c = expr[i]!;
      if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
        i++;
        continue;
      }
      if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c === '_') {
        let j = i + 1;
        while (j < expr.length) {
          const d = expr[j]!;
          if (!((d >= 'a' && d <= 'z') || (d >= 'A' && d <= 'Z') || (d >= '0' && d <= '9') || d === '_')) break;
          j++;
        }
        toks.push({ k: 'ident', v: expr.slice(i, j) });
        i = j;
        continue;
      }
      if (c >= '0' && c <= '9') {
        let j = i + 1;
        while (j < expr.length && expr[j]! >= '0' && expr[j]! <= '9') j++;
        toks.push({ k: 'int', v: expr.slice(i, j) });
        i = j;
        continue;
      }
      if (c === '(' || c === ')' || c === ':' || c === ',') {
        toks.push({ k: 'punct', v: c });
        i++;
        continue;
      }
      toks.push({ k: 'other', v: c });
      i++;
    }

    // 语法：ident "(" [ "(" ] int [ ")" ] [ "::" ident ] ")" EOF
    let p = 0;
    const peek = () => toks[p];
    const eat = (k: string, v?: string): boolean => {
      const t = peek();
      if (!t || t.k !== k || (v !== undefined && t.v !== v)) return false;
      p++;
      return true;
    };
    if (!eat('ident', 'gen_next_uid')) return null;
    if (!eat('punct', '(')) return null;
    // 参数：`(<int>)` 或 `<int>`（二选一，不可两段可选——否则会把外层收括号吃掉）
    let num: { k: string; v: string } | undefined;
    if (peek()?.k === 'punct' && peek()!.v === '(') {
      p++;
      num = peek();
      if (!num || num.k !== 'int') return null;
      p++;
      if (!eat('punct', ')')) return null;
    } else {
      num = peek();
      if (!num || num.k !== 'int') return null;
      p++;
    }
    if (peek()?.k === 'punct' && peek()!.v === ':') {
      if (!eat('punct', ':') || !eat('punct', ':') || !eat('ident')) return null;
    }
    if (!eat('punct', ')')) return null;
    if (p !== toks.length) return null; // 尾随记号（如 `| 5`）= 非本形态
    const n = Number(num.v);
    return Number.isSafeInteger(n) && n >= 0 ? n : null;
  },
};

