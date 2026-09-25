/**
 * dump-redact.ts — pg_dump 数据导出的环境凭据脱敏（共享实现）
 *
 * 调用方：`scripts/db/redact-secrets.ts`（备份管线 / 历史快照清洗 CLI）与
 * `scripts/check/check-no-secrets-in-backup.ts`（门禁）。规则只有一份——脱敏与
 * 校验必须同源，否则门禁会与清洗漂移。
 *
 * 策略（`openspec/changes/remove-secrets-from-git-backup` + `pack-credentials-into-encrypted-seed`）：
 *   1. `drop-data` 表（`isahl_meta.meta_llm_configs` / `meta_llm_provider`）：凭据列为 NOT NULL
 *      （行去 key 后无意义）→ 整段数据删除（表结构仍在 `ddl/`）；恢复由本地密文种子
 *      （`seed/ns/_<db>/local-secrets.sql.enc` + `--apply-local-secrets`）承担。
 *   2. `redact` 表（`isahl_meta.meta_data_source` / `meta_mise_env_vars` / `meta_mise_services`）：
 *      **兜底判据**——新快照已按 `scripts/db/local-secret-tables.txt` 清单 `--exclude-table-data`
 *      整表排除（凭据唯一载体 = 密文种子）；本规则覆盖历史产物与非排除路径（人工 pg_dump），
 *      置空凭据列并从 JSON 列删除凭据键。
 *
 * 解析约定（`NO_REGEX_FOR_PARSING` §判定方法 判定记录）：
 * pg_dump 纯文本导出为「段标记 + `COPY <表> (<列...>) FROM stdin;` + 制表符分隔行 + `\.`」。
 * 本实现用 `startsWith`/`indexOf`/`split('\t')` 做**结构性切分**，不用正则模拟解析器：
 * COPY 文本格式中字面制表符在值内被转义为 `\t`（同理换行 `\n`、反斜杠 `\\`），
 * 因此按字面制表符切分即是精确列切分。字段先反转义才能解析 JSON，再正向转义回写。
 */

import { applySpanEdits, isNullLiteral, scanStatement } from './dump-statements.ts';
import { splitStatements } from './sql-split.ts';

export type TableRule =
  | { table: string; mode: 'drop-data' }
  | { table: string; mode: 'redact'; columns: string[]; jsonColumns: Record<string, string[]> };

/** 脱敏清单：环境凭据类表/列。新增表/列在此登记（脱敏与门禁共用）。 */
export const RULES: TableRule[] = [
  // provider API key 密文（`api_key_enc` NOT NULL）——各环境由自身 `.env` 的 LLM_* 承载
  { table: 'isahl_meta.meta_llm_configs', mode: 'drop-data' },
  // provider API key **明文**（`api_key` NOT NULL，另一张 provider 定义表）——当前 0 行，
  // 一旦录入即随 dump 入库；与 meta_llm_configs 同款处理（行去 key 后无意义）
  { table: 'isahl_meta.meta_llm_provider', mode: 'drop-data' },
  // 数据源定义：保留行（host/port/库名等恢复所需），清掉凭据列与 JSON 内凭据键。
  // 兜底语义（2026-09-24）：本表行**不进新快照**——`backup-ddl` 按
  // `local-secret-tables.txt` 清单 `--exclude-table-data`（凭据唯一载体 = 密文种子
  // `seed/ns/_<db>/local-secrets.sql.enc`，恢复 = `--apply-local-secrets`）；本规则保留用于
  // 历史产物与非排除路径（如人工 pg_dump）+ 门禁判据（凭据 MUST 置空），二者判据同源。
  {
    table: 'isahl_meta.meta_data_source',
    mode: 'redact',
    columns: ['password_encrypted', 'connection_string'],
    jsonColumns: { config: ['password_encrypted'] },
  },
  // mise 环境变量（`is_secret` 标注行）与运行期令牌：保留行与键名，值一律清空（同上兜底语义）
  { table: 'isahl_meta.meta_mise_env_vars', mode: 'redact', columns: ['var_value'], jsonColumns: {} },
  { table: 'isahl_meta.meta_mise_services', mode: 'redact', columns: ['run_token'], jsonColumns: {} },
];

export const NULL_TOKEN = '\\N';

/**
 * 语句表名 → 规则表项。限定名优先（`isahl_meta.meta_data_source`）；未限定语句（无 `schema.`）
 * 回退末段匹配（保持手工载荷的既有行为）。脱敏与零机密门禁共用本判据，禁止第二份。
 */
export function matchRule(qual: string, table: string, mode?: TableRule['mode']): TableRule | undefined {
  return RULES.find(
    (r) =>
      (mode === undefined || r.mode === mode) &&
      (r.table === qual || (!qual.includes('.') && r.table.split('.').pop() === table)),
  );
}

/** 反引号/双引号包裹的标识符 → 裸名（比较用） */
export function bareName(ident: string): string {
  const t = ident.trim();
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith('`') && t.endsWith('`'))) {
    return t.slice(1, -1);
  }
  return t;
}

