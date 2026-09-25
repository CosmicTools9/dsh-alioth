/**
 * test-data-rules.ts — 运行时/历史面与测试残留判据的**唯一实现**（TS 侧）
 *
 * 清单真相源（与 `scripts/db/lib/runtime-data.sh` 同源，MUST NOT 内联第二份）：
 *   scripts/db/runtime-data-tables.txt   整表面：`schema.table` / `schema.*`
 *   scripts/db/test-data-patterns.txt    行级面：`<schema>.<table|* >.<column> = <POSIX ERE>`
 *
 * 消费方：`scripts/db/strip-test-data.ts`（dump 面剥离）+ `scripts/check/check-snapshot-runtime-rows.ts`
 * （快照门禁）——两者共用本模块的解析、判据与扫描实现（禁止第二套）。
 *
 * 解析约定（`NO_REGEX_FOR_PARSING.md` §判定方法 判定记录）：语句结构定位复用
 * `scripts/lib/dump-statements.ts`（字符状态机）+ `scripts/lib/sql-split.ts`（切分）；
 * 正则仅作用于**已由解析器抽出的叶值**（解码后的字符串字面量），MUST NOT 用于结构提取。
 *
 * 大小写语义：判据正则按 `i` 编译，与 SQL 侧 `~*` 语义对齐（同一份 pattern 两引擎同义）。
 *
 * 位置列载荷（pg_dump `--inserts`，无列清单）的列位来源（`ScanOptions.columnOrder`）：
 *   · `columnOrder`（DB `pg_attribute` 顺序，与 pg_dump 取值顺序同源）可用 ⇒ **列位精确**：
 *     只对判据列取值匹配，且取值个数 MUST 等于列数（不等即抛错——禁止静默错位）；
 *   · 不可用 ⇒ 由调用方按面选择 `positional: 'value'`（取值级，过宽）或 `'skip'`（不判定）。
 *     dump 面（backup-ddl）MUST 走列位精确；快照门禁无 DB 时 MUST 走 skip 并明示，
 *     MUST NOT 用过宽的取值级判据阻断推送（假阳性）。
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { decodeSqlStringLiteral } from './dump-redact.ts';
import { isNullLiteral, scanStatement, type StatementShape } from './dump-statements.ts';
import { splitStatements } from './sql-split.ts';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const RUNTIME_LIST_PATH = resolve(ROOT, 'scripts/db/runtime-data-tables.txt');
export const PATTERN_LIST_PATH = resolve(ROOT, 'scripts/db/test-data-patterns.txt');
export const LOCAL_SECRET_LIST_PATH = resolve(ROOT, 'scripts/db/local-secret-tables.txt');

/**
 * 机器本地机密表（唯一真相源 = `scripts/db/local-secret-tables.txt`，与 lib/local-secrets.sh 同源）：
 * 即便落在运行时面 schema（如 `isahl_auth.*`）MUST 跳过——取值只能 UI/环境录入、种子无法重建。
 * 归一化：去引号 + 保留 `schema.table` 形态（`isahl_auth."outbound_client"` → `isahl_auth.outbound_client`）。
 */
