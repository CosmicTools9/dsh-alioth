/**
 * sql-scope.ts — SQL **内容级**「冻结面 / 声明面」判定的唯一实现
 *
 * 调用方（MUST NOT 各自复刻判定）：
 *   - 提交面门禁 `scripts/check/check-isahl-ddl-boundary.ts`
 *   - OMP 运行期守卫 `.omp/extensions/{db-pipeline-guard,safety-guard}.ts`
 *
 * 正本：
 *   - `db-schema-boundary.isahl-schema-frozen`：isahl 拦集合 = `CREATE TABLE` 族 /
 *     `DROP TABLE` / `ALTER TABLE` 的加列·删列·列改类型·改名；约束增删、索引增删、
 *     触发器启停、默认值设置/移除属**允许面**；`TRUNCATE` 归提交面（OMP 运行期不拦）。
 *   - `isahl_meta.meta_collections` / `meta_fields` 写入 = 直写声明面锚点（绕过模型中心），
 *     与建表同级。
 *   - `docs/specs/NO_REGEX_FOR_PARSING.md`：SQL 结构语义判定 MUST 用真解析器
 *     （libpg_query / pg-query-emscripten），MUST NOT 用正则模拟解析器。
 *
 * 分工：
 *   - 语句切分 = `scripts/lib/sql-split.ts`（词法状态机：引号 / 注释 / dollar-quote / COPY 数据区）
 *   - 结构判定 = libpg_query AST → `classifyStatement()`
 *   - 词法层只有两个用途：① 候选预筛（哪些语句值得送解析器）；② 解析失败时的形态兜底
 *     （parse 失败 = 无法证明无违规，由调用方按 fail-closed 处理）
 */
import { splitStatements } from './sql-split.ts';

/** 命中对象相对冻结面的分类（调用方据此决定驳回 / 放行） */
export type SchemaClass =
  /** isahl schema = Alioth 模型管理空间（冻结面） */
  | 'isahl'
  /** isahl_meta 声明面锚点（白名单唯一来源，写入 = 伪造声明面） */
  | 'anchor'
  /** 未限定 schema（search_path 可能落 isahl）或未知 schema → 信息不足 */
  | 'unknown'
  /** 允许面 schema（配套工程 / 业务扩展 / 临时表） */
  | 'other';

export interface ScopeHit {
  /** 动作签名：`CREATE TABLE` / `CREATE TABLE AS` / `ALTER TABLE [<subtypes>]` /
   *  `RENAME TABLE` / `RENAME COLUMN` / `DROP TABLE` / `TRUNCATE` / `ANCHOR WRITE [<verb>]` */
  kind: string;
  /** 目标对象（`schema.name`；未限定则为裸名） */
  target: string;
  schemaClass: SchemaClass;
}

export interface PgRangeVar {
  schemaname?: string;
  relname?: string;
  relpersistence?: string;
}
export interface PgStmtShape {
  CreateStmt?: { relation?: PgRangeVar };
  CreateTableAsStmt?: { into?: { rel?: PgRangeVar }; objtype?: string };
  AlterTableStmt?: { relation?: PgRangeVar; cmds?: Array<{ AlterTableCmd?: { subtype?: string } }> };
  DropStmt?: {
    removeType?: string;
    objects?: Array<{ List?: { items?: Array<{ String?: { sval?: string } }> } }>;
  };
  TruncateStmt?: { relations?: Array<{ RangeVar?: PgRangeVar }> };
  RenameStmt?: { renameType?: string; relation?: PgRangeVar; subname?: string; newname?: string };
  InsertStmt?: { relation?: PgRangeVar };
  UpdateStmt?: { relation?: PgRangeVar };
  DeleteStmt?: { relation?: PgRangeVar };
  CopyStmt?: { relation?: PgRangeVar; is_from?: boolean };
  SelectStmt?: { intoClause?: { rel?: PgRangeVar } };
  DoStmt?: { args?: Array<{ DefElem?: { defname?: string; arg?: { String?: { sval?: string } } } }> };
}
/** libpg_query 语句节点（`parse_tree.stmts[i]`） */
export interface StmtNode {
  stmt: PgStmtShape;
  stmt_location?: number;
  stmt_len?: number;
}

