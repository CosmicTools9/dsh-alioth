/**
 * dump-statements.ts — pg_dump 数据语句的结构定位原语（共享实现）
 *
 * 调用方（扫描器单一源，禁止第二套）：
 *   · `scripts/seed/strip-seed-owner-columns.ts`（行级授权三列载荷判定/清空）
 *   · `scripts/lib/dump-redact.ts`（元数据面快照脱敏：按列名置空 / 删 JSON 键）
 *   · `scripts/check/check-no-secrets-in-backup.ts`（零机密门禁的 INSERT 形态判据）
 *
 * 解析约定（`NO_REGEX_FOR_PARSING.md` §判定方法 判定记录）：字符状态机（引号 / E 串 / 美元引用 /
 * 注释 / 括号深度），只做语句内「表名 · 列清单 · 取值项 span · SET 赋值 span」定位——不用正则模拟解析器。
 * 覆盖 `pg_dump --inserts`（位置列）与 `--column-inserts`（具名列）两种 INSERT 形态，以及 `UPDATE … SET`。
 */

/** 语句内的偏移区间（相对语句文本） */
export interface Span {
  start: number;
  end: number;
}

export type InsertShape = {
  kind: 'insert';
  /** 表名**末段**（比较用；`RULES`/载荷清单按末段或限定名各自取用） */
  table: string;
  /** **限定名**（`schema.表`，各段去引号；`positions` 查表键，避免同名异 schema 错配） */
  qual: string;
  /** 具名列清单（null = 位置形态，需外部列位表） */
  cols: string[] | null;
  /** 每个元组的取值项 span */
  tuples: Span[][];
  /** `INSERT … SELECT …`：取值来自查询 ⇒ 无取值项可定位 */
  hasSelect: boolean;
};
export type UpdateShape = {
  kind: 'update';
  table: string;
  qual: string;
  assigns: { column: string; start: number; end: number }[];
};
export type StatementShape = InsertShape | UpdateShape;

function isIdentChar(c: string): boolean {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c === '_' || c === '$' || c === '-';
}

/** 跳过空白与注释（行注释 `--` 与块注释；返回首个非 trivia 字符下标） */
export function skipTrivia(s: string, i: number): number {
  const n = s.length;
  for (;;) {
    while (i < n && (s[i] === ' ' || s[i] === '\t' || s[i] === '\n' || s[i] === '\r')) i++;
    if (i + 1 < n && s[i] === '-' && s[i + 1] === '-') {
      const nl = s.indexOf('\n', i);
      i = nl < 0 ? n : nl + 1;
      continue;
    }
    if (i + 1 < n && s[i] === '/' && s[i + 1] === '*') {
      const end = s.indexOf('*/', i + 2);
      i = end < 0 ? n : end + 2;
      continue;
    }
    return i;
  }
}

/** 跳过字符串字面量 / 引号标识符 / 美元引用；非引用起首 → 原样返回 i */
export function skipQuoted(s: string, i: number): number {
  const n = s.length;
  const c = s[i]!;
  const escapeString = (c === 'E' || c === 'e') && s[i + 1] === "'";
  if (c === "'" || escapeString) {
    let j = i + (escapeString ? 2 : 1);
    while (j < n) {
      if (escapeString && s[j] === '\\') {
        j += 2;
        continue;
      }
      if (s[j] === "'") {
        if (s[j + 1] === "'") {
          j += 2;
          continue;
        }
        return j + 1;
      }
      j++;
    }
    return n;
  }
  if (c === '"') {
    let j = i + 1;
    while (j < n) {
      if (s[j] === '"') {
        if (s[j + 1] === '"') {
          j += 2;
          continue;
        }
        return j + 1;
      }
      j++;
    }
    return n;
  }
  if (c === '$') {
    let j = i + 1;
    while (j < n && isIdentChar(s[j]!)) j++;
    if (j < n && s[j] === '$') {
      const tag = s.slice(i, j + 1);
      const end = s.indexOf(tag, j + 1);
      return end < 0 ? n : end + tag.length;
    }
  }
  return i;
}

