/**
 * sql-split.ts — SQL 语句切分（词法状态机，非正则解析）
 *
 * 自 `check-isahl-ddl-boundary.ts` 抽取共享（consolidate-model-seed-table-contract）。
 * 跟踪单引号/双引号/行注释/块注释/美元引用（$tag$...$tag$）/COPY FROM stdin 数据区，
 * 在顶层 ';' 处切分。任何切分歧义由下游真解析器（libpg_query）兜底（fail closed）。
 */

/** 切分产物：单条语句文本 + 其在源文件中的字符偏移（父进程据此换算行号）。 */
export interface SqlStatement {
  text: string;
  offset: number;
}

export function splitStatements(sql: string): SqlStatement[] {
  return [...iterStatements(sql)];
}

/**
 * 惰性切分：逐条 yield（消费完即可回收），用于**只读扫描**面。
 * 大载荷（88 MB dump ≈ 3.5e4 条语句）下语句数组与逐文件高水位是 GB 级峰值的主因；
 * 扫描方改用它后峰值只与**单个文件**大小相关。语义与 `splitStatements` 完全一致
 * （同一实现，后者 = 前者 + 收集）。
 */
export function* iterStatements(sql: string): Generator<SqlStatement> {
  let start = 0;
  let i = 0;
  let state: 'normal' | 'sq' | 'dq' | 'line' | 'block' | 'dollar' | 'copydata' = 'normal';
  let dollarTag = '';
  const n = sql.length;
  // psql 元命令清洗：原实现把全文拆成**字符数组**（`sql.split('')`）后就地抹空格——
  // 对 88 MB dump 即 8.8e7 个元素（实测单文件峰值 2.1 GiB 的主因）。改为记录被清洗的
  // 行区间，切片时按位展开成等长空格：输出逐字节一致（长度/偏移/行号对齐不变），
  // 内存从 O(字符数) 降到 O(元命令行数)。
  const blanks: { from: number; to: number }[] = [];
  const cleanedSlice = (from: number, to: number): string => {
    let out = '';
    let cursor = from;
    for (const b of blanks) {
      if (b.to <= cursor) continue;
      if (b.from >= to) break;
      const s = b.from > cursor ? b.from : cursor;
      const e = b.to < to ? b.to : to;
      out += sql.slice(cursor, s);
      out += ' '.repeat(e - s);
      cursor = e;
    }
    return out + sql.slice(cursor, to);
  };
  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];
    switch (state) {
      case 'normal':
        // psql 元命令（\echo/\set/\i 等）：行首（前导空白后）为反斜杠 → 整行清洗为
        // 等长空格（保持 offset/行列对齐），不进入 chunk；否则无分号的元命令会与
        // 后续语句合并成非法 chunk 导致 fail closed。COPY FROM stdin 数据区
        // （copydata 状态）由 \. 结束标记单独处理，不受影响。
        if (c === '\\') {
          const lineBegin = sql.lastIndexOf('\n', i - 1) + 1;
          if (/^\s*$/.test(sql.slice(lineBegin, i))) {
            let j = i;
            while (j < n && sql[j] !== '\n') j++;
            blanks.push({ from: i, to: j });
            state = 'line';
          }
        }
        else if (c === "'") state = 'sq';
        else if (c === '"') state = 'dq';
        else if (c === '-' && next === '-') { state = 'line'; i++; }
        else if (c === '/' && next === '*') { state = 'block'; i++; }
        else if (c === '$') {
          let j = i + 1;
          while (j < n && /[A-Za-z0-9_]/.test(sql[j])) j++;
          if (j < n && sql[j] === '$') { dollarTag = sql.slice(i, j + 1); state = 'dollar'; i = j; }
        } else if (c === ';') {
          const stmtText = cleanedSlice(start, i + 1);
          const trimmed = stmtText.trim();
          yield { text: stmtText, offset: start };
          start = i + 1;
          // COPY ... FROM stdin; 后跟原始数据区直到 \. 行（数据区不属于任何语句）
          if (/^COPY\s+/i.test(trimmed) && /\bFROM\s+stdin\b/i.test(trimmed)) state = 'copydata';
        }
        break;
      case 'sq':
        if (c === "'") { if (next === "'") i++; else state = 'normal'; }
        break;
      case 'dq':
        if (c === '"') { if (next === '"') i++; else state = 'normal'; }
        break;
      case 'line':
        if (c === '\n') state = 'normal';
        break;
      case 'block':
        if (c === '*' && next === '/') { state = 'normal'; i++; }
        break;
      case 'dollar':
        if (sql.startsWith(dollarTag, i)) { state = 'normal'; i += dollarTag.length - 1; }
        break;
      case 'copydata':
        if (c === '\\' && next === '.') {
          state = 'normal';
          start = i + 2; // 数据区不属于任何语句：推进起点越过 \.
          i++;
        }
        break;
    }
    i++;
  }
  if (start < n && cleanedSlice(start, n).trim()) yield { text: cleanedSlice(start, n), offset: start };
}
