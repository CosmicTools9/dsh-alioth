/**
 * rust-literals.ts — Rust 字符串字面量词法提取（共享 helper）
 *
 * 用途: 审计脚本需要从 `.rs` 中取出**字符串字面量**（应用侧 SQL 常内联在 `sqlx::query(r#"…"#)`
 *   或 `const _SQL_*`），再交给 SQL 解析器判定。禁止用正则模拟该提取
 *   （NO_REGEX_FOR_PARSING：`r#"…"#` 的变长井号/转义/注释都需要词法状态机）。
 *
 * 语义: 识别 ① 行注释；② 块注释（含嵌套）；③ 原始串 `r"…"` / `r#"…"#` / `r##"…"##`
 *   （可带 `b` 前缀）；④ 普通串（可带 `b` 前缀，处理反斜杠转义）；其余字符跳过。
 *
 * 两个 API（同一词法实现）:
 *   - `rustStringLiterals(src)` → 字面量**内容**列表（按出现顺序；既有消费者用）
 *   - `extractRustLiterals(src)` → 带 `line` / `start` / `end` 的定位信息（新增消费者用，
 *     如 `check-dynamic-table-name.ts` / `check-knowledge-domains.ts` 的相邻 token 判定）
 */

/** 字符串字面量（含定位） */
export interface RustLiteral {
  /** 字面量内容（不含引号；raw string 已剥离 `#` 定界） */
  text: string;
  /** 起始行号（1-based） */
  line: number;
  /** 字面量在源码中的起止字节偏移（`end` 为开区间） */
  start: number;
  end: number;
}

/** 提取源码中的全部字符串字面量（含定位，按出现顺序）。 */
export function extractRustLiterals(src: string): RustLiteral[] {
  const out: RustLiteral[] = [];
  let i = 0;
  let line = 1;
  const n = src.length;
  /** 行号推进（切分区间内的换行计数） */
  const bump = (from: number, to: number) => {
    for (let k = from; k < to && k < n; k++) if (src[k] === '\n') line++;
  };
  while (i < n) {
    const c = src[i];
    const next = src[i + 1];
    if (c === '\n') {
      line++;
      i++;
      continue;
    }
    if (c === '/' && next === '/') {
      while (i < n && src[i] !== '\n') i++;
      continue;
    }
    if (c === '/' && next === '*') {
      i += 2;
      let depth = 1;
      while (i < n && depth > 0) {
        if (src[i] === '/' && src[i + 1] === '*') {
          depth++;
          i += 2;
        } else if (src[i] === '*' && src[i + 1] === '/') {
          depth--;
          i += 2;
        } else {
          if (src[i] === '\n') line++;
          i++;
        }
      }
      continue;
    }
    const rawStart = /^(?:b)?r(#*)"/.exec(src.slice(i, i + 64));
    if (rawStart) {
      const start = i;
      const startLine = line;
      const hashes = rawStart[1] ?? '';
      const bodyStart = i + rawStart[0].length;
      const terminator = `"${hashes}`;
      const end = src.indexOf(terminator, bodyStart);
      if (end === -1) break;
      const text = src.slice(bodyStart, end);
      const litEnd = end + terminator.length;
      bump(start, litEnd);
      out.push({ text, line: startLine, start, end: litEnd });
      i = litEnd;
      continue;
    }
    const normalStart = /^(?:b)?"/.exec(src.slice(i, i + 2));
    if (normalStart) {
      const start = i;
      const startLine = line;
      let j = i + normalStart[0].length;
      let buf = '';
      while (j < n) {
        if (src[j] === '\\') {
          if (src[j + 1] === '\n') line++;
          buf += src[j + 1] ?? '';
          j += 2;
          continue;
        }
        if (src[j] === '"') break;
        if (src[j] === '\n') line++;
        buf += src[j];
        j++;
      }
      const end = j + 1;
      out.push({ text: buf, line: startLine, start, end });
      i = end;
      continue;
    }
    i++;
  }
  return out;
}

/** 提取源码中的全部字符串字面量内容（按出现顺序）。 */
export function rustStringLiterals(src: string): string[] {
  return extractRustLiterals(src).map((lit) => lit.text);
}