/** 解析 `COPY <表> (<列...>) FROM stdin;` 头行；非 COPY 头 → null */
export function parseCopyHeader(line: string): { table: string; cols: string[] } | null {
  if (!line.startsWith('COPY ')) return null;
  const fromIdx = line.indexOf(' FROM stdin;');
  if (fromIdx < 0) return null;
  const head = line.slice(5, fromIdx).trim();
  const paren = head.indexOf('(');
  if (paren < 0) return { table: bareName(head), cols: [] };
  const table = bareName(head.slice(0, paren));
  const inner = head.slice(paren + 1, head.lastIndexOf(')'));
  const cols = inner.split(',').map((c) => bareName(c));
  return { table, cols };
}

/** `INSERT INTO <表> ...` 语句的表名；非 INSERT → null */
export function parseInsertTable(line: string): string | null {
  if (!line.startsWith('INSERT INTO ')) return null;
  const rest = line.slice('INSERT INTO '.length);
  const space = rest.indexOf(' ');
  return bareName(space < 0 ? rest : rest.slice(0, space));
}

/** COPY 文本字段反转义（`\t`/`\n`/`\r`/`\\` 等）。返回 null = 含未支持转义，调用方须保守处理。 */
export function unescapeCopyField(raw: string): string | null {
  let out = '';
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch !== '\\') {
      out += ch;
      continue;
    }
    const next = raw[++i];
    if (next === undefined) return null;
    switch (next) {
      case 't': out += '\t'; break;
      case 'n': out += '\n'; break;
      case 'r': out += '\r'; break;
      case 'b': out += '\b'; break;
      case 'f': out += '\f'; break;
      case 'v': out += '\v'; break;
      case '\\': out += '\\'; break;
      default:
        // `\xHH` / 八进制 / 其他：不猜测语义，交由调用方保守处理
        return null;
    }
  }
  return out;
}

/** COPY 文本字段正向转义（回写用；反斜杠优先） */
export function escapeCopyField(value: string): string {
  return value
    .split('\\').join('\\\\')
    .split('\t').join('\\t')
    .split('\n').join('\\n')
    .split('\r').join('\\r');
}

export interface RedactStats {
  droppedTables: string[];
  droppedRows: number;
  redactedRows: number;
  redactedFields: number;
  warnings: string[];
}

/** 数据行切片：返回 `[行数组, 终止行下标]`（终止行 = 换行后的 `\.`） */
export function sliceCopyRows(
  lines: string[],
  headerIdx: number,
): { rows: string[]; endIdx: number } {
  const rows: string[] = [];
  let j = headerIdx + 1;
  for (; j < lines.length && lines[j] !== '\\.'; j++) rows.push(lines[j]);
  return { rows, endIdx: j };
}

/** 脱敏一份 pg_dump 纯文本导出（幂等：已脱敏内容二次运行无变化） */
export function redactDump(sql: string, stats: RedactStats): string {
  const lines = sql.split('\n');
  const hasCopy = lines.some((l) => l.startsWith('COPY '));
  const hasInsert = lines.some((l) => l.startsWith('INSERT INTO '));
  if (hasCopy && hasInsert) {
    // 混用形态无法保证两条路径的判据覆盖（形态归一由种子载荷形态门禁负责）
    throw new Error('redactDump: 同一 dump 混用 COPY 与 INSERT 形态——拒绝脱敏（须先归一形态）');
  }
  if (hasInsert) return redactInsertForm(sql, stats);
  const out: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const header = parseCopyHeader(lines[i]);

    if (!header) {
      // 等价载体（pg_dump --column-inserts）：drop-data 表整语句删除
      const insertTable = parseInsertTable(lines[i]);
      if (insertTable && RULES.some((r) => r.table === insertTable && r.mode === 'drop-data')) {
        stats.droppedRows += 1;
        continue;
      }
      out.push(lines[i]);
      continue;
    }

    const rule = RULES.find((r) => r.table === header.table);
    if (!rule) {
      out.push(lines[i]);
      continue;
    }

    const { rows, endIdx } = sliceCopyRows(lines, i);
    out.push(lines[i]); // 头行保留（结构可读）
    if (rule.mode === 'drop-data') {
      stats.droppedRows += rows.length;
      if (!stats.droppedTables.includes(header.table)) stats.droppedTables.push(header.table);
    } else {
      const plainCols = rule.columns.map((c) => header.cols.indexOf(c)).filter((n) => n >= 0);
      const jsonCols = Object.entries(rule.jsonColumns)
        .map(([col, keys]) => ({ idx: header.cols.indexOf(col), keys }))
        .filter((x) => x.idx >= 0);
      for (const row of rows) {
        if (row === '') {
          out.push(row);
          continue;
        }
        const fields = row.split('\t');
        let touched = false;
        for (const k of plainCols) {
          if (fields[k] !== undefined && fields[k] !== NULL_TOKEN) {
            fields[k] = NULL_TOKEN;
            stats.redactedFields += 1;
            touched = true;
          }
        }
        for (const { idx, keys } of jsonCols) {
          const raw = fields[idx];
          if (raw === undefined || raw === NULL_TOKEN || raw === '') continue;
          const decoded = unescapeCopyField(raw);
          if (decoded === null) {
            stats.warnings.push(`${header.table}.${header.cols[idx]}: 含未支持转义，保留原值（门禁将复查）`);
            continue;
          }
          let parsed: unknown;
          try {
            parsed = JSON.parse(decoded);
          } catch {
            stats.warnings.push(`${header.table}.${header.cols[idx]}: JSON 解析失败，保留原值（门禁将复查）`);
            continue;
          }
          if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) continue;
          const obj = parsed as Record<string, unknown>;
          let keyTouched = false;
          for (const key of keys) {
            if (key in obj) {
              delete obj[key];
              keyTouched = true;
            }
          }
          if (keyTouched) {
            fields[idx] = escapeCopyField(JSON.stringify(obj));
            stats.redactedFields += 1;
            touched = true;
          }
        }
        if (touched) stats.redactedRows += 1;
        out.push(fields.join('\t'));
      }
    }
    if (endIdx < lines.length) out.push(lines[endIdx]); // `\.` 终止行
    i = endIdx;
  }

  return out.join('\n');
}