export function readIdent(s: string, i: number): { value: string; next: number } {
  const n = s.length;
  if (s[i] === '"') {
    let j = i + 1;
    let out = '';
    while (j < n) {
      if (s[j] === '"') {
        if (s[j + 1] === '"') {
          out += '"';
          j += 2;
          continue;
        }
        return { value: out, next: j + 1 };
      }
      out += s[j]!;
      j++;
    }
    return { value: out, next: n };
  }
  let j = i;
  while (j < n && isIdentChar(s[j]!)) j++;
  return { value: s.slice(i, j), next: j };
}

/** 读 `schema.表` 限定名（末段 + 限定名；各段去引号） */
export function readQualified(s: string, i: number): { table: string; qual: string; next: number } {
  let cur = readIdent(s, i);
  const segs = [cur.value];
  let j = cur.next;
  for (;;) {
    const k = skipTrivia(s, j);
    if (s[k] !== '.') break;
    cur = readIdent(s, k + 1);
    segs.push(cur.value);
    j = cur.next;
  }
  return { table: segs[segs.length - 1]!, qual: segs.join('.'), next: j };
}

function readWord(s: string, i: number): { word: string; next: number } {
  let j = i;
  const n = s.length;
  while (j < n) {
    const c = s[j]!;
    if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c === '_') j++;
    else break;
  }
  return { word: s.slice(i, j), next: j };
}

/** `(` … `)` 内顶层逗号切分的项 span（相对语句文本） */
export function readParenItems(s: string, open: number): { items: Span[]; next: number } {
  const n = s.length;
  const items: Span[] = [];
  let depth = 0;
  let itemStart = -1;
  for (let j = open; j < n; j++) {
    const c = s[j]!;
    const quotedEnd =
      c === "'" || c === '"' || c === '$' || ((c === 'E' || c === 'e') && s[j + 1] === "'") ? skipQuoted(s, j) : j;
    if (quotedEnd > j) {
      j = quotedEnd - 1;
      continue;
    }
    if (c === '-' && s[j + 1] === '-') {
      const nl = s.indexOf('\n', j);
      j = nl < 0 ? n : nl;
      continue;
    }
    if (c === '(') {
      depth++;
      if (depth === 1) itemStart = j + 1;
      continue;
    }
    if (c === ')') {
      depth--;
      if (depth === 0) {
        if (itemStart >= 0) {
          const raw = s.slice(itemStart, j);
          items.push({
            start: itemStart + (raw.length - raw.trimStart().length),
            end: itemStart + raw.trimEnd().length,
          });
        }
        return { items, next: j + 1 };
      }
      continue;
    }
    if (c === ',' && depth === 1 && itemStart >= 0) {
      const raw = s.slice(itemStart, j);
      items.push({
        start: itemStart + (raw.length - raw.trimStart().length),
        end: itemStart + raw.trimEnd().length,
      });
      itemStart = j + 1;
    }
  }
  return { items, next: n };
}