/** 允许面 schema 名单：isahl_* 配套工程 + 业务扩展 schema（未限定 / 未知 → `unknown`） */
const ALLOWED_SCHEMA = /^(?:isahl_[A-Za-z0-9_]+|wz_fssc|ag_catalog|public|staging)$/;
/** 声明面锚点（写入 = 绕过模型中心直写模型定义） */
const ANCHOR_TABLES: Record<string, true> = { meta_collections: true, meta_fields: true };

/**
 * ALTER TABLE 拦集合（显式列举，与 `db-schema-boundary.isahl-schema-frozen` 正本一致）：
 * 加列 / 删列 / 列改类型 = 改变列集合或列身份的动作。
 * 约束增删、索引增删、触发器启停、默认值设置/移除属允许面。新增子型默认放行——
 * 本集合只增不改（禁止回到「非白名单即拦」）。
 */
const BLOCKED_ALTER_SUBTYPES: Record<string, true> = {
  AT_AddColumn: true,
  AT_DropColumn: true,
  AT_AlterColumnType: true,
};

function schemaClassOf(rv: PgRangeVar | undefined): SchemaClass {
  if (!rv) return 'unknown';
  if (rv.relpersistence === 't') return 'other'; // 临时表落 pg_temp，结构上无法指向 isahl
  const s = rv.schemaname;
  if (s === 'isahl') return 'isahl';
  if (s && ALLOWED_SCHEMA.test(s)) return 'other';
  return 'unknown';
}

function targetOf(rv: PgRangeVar): string {
  return [rv.schemaname, rv.relname].filter(Boolean).join('.') || '?';
}

function anchorVerb(rv: PgRangeVar | undefined): string | null {
  if (!rv?.relname) return null;
  if (rv.schemaname === 'isahl_meta' && ANCHOR_TABLES[rv.relname] === true) return rv.relname;
  return null;
}

/**
 * 数据面动作（非结构动作）：**不升级 cause**——它们不在 `isahl` 冻结的拦截面内
 * （拦截面 = 表族结构动作四类，见 `db-schema-boundary::isahl-schema-frozen`）。
 * `TRUNCATE` 的防护由提交面门禁 `check-isahl-ddl-boundary.ts` 单层承担；
 * 无 `WHERE` 的 `UPDATE`/`DELETE` 与 `COPY … FROM` 无运行期拦截。
 */
const NON_STRUCTURAL_KINDS: Record<string, true> = {
  TRUNCATE: true,
  'UPDATE-NO-WHERE': true,
  'DELETE-NO-WHERE': true,
  'COPY-FROM': true,
};

/**
 * AST → 命中（`null` = 该语句不在冻结面 / 声明面上）。
 *
 * 纯函数：不做 IO、不递归（`DO` 块体的递归由 `decideSqlScope` 承担）。
 */
