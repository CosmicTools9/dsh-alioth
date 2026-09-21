/**
 * json-bigint-safe — 保护 >2^53 整数（Alioth id）的 JSON 读写往返。
 *
 * 背景（2026-09-13 实证）：Alioth 维度行 id 为 17 位（如 `522417556774978450`），
 * 超出 JS `Number.MAX_SAFE_INTEGER`（2^53-1）。`JSON.parse` 会把它舍入（`…400`），
 * `scripts/prototype-tool.js` 的 parse→mutate→stringify 往返因此会在每次 build 回写
 * `block.json` / `module.json` / `app.json` 时**静默污染 id**（`ID_JSON_PRECISION` 类缺陷；
 * production-order 块 r1 评审的 P1 即此）。
 *
 * 机制（不引入依赖，也不用正则模拟解析器）：
 *   ① `parseBigIntSafe`：字符级 tokenizer 扫描原文，把「整数 token 且 |值| > 2^53」的数字文本
 *      换成**全局唯一哨兵字符串字面量**（JSON 合法），其余语法交给 `JSON.parse`；
 *      哨兵 → 原始数字文本登记到模块级表。
 *   ② `stringifyBigIntSafe`：`JSON.stringify` 后**总是**把文本中出现的哨兵字面量扫回原始数字
 *      （字面 `split/join`，非正则解析）。哨兵全局唯一 ⇒ 与「读路径 / 写路径」是否同名无关
 *      （写盘常经 `xxx.tmp` 再 rename），跨多次读写的未引用哨兵是无害 no-op。
 *
 * 局限：只保护**整数**（id 语义）；浮点大数不在此契约内。
 */

import { readFileSync, writeFileSync } from 'node:fs';

const SENTINEL_PREFIX = '\u0000ALIOTH_BIGINT:';
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE = -MAX_SAFE;

/** 哨兵字面量 → 原始数字文本（跨全部 parse 累计；哨兵全局唯一） */
const SENTINELS = new Map<string, string>();
let sentinelSeq = 0;

type Token = { start: number; end: number; digits: string };

/** 提取「整数且超出安全范围」的数字 token（字符级扫描；字符串/转义内的数字不参与） */
function scanBigIntTokens(text: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  const n = text.length;
  let inString = false;
  while (i < n) {
    const ch = text[i];
    if (inString) {
      if (ch === '\\') {
        i += 2;
        continue;
      }
      if (ch === '"') inString = false;
      i += 1;
      continue;
    }
    if (ch === '"') {
      inString = true;
      i += 1;
      continue;
    }
    const isNumStart = (ch >= '0' && ch <= '9') || (ch === '-' && text[i + 1] >= '0' && text[i + 1] <= '9');
    if (!isNumStart) {
      i += 1;
      continue;
    }
    let j = i;
    if (text[j] === '-') j += 1;
    while (j < n && text[j] >= '0' && text[j] <= '9') j += 1;
    const isInteger = !(text[j] === '.' || text[j] === 'e' || text[j] === 'E' || text[j] === 'n');
    if (isInteger) {
      const digits = text.slice(i, j);
      let big = false;
      try {
        const v = BigInt(digits);
        big = v > MAX_SAFE || v < MIN_SAFE;
      } catch {
        big = false;
      }
      if (big) tokens.push({ start: i, end: j, digits });
    }
    i = j;
  }
  return tokens;
}

/** 解析可含大整数的 JSON 文本（大整数以哨兵字符串表示；写回时由 `stringifyBigIntSafe` 还原） */
export function parseBigIntSafe(text: string): unknown {
  const tokens = scanBigIntTokens(text);
  if (tokens.length === 0) return JSON.parse(text);
  let rewritten = '';
  let cursor = 0;
  for (const t of tokens) {
    const sentinel = `${SENTINEL_PREFIX}${sentinelSeq++}${SENTINEL_PREFIX}`;
    SENTINELS.set(JSON.stringify(sentinel), t.digits);
    rewritten += text.slice(cursor, t.start) + JSON.stringify(sentinel);
    cursor = t.end;
  }
  rewritten += text.slice(cursor);
  return JSON.parse(rewritten);
}

/** 把序列化文本中的哨兵字面量扫回原始数字文本（字面替换，非正则） */
export function restoreBigInts(serialized: string): string {
  let out = serialized;
  for (const [needle, digits] of SENTINELS) {
    if (out.includes(needle)) out = out.split(needle).join(digits);
  }
  return out;
}

/**
 * 取「解析后仍是哨兵串」的原始数字文本；非大整数返回 `null`。
 * 用于校验场景（如 block.json 的坐标 id 与 DB 比对）——避免再次经 Number 舍入。
 */
export function rawIntegerText(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  for (const [needle, digits] of SENTINELS) {
    if (needle === JSON.stringify(value)) return digits;
  }
  return null;
}

/** 序列化（默认 2 空格缩进 + 尾换行）并还原大整数 */
export function stringifyBigIntSafe(value: unknown, indent = 2): string {
  return restoreBigInts(`${JSON.stringify(value, null, indent)}\n`);
}

/** 读文件（大整数保护） */
export function readJsonBigIntSafe(path: string): unknown {
  return parseBigIntSafe(readFileSync(path, 'utf-8'));
}

/** 写文件（大整数保护；保持既有 `JSON.stringify(v, null, 2) + '\n'` 形态） */
export function writeJsonBigIntSafe(path: string, value: unknown): void {
  writeFileSync(path, stringifyBigIntSafe(value), 'utf-8');
}