export function loadLocalSecretTables(path: string = LOCAL_SECRET_LIST_PATH): Set<string> {
  const out = new Set<string>();
  if (!existsSync(path)) return out;
  for (const raw of readFileSync(path, 'utf8').split('\n')) {
    const line = raw.trim();
    if (line.length === 0 || line.startsWith('#')) continue;
    out.add(line.replace(/"/g, ''));
  }
  return out;
}

export interface RuntimeEntry {
  /** 清单原样（`schema.table` 或 `schema.*`） */
  qual: string;
  schema: string;
  /** 表名或 `*` */
  table: string;
}

export interface RowRule {
  schema: string;
  /** 表名或 `*` */
  table: string;
  column: string;
  pattern: string;
  re: RegExp;
}

export interface ScanOptions {
  /**
   * 是否施加「运行时整表面」判据（默认 true）。ns 内容种子面（`seed/ns/**`）的表集由各 ns 的
   * `seed-manifest.json` 唯一声明（`db-script-scope-contract`），与该面比对本清单属跨面误判
   * ⇒ 该面 MUST 传 false（行级判据仍全face生效）。
   */
  runtimeTables?: boolean;
  /** qual（`schema.table`）→ 物理列序（pg_attribute.attnum 序，与 pg_dump 取值顺序同源） */
  columnOrder?: Map<string, string[]>;
  /** 位置列载荷策略：precise 需 columnOrder；value = 取值级（过宽）；skip = 不判定 */
  positional?: 'precise' | 'value' | 'skip';
}

export interface Manifests {
  runtime: RuntimeEntry[];
  rules: RowRule[];
  /** 机器本地机密表（`schema.table`，去引号）——运行时面内的例外，MUST NOT 判违规/清理 */
  secretTables: Set<string>;
}

export interface Violation {
  kind: 'runtime-table' | 'test-row';
  /** 语句内出现的限定表名 */
  qual: string;
  /** 命中的清单项或判据（可读形态） */
  detail: string;
  /** 命中取值（test-row 时提供，供人工复核） */
  value?: string;
  /** 位置列载荷且按**取值级**（过宽）命中——复核时须知；列位精确命中不带该标记 */
  positional?: boolean;
  /** 语句文本的起始偏移（调用方换算行号） */
  offset: number;
}

/** 加载并校验两清单：缺失 / 为空 / 行不可解析 / 正则不可编译 → 抛出（fail-closed，禁止 no-op 降级） */
export function loadManifests(
  runtimePath: string = RUNTIME_LIST_PATH,
  patternPath: string = PATTERN_LIST_PATH,
): Manifests {
  if (!existsSync(runtimePath)) throw new Error(`整表清单缺失：${runtimePath}`);
  if (!existsSync(patternPath)) throw new Error(`行级判据清单缺失：${patternPath}`);

  const lines = (p: string): string[] =>
    readFileSync(p, 'utf8')
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith('#'));

  const runtime: RuntimeEntry[] = [];
  for (const line of lines(runtimePath)) {
    const dot = line.indexOf('.');
    if (dot <= 0 || dot === line.length - 1) throw new Error(`整表清单行不可解析「${line}」（期望 schema.table 或 schema.*）`);
    const schema = line.slice(0, dot);
    const table = line.slice(dot + 1);
    if (table.includes('.')) throw new Error(`整表清单行不可解析「${line}」（表段 MUST NOT 含点号）`);
    runtime.push({ qual: line, schema, table });
  }
  if (runtime.length === 0) throw new Error(`整表清单为空：${runtimePath}（空清单 = 无排除面）`);

  const rules: RowRule[] = [];
  for (const line of lines(patternPath)) {
    const sep = line.indexOf(' = ');
    if (sep <= 0) throw new Error(`行级判据行不可解析「${line}」（期望 <schema>.<table|* >.<column> = <pattern>）`);
    const head = line.slice(0, sep);
    const pattern = line.slice(sep + 3);
    const segs = head.split('.');
    if (segs.length !== 3 || segs.some((s) => s.length === 0)) {
      throw new Error(`行级判据键不可解析「${head}」（期望 schema.table.column，table 可为 *）`);
    }
    if (pattern.length === 0) throw new Error(`行级判据缺模式「${line}」`);
    let re: RegExp;
    try {
      re = new RegExp(pattern, 'i');
    } catch (e) {
      throw new Error(`行级判据模式不可编译「${line}」：${(e as Error).message}`);
    }
    rules.push({ schema: segs[0]!, table: segs[1]!, column: segs[2]!, pattern, re });
  }
  if (rules.length === 0) throw new Error(`行级判据清单为空：${patternPath}`);

  return { runtime, rules, secretTables: loadLocalSecretTables() };
}

/** 命中运行时整表面？（`schema.*` 通配 + 显式表名） */
export function runtimeViolation(shape: StatementShape, m: Manifests): RuntimeEntry | undefined {
  const schema = shape.qual.includes('.') ? shape.qual.slice(0, shape.qual.lastIndexOf('.')) : '';
  for (const e of m.runtime) {
    if (e.schema !== schema) continue;
    if (e.table === '*' || e.table === shape.table) {
      // 机密表例外：本地面机密不得因「整 schema 属运行时面」被判违规/被清
      if (m.secretTables?.has(shape.qual)) return undefined;
      return e;
    }
  }
  return undefined;
}

/** 命中行级测试残留判据？（具名列按列定位；位置列退化为取值级） */
export function rowViolation(
  shape: StatementShape,
  m: Manifests,
  text: string,
  opts: ScanOptions = {},
): { rule: RowRule; value: string; level: 'column' | 'value' } | undefined {
  if (shape.kind !== 'insert' || shape.hasSelect) return undefined;
  const schema = shape.qual.includes('.') ? shape.qual.slice(0, shape.qual.lastIndexOf('.')) : '';
  const candidates = m.rules.filter(
    (r) => r.schema === schema && (r.table === '*' || r.table === shape.table),
  );
  if (candidates.length === 0) return undefined;

  // 列位解析：具名列直接用；位置列取 columnOrder（不可用则按策略处理）
  let columns: (string | null)[];
  if (shape.cols) {
    columns = shape.cols;
  } else {
    const order = opts.columnOrder?.get(shape.qual);
    const values = shape.tuples[0]?.length ?? 0;
    if (order) {
      if (order.length !== values) {
        throw new Error(
          `${shape.qual}: 位置列取值数 ${values} ≠ 物理列数 ${order.length}（并发 DDL 或列位来源漂移）——fail-loud，禁止静默错位判定`,
        );
      }
      columns = order;
    } else if (opts.positional === 'value') {
      columns = []; // 空数组 = 无列位信息 ⇒ 下面对每个取值都尝试判据（取值级）
    } else {
      return undefined; // 'skip'（默认）：位置列且无列位来源 ⇒ 不判定
    }
  }

  for (const tuple of shape.tuples) {
    for (let i = 0; i < tuple.length; i += 1) {
      const span = tuple[i]!;
      const literal = text.slice(span.start, span.end);
      if (isNullLiteral(literal)) continue;
      const value = decodeSqlStringLiteral(literal)?.text;
      if (value === undefined) continue;
      for (const rule of candidates) {
        if (columns.length > 0 && columns[i] !== rule.column) continue;
        if (rule.re.test(value)) return { rule, value, level: columns.length > 0 ? 'column' : 'value' };
      }
    }
  }
  return undefined;
}

/** 从 DB（`pg_attribute` 序）加载物理列序：返回 null = DB 不可达/查询失败（调用方定策略） */
export function loadColumnOrderFromDb(url: string, schemas: string[] = ['isahl', 'isahl_meta', 'isahl_audit']): Map<string, string[]> | null {
  const list = schemas.map((s) => `'${s}'`).join(',');
  const sql = `SELECT n.nspname||'.'||c.relname||E'\\t'||a.attname
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r','p') AND n.nspname IN (${list})
ORDER BY n.nspname, c.relname, a.attnum`;
  const r = spawnSync('psql', [url, '-tAc', sql], { encoding: 'utf-8', timeout: 60_000 });
  if (r.status !== 0 || !r.stdout) return null;
  const map = new Map<string, string[]>();
  for (const line of r.stdout.split('\n')) {
    if (!line) continue;
    const [qual, column] = line.split('\t');
    if (!qual || !column) continue;
    const arr = map.get(qual);
    if (arr) arr.push(column);
    else map.set(qual, [column]);
  }
  return map.size > 0 ? map : null;
}

/** 扫描 dump 文本：返回全部违规（运行时表数据 INSERT / 测试残留行） */
export function scanDump(sql: string, m: Manifests, opts: ScanOptions = {}): Violation[] {
  const out: Violation[] = [];
  for (const stmt of splitStatements(sql)) {
    const shape = scanStatement(stmt.text);
    if (!shape) continue;
    const rt = opts.runtimeTables === false ? undefined : runtimeViolation(shape, m);
    if (rt) {
      out.push({
        kind: 'runtime-table',
        qual: shape.qual,
        detail: rt.qual,
        offset: stmt.offset,
      });
      continue;
    }
    const row = rowViolation(shape, m, stmt.text, opts);
    if (row) {
      out.push({
        kind: 'test-row',
        qual: shape.qual,
        detail: `${row.rule.schema}.${row.rule.table === '*' ? '*' : row.rule.table}.${row.rule.column} = ${row.rule.pattern}`,
        value: row.value,
        positional: row.level === 'value',
        offset: stmt.offset,
      });
    }
  }
  return out;
}

/** 离线筛选：删除运行时表数据语句与测试残留行（dump 面 `--fix` 用） */
export function stripDump(sql: string, m: Manifests, opts: ScanOptions = {}): { out: string; removed: Violation[] } {
  const removed: Violation[] = [];
  const edits: { start: number; end: number }[] = [];
  for (const stmt of splitStatements(sql)) {
    const shape = scanStatement(stmt.text);
    if (!shape) continue;
    const rt = opts.runtimeTables === false ? undefined : runtimeViolation(shape, m);
    const row = rt ? undefined : rowViolation(shape, m, stmt.text, opts);
    if (!rt && !row) continue;
    edits.push({ start: stmt.offset, end: stmt.offset + stmt.text.length });
    removed.push(
      rt
        ? { kind: 'runtime-table', qual: shape.qual, detail: rt.qual, offset: stmt.offset }
        : {
            kind: 'test-row',
            qual: shape.qual,
            detail: `${row!.rule.schema}.${row!.rule.table}.${row!.rule.column}`,
            value: row!.value,
            positional: row!.level === 'value',
            offset: stmt.offset,
          },
    );
  }
  if (edits.length === 0) return { out: sql, removed };
  let out = '';
  let cursor = 0;
  for (const e of edits) {
    out += sql.slice(cursor, e.start);
    cursor = e.end;
  }
  out += sql.slice(cursor);
  return { out, removed };
}