export function classifyStatement(rs: StmtNode): ScopeHit | null {
  const s = rs.stmt;
  if (s.CreateStmt) {
    const rel = s.CreateStmt.relation;
    if (!rel) return null;
    return { kind: 'CREATE TABLE', target: targetOf(rel), schemaClass: schemaClassOf(rel) };
  }
  // CREATE TABLE … AS / SELECT … INTO：同属 `CREATE TABLE` 族（objtype=OBJECT_TABLE）。
  // CREATE MATERIALIZED VIEW（OBJECT_MATVIEW）属允许面，不拦。
  if (s.CreateTableAsStmt) {
    if (s.CreateTableAsStmt.objtype !== 'OBJECT_TABLE') return null;
    const rel = s.CreateTableAsStmt.into?.rel;
    if (!rel) return null;
    return { kind: 'CREATE TABLE AS', target: targetOf(rel), schemaClass: schemaClassOf(rel) };
  }
  if (s.SelectStmt?.intoClause?.rel) {
    const rel = s.SelectStmt.intoClause.rel;
    return { kind: 'CREATE TABLE AS', target: targetOf(rel), schemaClass: schemaClassOf(rel) };
  }
  if (s.AlterTableStmt) {
    const a = s.AlterTableStmt;
    const subtypes = (a.cmds ?? [])
      .map((c) => c.AlterTableCmd?.subtype)
      .filter((st): st is string => typeof st === 'string');
    const blocked = subtypes.filter((st) => BLOCKED_ALTER_SUBTYPES[st] === true);
    if (blocked.length === 0 || !a.relation) return null; // 允许子句：约束 / 索引 / 触发器 / 默认值
    return {
      kind: `ALTER TABLE [${blocked.join(', ')}]`,
      target: targetOf(a.relation),
      schemaClass: schemaClassOf(a.relation),
    };
  }
  // 改名（RENAME COLUMN / RENAME TO）在 PG 中解析为 RenameStmt 而非 AlterTableStmt。
  // RENAME CONSTRAINT（OBJECT_TABCONSTRAINT）属约束面 → 允许。
  if (s.RenameStmt) {
    const r = s.RenameStmt;
    const isColumn = r.renameType === 'OBJECT_COLUMN';
    const isTable = r.renameType === 'OBJECT_TABLE';
    if ((!isColumn && !isTable) || !r.relation) return null;
    return {
      kind: isColumn ? 'RENAME COLUMN' : 'RENAME TABLE',
      target: `${targetOf(r.relation)}${r.subname ? `.${r.subname}` : ''} → ${r.newname ?? '?'}`,
      schemaClass: schemaClassOf(r.relation),
    };
  }
  if (s.DropStmt) {
    const d = s.DropStmt;
    if (d.removeType !== 'OBJECT_TABLE') return null;
    for (const obj of d.objects ?? []) {
      // 实测形状（libpg_query v16）：限定名 = `{ List: { items: [String, String?] } }`
      const parts = (obj?.List?.items ?? [])
        .map((it) => it?.String?.sval)
        .filter((v): v is string => typeof v === 'string');
      if (parts.length === 0) continue;
      const rv: PgRangeVar =
        parts.length >= 2 ? { schemaname: parts[0], relname: parts[1] } : { relname: parts[0] };
      return { kind: 'DROP TABLE', target: targetOf(rv), schemaClass: schemaClassOf(rv) };
    }
    return null;
  }
  if (s.TruncateStmt) {
    for (const rel of s.TruncateStmt.relations ?? []) {
      const rv = rel?.RangeVar;
      if (!rv) continue;
      if (anchorVerb(rv)) {
        return { kind: 'ANCHOR WRITE [TRUNCATE]', target: targetOf(rv), schemaClass: 'anchor' };
      }
      return { kind: 'TRUNCATE', target: targetOf(rv), schemaClass: schemaClassOf(rv) };
    }
    return null;
  }
  for (const [key, verb] of [
    ['InsertStmt', 'INSERT'],
    ['UpdateStmt', 'UPDATE'],
    ['DeleteStmt', 'DELETE'],
  ] as const) {
    const st = s[key] as { relation?: PgRangeVar; whereClause?: unknown } | undefined;
    if (!st?.relation) continue;
    if (anchorVerb(st.relation)) {
      return { kind: `ANCHOR WRITE [${verb}]`, target: targetOf(st.relation), schemaClass: 'anchor' };
    }
    // 危险数据操作：无 WHERE 的整表改写 → 需窗类（不升级 cause）
    if (verb !== 'INSERT' && !st.whereClause) {
      return {
        kind: `${verb}-NO-WHERE`,
        target: targetOf(st.relation),
        schemaClass: schemaClassOf(st.relation),
      };
    }
    return null;
  }
  if (s.CopyStmt?.relation && s.CopyStmt.is_from) {
    if (anchorVerb(s.CopyStmt.relation)) {
      return {
        kind: 'ANCHOR WRITE [COPY]',
        target: targetOf(s.CopyStmt.relation),
        schemaClass: 'anchor',
      };
    }
    // 批量导入（COPY … FROM）→ 需窗类（不升级 cause）
    return {
      kind: 'COPY-FROM',
      target: targetOf(s.CopyStmt.relation),
      schemaClass: schemaClassOf(s.CopyStmt.relation),
    };
  }
  return null;
}