/**
 * 具名列 INSERT 形态脱敏（`pg_dump --column-inserts`）——与 COPY 路径**同规则**（`RULES` 单一源）。
 * 规则表全为 `isahl_meta.*` ⇒ 按 table 末段名匹配（dump 语境下无同名异 schema 表）。
 */
function redactInsertForm(sql: string, stats: RedactStats): string {
  const edits: { start: number; end: number; text: string }[] = [];
  for (const stmt of splitStatements(sql)) {
    const shape = scanStatement(stmt.text);
    if (!shape || shape.kind !== 'insert') continue;
    const rule = matchRule(shape.qual, shape.table);
    if (!rule) continue;
    if (rule.mode === 'drop-data') {
      edits.push({ start: stmt.offset, end: stmt.offset + stmt.text.length, text: '' });
      stats.droppedRows += 1;
      if (!stats.droppedTables.includes(rule.table)) stats.droppedTables.push(rule.table);
      continue;
    }
    if (!shape.cols) {
      stats.warnings.push(`${rule.table}: 位置列 INSERT 无法按列名脱敏，保留原值（门禁将复查）`);
      continue;
    }
    const tuple = shape.tuples[0];
    if (!tuple) continue;
    let touched = false;
    for (const col of rule.columns) {
      const idx = shape.cols.indexOf(col);
      const item = idx >= 0 ? tuple[idx] : undefined;
      if (!item) continue;
      const value = stmt.text.slice(item.start, item.end);
      if (isNullLiteral(value)) continue;
      edits.push({ start: stmt.offset + item.start, end: stmt.offset + item.end, text: 'NULL' });
      stats.redactedFields += 1;
      touched = true;
    }
    for (const [col, keys] of Object.entries(rule.jsonColumns)) {
      const idx = shape.cols.indexOf(col);
      const item = idx >= 0 ? tuple[idx] : undefined;
      if (!item) continue;
      const value = stmt.text.slice(item.start, item.end);
      const stripped = stripJsonKeys(value, keys);
      if (stripped === null) {
        stats.warnings.push(`${rule.table}.${col}: 非字符串字面量或 JSON 解析失败，保留原值（门禁将复查）`);
        continue;
      }
      if (stripped !== value) {
        edits.push({ start: stmt.offset + item.start, end: stmt.offset + item.end, text: stripped });
        stats.redactedFields += 1;
        touched = true;
      }
    }
    if (touched) stats.redactedRows += 1;
  }
  return applySpanEdits(sql, edits);
}

/** 字符串字面量结束后第一个引号位（`''` 视为转义）；非引号起首 → -1 */
function closingQuoteIndex(literal: string): number {
  if (literal[0] !== "'") return -1;
  for (let i = 1; i < literal.length; i++) {
    if (literal[i] !== "'") continue;
    if (literal[i + 1] === "'") {
      i++;
      continue;
    }
    return i;
  }
  return -1;
}

/** 字符串字面量 → `{ text, suffix }`（`''` 解码；尾缀 cast 原样保留在 suffix）；非字符串字面量 → null */
export function decodeSqlStringLiteral(literal: string): { text: string; suffix: string } | null {
  const t = literal.trim();
  const endQuote = closingQuoteIndex(t);
  if (endQuote < 0) return null;
  return { text: t.slice(1, endQuote).split("''").join("'"), suffix: t.slice(endQuote + 1).trim() };
}

/** JSON 字面量去键：`'{"a":1}'::jsonb` → 去键重编码（保留尾缀 cast）；未变 / 非 JSON → 原串或 null */
function stripJsonKeys(literal: string, keys: readonly string[]): string | null {
  const decoded = decodeSqlStringLiteral(literal);
  if (!decoded) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(decoded.text);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
  const obj = parsed as Record<string, unknown>;
  let touched = false;
  for (const key of keys) {
    if (key in obj) {
      delete obj[key];
      touched = true;
    }
  }
  if (!touched) return literal;
  return `'${JSON.stringify(obj).split("'").join("''")}'${decoded.suffix}`;
}