/** `INSERT`/`UPDATE` 语句结构定位；其他语句 → null */
export function scanStatement(text: string): StatementShape | null {
  const head = readWord(text, skipTrivia(text, 0));
  const verb = head.word.toUpperCase();
  if (verb === 'INSERT') {
    const into = readWord(text, skipTrivia(text, head.next));
    if (into.word.toUpperCase() !== 'INTO') return null;
    const qual = readQualified(text, skipTrivia(text, into.next));
    const table = stripIdentQuotes(qual.table);
    let j = skipTrivia(text, qual.next);
    let cols: string[] | null = null;
    if (text[j] === '(') {
      const par = readParenItems(text, j);
      cols = par.items.map((it) => stripIdentQuotes(text.slice(it.start, it.end)));
      j = skipTrivia(text, par.next);
    }
    const kw = readWord(text, j);
    const upper = kw.word.toUpperCase();
    if (upper !== 'VALUES') {
      return { kind: 'insert', table, qual: qual.qual, cols, tuples: [], hasSelect: upper === 'SELECT' };
    }
    const tuples: Span[][] = [];
    let k = skipTrivia(text, kw.next);
    while (k < text.length && text[k] === '(') {
      const par = readParenItems(text, k);
      tuples.push(par.items);
      k = skipTrivia(text, par.next);
      if (text[k] !== ',') break;
      k = skipTrivia(text, k + 1);
    }
    return { kind: 'insert', table, qual: qual.qual, cols, tuples, hasSelect: false };
  }
  if (verb !== 'UPDATE') return null;
  let j = skipTrivia(text, head.next);
  const firstWord = readWord(text, j);
  if (firstWord.word.toUpperCase() === 'ONLY') j = skipTrivia(text, firstWord.next);
  const qual = readQualified(text, j);
  const setKw = readWord(text, skipTrivia(text, qual.next));
  if (setKw.word.toUpperCase() !== 'SET') return null;
  const assigns: { column: string; start: number; end: number }[] = [];
  let k = skipTrivia(text, setKw.next);
  for (;;) {
    const col = readIdent(text, k);
    const after = skipTrivia(text, col.next);
    if (text[after] !== '=') break;
    const start = skipTrivia(text, after + 1);
    let end = start;
    let depth = 0;
    while (end < text.length) {
      const c = text[end]!;
      const quotedEnd =
        c === "'" || c === '"' || c === '$' || ((c === 'E' || c === 'e') && text[end + 1] === "'")
          ? skipQuoted(text, end)
          : end;
      if (quotedEnd > end) {
        end = quotedEnd;
        continue;
      }
      if (c === '(') depth++;
      else if (c === ')') depth--;
      else if (depth === 0) {
        if (c === ',') break;
        const w = readWord(text, end);
        const up = w.word.toUpperCase();
        if (w.word && (up === 'WHERE' || up === 'FROM' || up === 'RETURNING')) break;
      }
      end++;
    }
    const raw = text.slice(start, end);
    assigns.push({ column: stripIdentQuotes(col.value), start, end: start + raw.trimEnd().length });
    if (text[end] !== ',') break;
    k = skipTrivia(text, end + 1);
  }
  return { kind: 'update', table: stripIdentQuotes(qual.table), qual: qual.qual, assigns };
}

/** 反引号/双引号包裹的标识符 → 裸名（比较用） */
export function stripIdentQuotes(ident: string): string {
  const t = ident.trim();
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith('`') && t.endsWith('`'))) return t.slice(1, -1);
  return t;
}

/** 按 span 应用替换（倒序拼接，保持其余字节不变） */
export function applySpanEdits(text: string, edits: readonly { start: number; end: number; text: string }[]): string {
  let out = text;
  for (const e of [...edits].sort((a, b) => b.start - a.start)) {
    out = out.slice(0, e.start) + e.text + out.slice(e.end);
  }
  return out;
}

/** `NULL`（可带 `::cast` 尾缀）判定 */
export function isNullLiteral(text: string): boolean {
  const t = text.trim();
  if (t.slice(0, 4).toUpperCase() !== 'NULL') return false;
  const rest = t.slice(4).trim();
  return rest === '' || rest.startsWith('::');
}

/** 行首偏移表（`starts[i]` = 第 i 行的起始字符偏移） */
export function lineStartOffsets(text: string): number[] {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === '\n') starts.push(i + 1);
  return starts;
}

/** 文件内偏移 → 1-based 行号（二分于行首偏移表） */
export function lineAt(starts: readonly number[], offset: number): number {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if ((starts[mid] ?? 0) <= offset) lo = mid;
    else hi = mid - 1;
  }
  return lo + 1;
}

/** 表达式是否携带数字数组字面量（`'{1,2}'` / `ARRAY[1,2]`） */
export function hasNumericArrayLiteral(expr: string): boolean {
  let i = 0;
  while (i < expr.length) {
    const quotedEnd =
      expr[i] === "'" || expr[i] === '$' || ((expr[i] === 'E' || expr[i] === 'e') && expr[i + 1] === "'")
        ? skipQuoted(expr, i)
        : i;
    if (quotedEnd > i) {
      const inner = expr.slice(i + 1, Math.max(i + 1, quotedEnd - 1));
      if (/^\{[\s0-9,]*\}$/.test(inner) && /[0-9]/.test(inner)) return true;
      i = quotedEnd;
      continue;
    }
    if ((expr[i] === 'A' || expr[i] === 'a') && expr.slice(i, i + 6).toUpperCase() === 'ARRAY[') {
      const close = expr.indexOf(']', i + 6);
      const inner = close < 0 ? expr.slice(i + 6) : expr.slice(i + 6, close);
      if (/[0-9]/.test(inner)) return true;
      i = close < 0 ? expr.length : close + 1;
      continue;
    }
    i++;
  }
  return false;
}