// ── 解析器装载（惰性；pg-query-emscripten 在 .d.ts 中无类型面，此处声明最小结构） ──
interface PgParser {
  parse(sql: string): { error?: { message?: string }; parse_tree?: { stmts?: StmtNode[] } };
}
type PgParserCtor = new () => PgParser | Promise<PgParser>;
let parserCtor: Promise<PgParserCtor> | null = null;

function loadParser(): Promise<PgParserCtor> {
  if (!parserCtor) {
    parserCtor = import('pg-query-emscripten').then((m) => {
      const mod = m as unknown as { default?: PgParserCtor };
      return (mod.default ?? (m as unknown as PgParserCtor)) as PgParserCtor;
    });
  }
  return parserCtor;
}

/** 单语句解析上限：pg-query-emscripten 封装对 >~55KB 输入内存损坏（实测 52KB 正常 / 60KB 崩溃） */
export const OVERSIZE_BYTES = 50_000;

/**
 * 解析 SQL 文本 → 语句节点（每次调用**新建 wasm 实例**：封装每次 parse 泄漏输入缓冲，
 * 复用实例累计 ~55KB 后堆损坏；单实例单 parse 无累计）。
 * 超限（`OVERSIZE_BYTES`）或异常 → `{ error }`（调用方 fail-closed）。
 */
