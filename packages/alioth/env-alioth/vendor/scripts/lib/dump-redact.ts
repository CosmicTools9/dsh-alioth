/**
 * dump-redact.ts — pg_dump 数据导出的环境凭据脱敏（共享实现）
 *
 * 调用方：`scripts/db/redact-secrets.ts`（备份管线 / 历史快照清洗 CLI）与
 * `scripts/check/check-no-secrets-in-backup.ts`（门禁）。规则只有一份——脱敏与
 * 校验必须同源，否则门禁会与清洗漂移。
 *
 * 策略（`openspec/changes/remove-secrets-from-git-backup`）：
 *   1. `drop-data` 表（`isahl_meta.meta_llm_configs`）：凭据列 `api_key_enc` 为 NOT NULL，
 *      脱敏后行无意义 → 整段 `COPY` 数据删除（表结构仍在 `ddl/`）。
 *   2. `redact` 表（`isahl_meta.meta_data_source`）：行本身是数据源定义（恢复所需）→
 *      仅置空凭据列，并从 JSON 列删除凭据键。
 *
 * 解析约定（`NO_REGEX_FOR_PARSING` §判定方法 判定记录）：
 * pg_dump 纯文本导出为「段标记 + `COPY <表> (<列...>) FROM stdin;` + 制表符分隔行 + `\.`」。
 * 本实现用 `startsWith`/`indexOf`/`split('\t')` 做**结构性切分**，不用正则模拟解析器：
 * COPY 文本格式中字面制表符在值内被转义为 `\t`（同理换行 `\n`、反斜杠 `\\`），
 * 因此按字面制表符切分即是精确列切分。字段先反转义才能解析 JSON，再正向转义回写。
 */

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
  // 数据源定义：保留行（host/port/库名等恢复所需），清掉凭据列与 JSON 内凭据键
  {
    table: 'isahl_meta.meta_data_source',
    mode: 'redact',
    columns: ['password_encrypted', 'connection_string'],
    jsonColumns: { config: ['password_encrypted'] },
  },
  // mise 环境变量（`is_secret` 标注行）与运行期令牌：保留行与键名，值一律清空
  { table: 'isahl_meta.meta_mise_env_vars', mode: 'redact', columns: ['var_value'], jsonColumns: {} },
  { table: 'isahl_meta.meta_mise_services', mode: 'redact', columns: ['run_token'], jsonColumns: {} },
];

export const NULL_TOKEN = '\\N';

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