export async function parseStatements(text: string): Promise<{ stmts: StmtNode[] } | { error: string }> {
  if (Buffer.byteLength(text, 'utf8') > OVERSIZE_BYTES) {
    return { error: `语句超 ${OVERSIZE_BYTES}B 上限（wasm 安全阈值）` };
  }
  try {
    const Ctor = await loadParser();
    const pg = await (new Ctor() as PgParser | Promise<PgParser>);
    const r = pg.parse(text);
    if (r.error) return { error: r.error.message ?? JSON.stringify(r.error) };
    return { stmts: r.parse_tree?.stmts ?? [] };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

// ── 词法层：候选预筛 + 解析失败兜底（只作用于「语句头」/「已抽取字面量」） ──

/**
 * DDL 形态判定（解析失败时的 fail-closed 依据）：只看语句起始的动词位——
 * COPY/INSERT 的数据值深处可能嵌 `ALTER TABLE` 字样（如审计留痕文本），不算 DDL 形态。
 */
export function looksLikeDdl(stmtText: string): boolean {
  const body = stripLeadingComments(stmtText);
  return /\b(CREATE|ALTER|DROP|TRUNCATE)\s+(TABLE|SCHEMA|DATABASE|VIEW|MATERIALIZED|FUNCTION|INDEX|TYPE|DOMAIN|ROLE)\b/i.test(
    body.slice(0, 200),
  );
}

/** 剥前导空白 / 行注释 / 块注释（语句头提取用） */
function stripLeadingComments(text: string): string {
  let rest = text;
  for (;;) {
    const stripped = rest.replace(/^\s+/, '');
    if (stripped.startsWith('--')) {
      const nl = stripped.indexOf('\n');
      rest = nl === -1 ? '' : stripped.slice(nl + 1);
      continue;
    }
    if (stripped.startsWith('/*')) {
      const end = stripped.indexOf('*/');
      rest = end === -1 ? '' : stripped.slice(end + 2);
      continue;
    }
    return stripped;
  }
}

/** 需要送解析器的语句头（可执行结构动作 / 声明面写入 / 动态命令行） */
const CANDIDATE_HEADS: Record<string, true> = {
  create: true,
  alter: true,
  drop: true,
  truncate: true,
  insert: true,
  update: true,
  delete: true,
  merge: true,
  copy: true,
  do: true,
};

export interface StatementHead {
  /** 语句头关键词（小写）；取不到时为 `''` */
  head: string;
  /** 语句头是否是 shell 展开（`$VAR` / `${…}` / 反引号）→ 内容不可见 */
  opaque: boolean;
  /** 是否值得送解析器（候选） */
  candidate: boolean;
}

/**
 * 语句头提取（词法层，只读首 token；不做 SQL 结构解析）。
 * `with` / `select` 仅当文本含 `INTO`（`SELECT … INTO` / `WITH … INSERT`）时才算候选——
 * 读命令不因此触达解析器（读路径零解析是硬要求：拦截 MUST NOT 误伤只读查询）。
 */
export function statementHead(stmtText: string): StatementHead {
  const body = stripLeadingComments(stmtText);
  const m = /^([A-Za-z_][A-Za-z0-9_]*|\$\{?[A-Za-z_0-9]*\}?|`)/.exec(body);
  if (!m) return { head: '', opaque: body.length > 0 && body.startsWith('$'), candidate: false };
  if (m[1].startsWith('$') || m[1].startsWith('`')) return { head: '', opaque: true, candidate: false };
  const head = m[1].toLowerCase();
  const candidate =
    head === 'with' || head === 'select' ? /\binto\b/i.test(body) : CANDIDATE_HEADS[head] === true;
  return { head, opaque: false, candidate };
}

// ── 解析失败时的形态兜底（与 AST 判定同语义的兜底面，只在 parse 失败时启用） ──
const DDL_TABLE_HEAD = /\b(CREATE|ALTER|DROP)\s+TABLE\b/i;
// 限定名两个标识符都要捕获：`schema.name` / `name`（旧版把「点号」当第二组，relname 恒为 '?'）
const DDL_TARGET =
  /^\s*(?:IF\s+(?:NOT\s+)?EXISTS\s+)?["'`]?([A-Za-z_][A-Za-z0-9_]*)["'`]?(?:\s*\.\s*["'`]?([A-Za-z_][A-Za-z0-9_]*)["'`]?)?/i;
const TEMP_CREATE = /\bCREATE\s+(?:TEMP|TEMPORARY)\s+TABLE\b/i;
const ALTER_BLOCK_MARKERS: RegExp[] = [
  /\bRENAME\s+(?!CONSTRAINT\b)/i,
  /\bTYPE\b/i,
  /\bADD\s+(?!CONSTRAINT\b|PRIMARY\b|UNIQUE\b|FOREIGN\b|CHECK\b|EXCLUDE\b)/i,
  /\bDROP\s+(?!CONSTRAINT\b)/i,
  /\b(?:SET|DROP)\s+NOT\s+NULL\b/i,
];
const ANCHOR_DML = /\b(INSERT\s+INTO|UPDATE|DELETE\s+FROM|TRUNCATE|COPY)\s+isahl_meta\.(meta_collections|meta_fields)\b/i;

/**
 * 词法兜底命中的两个触发场景（**均不解析结构，只做形态识别**）：
 *   ① 候选语句真解析失败（无法证明无违规 → fail-closed）；
 *   ② `DO $$ … $$` 块体（plpgsql 命令式文本：`BEGIN`/`IF` 前缀使其非合法 SQL 语句，
 *      语句切分后取不到结构动作头 → 按动态命令行文本做词法扫描）。
 */
/** 剥离 SQL 注释（行 `--` 与块注释）：注释中的动作词不构成结构动作（见 `lexicalHits` 注释）。 */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/--[^\n]*/g, " ");
}

function lexicalHits(text: string): ScopeHit[] {
  const hits: ScopeHit[] = [];
  // 扫描前剥离 SQL 注释（两个调用点语义必须一致：词法兜底 + DO 块的「DDL 形态」判定）——
  // 注释里出现 `DROP TABLE` 等词不构成结构动作；旧版扫原文 ⇒ 引导/迁移注释中的说明文字
  // 产生假命中（实测 DO 块内注释提及 `DROP TABLE` 即误判 `cause=unknown` 驳回）。
  // 剥离后再取下标/切片，保证文本一致。
  const scan = stripComments(text);
  const anchor = ANCHOR_DML.exec(scan);
  if (anchor) {
    hits.push({
      kind: `ANCHOR WRITE [${anchor[1].split(/\s+/)[0].toUpperCase()}]`,
      target: `isahl_meta.${anchor[2]}`,
      schemaClass: 'anchor',
    });
  }
  for (const m of scan.matchAll(new RegExp(DDL_TABLE_HEAD.source, 'gi'))) {
    const start = m.index ?? 0;
    const nl = scan.indexOf(';', start);
    const stmt = scan.slice(start, nl === -1 ? scan.length : nl);
    const isAlter = /^ALTER$/i.test(m[1]);
    if (isAlter && !ALTER_BLOCK_MARKERS.some((re) => re.test(stmt))) continue; // 允许子句
    if (TEMP_CREATE.test(stmt)) continue; // 临时表无法指向 isahl
    const t = DDL_TARGET.exec(stmt.slice(m[0].length));
    const rv: PgRangeVar = t?.[2] ? { schemaname: t[1], relname: t[2] } : { relname: t?.[1] ?? '?' };
    hits.push({ kind: `${m[1].toUpperCase()} TABLE`, target: targetOf(rv), schemaClass: schemaClassOf(rv) });
  }
  return hits;
}

// ── 对外主入口 ──
/**
 * 判定原因（调用方据此决定姿态）：
 *  - `anchor`：声明面锚点写入 → 硬拦无确认口
 *  - `isahl`：isahl 冻结面结构动作 → 硬拦无确认口
 *  - `unknown`：目标未限定 / 未知 schema → 无法证明安全（调用方一律驳回）
 *  - `unresolvable`：内容不可见（shell 展开 / 解析失败且呈 DDL 形态）→ 信息不足
 *  - `null`：放行
 */
export type ScopeCause = 'anchor' | 'isahl' | 'unknown' | 'unresolvable' | null;

export interface ScopeDecision {
  cause: ScopeCause;
  hits: ScopeHit[];
  /** 不可判定内容的片段（供确认正文展示） */
  opaque: string[];
}

/**
 * 内容级判定（异步：候选语句走 libpg_query；只读语句零解析）。
 *
 * `DO $$ … $$` 块体的内联命令按**动态命令行文本**词法扫描（plpgsql 非 SQL，无结构可解析）；
 * 函数体（`CREATE FUNCTION … AS $$ … $$`）不扫描——其 DDL 由 G1 库内事件触发器兜底。
 */
export async function decideSqlScope(sql: string): Promise<ScopeDecision> {
  const hits: ScopeHit[] = [];
  const opaque: string[] = [];
  for (const st of splitStatements(sql)) {
    const h = statementHead(st.text);
    if (h.opaque) {
      opaque.push(st.text.trim().slice(0, 120));
      continue;
    }
    if (!h.candidate) continue;
    const parsed = await parseStatements(st.text);
    if ('error' in parsed) {
      const fb = lexicalHits(st.text);
      if (fb.length > 0) hits.push(...fb);
      else if (looksLikeDdl(st.text)) opaque.push(st.text.trim().slice(0, 120));
      continue;
    }
    for (const rs of parsed.stmts) {
      const hit = classifyStatement(rs);
      if (hit) {
        hits.push(hit);
        continue;
      }
      for (const arg of rs.stmt.DoStmt?.args ?? []) {
        const body = arg.DefElem?.defname === 'as' ? arg.DefElem?.arg?.String?.sval : undefined;
        if (!body) continue;
        const inner = lexicalHits(body);
        if (inner.length > 0) hits.push(...inner);
        else if (
          /\b(?:CREATE|ALTER|DROP|TRUNCATE)\s+(?:TABLE|VIEW|FUNCTION|INDEX|TYPE)\b/i.test(
            stripComments(body),
          )
        ) {
          opaque.push(body.trim().slice(0, 120));
        }
      }
    }
  }
  const cause: ScopeCause = hits.some((h) => h.schemaClass === 'anchor')
    ? 'anchor'
    : hits.some((h) => h.schemaClass === 'isahl' && !NON_STRUCTURAL_KINDS[h.kind])
      ? 'isahl'
      : hits.some((h) => h.schemaClass === 'unknown' && !NON_STRUCTURAL_KINDS[h.kind])
        ? 'unknown'
        : opaque.length > 0
          ? 'unresolvable'
          : null;
  return { cause, hits, opaque };
}
