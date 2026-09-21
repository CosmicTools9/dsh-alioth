/**
 * code-inline-sql.ts — 「代码内联 SQL」通道判定（唯一实现）
 *
 * 定位：`db-schema-boundary::isahl-schema-frozen` 要求「经内联 psql、SQL 文件、**代码内联 SQL**
 * 的间接执行 MUST 一并拦截」。内联 psql 由 `psql-invocation.ts` 覆盖、SQL 文件由
 * `check-isahl-ddl-boundary.ts` 覆盖；本文件覆盖**代码内联**面：
 *   从 JS/TS 源码提取字符串/模板字面量 → 每个字面量交既有判定器
 *   （`psql-invocation.ts` 命令形态载荷 + `sql-scope.ts` SQL 形态真解析）。
 * 不新增解析器、不新增 SQL 判定、不写 SQL 结构正则。
 *
 * Python 面**无可用解析器**（仓内工具链无 Python parser，规约禁正则模拟解析器），故走
 * **通道级**规则：同时命中「DB 客户端标记」与「写关键词」才判为写通道。通道判定 ≠ 内容判定，
 * 对混淆（f-string 动态拼接、编码后拼接）无抵抗力——已声明的上限。
 *
 * 姿态：只读不拦（`SELECT` 类字面量判定结果恒为 cause=null）；写形字面量按 sql-scope 四类 cause 处理。
 */
import { transformSync } from 'esbuild';
import { js } from './parsers.ts';
import { scanDbCommand, type SqlPayload } from './psql-invocation.ts';
import { decideSqlScope, looksLikeDdl, type ScopeCause, type ScopeHit } from './sql-scope.ts';

export interface CodeLiteral {
  kind: 'string' | 'template';
  text: string;
  /** 模板含插值：cooked 段拼合，内容不完整 → 按固定前缀细分判定 */
  dynamic: boolean;
  /** 模板的固定段（quasis，按插值切开）：segment[0] = 首个插值前的固定前缀 */
  segments: string[];
  /** 所在调用位的 callee 名（如 `tool.bash` / `Bun.sql` / `query`）；null = 不在调用实参位 */
  callee: string | null;
}

export interface InlineJudge {
  /** JS/TS 面是否解析成功（false = 无法证明，调用方按自身姿态处置） */
  parsed: boolean;
  literalCount: number;
  payloads: SqlPayload[];
  opaque: string[];
  hits: (ScopeHit & { source: string })[];
  /** 命中原因（Python 面为通道级判定，报告为 'channel'） */
  cause: ScopeCause | 'channel';
}

/** Python 通道级规则的「DB 客户端标记」：库连接或 psql 子进程 */
const PY_DB_MARKERS = ['psycopg', 'asyncpg', 'sqlalchemy', 'pg8000', 'psql', 'copy_from', 'copy_expert'];

/** Python 通道级规则的「写关键词」：命中任一即视为写通道。
 *  刻意**不含** `update ` / `copy `——`SELECT … FOR UPDATE` 等只读形态会误命中（只读不拦）；
 *  UPDATE / COPY 的 Python 面覆盖上限已声明在模块头注。 */
const PY_WRITE_KEYWORDS = [
  'create table',
  'create index',
  'create schema',
  'alter table',
  'drop table',
  'drop index',
  'truncate',
  'insert into',
  'delete from',
];

/**
 * 执行 API 名单 —— 提交面「执行位」判据（唯一实现）。
 * 只有名单内 callee 的实参/标签模板才可能承载**待执行** SQL；名单外一律视为语料
 * （`describe` / `it` / `expect` / `toContain` / 测试辅助函数）。
 * 实测依据：`scripts/{check,lib,pre}` 探针显示「在调用实参位」不等价于执行位——33 处调用实参位里
 * 绝大多数是测试 DSL（见 change `close-db-guard-channel-gaps`）。
 * 已知代价（声明）：别名导入（`import { query as q }`）与包装调用会漏。
 */
export const EXEC_API: Record<string, true> = {
  // 宿主工具桥
  'tool.bash': true,
  'tool.eval': true,
  // Bun 运行时
  'Bun.$': true,
  '$': true,
  'Bun.sql': true,
  'Bun.spawn': true,
  'Bun.spawnSync': true,
  // Node child_process / 进程
  'exec': true,
  'execSync': true,
  'spawn': true,
  'spawnSync': true,
  'child_process.exec': true,
  'child_process.execSync': true,
  'child_process.spawn': true,
  'child_process.spawnSync': true,
  // Python 子进程
  'subprocess.run': true,
  'subprocess.Popen': true,
  'subprocess.call': true,
  'os.system': true,
  'os.popen': true,
  // DB 客户端查询位
  'client.query': true,
  'pool.query': true,
  'db.query': true,
  'sql.query': true,
  'pg.query': true,
  'knex.raw': true,
};

/** 从 JS/TS 源码提取字符串与模板字面量（meriyah；TS/TSX 先经 esbuild 去类型） */
export function extractJsLiterals(source: string): { parsed: boolean; literals: CodeLiteral[] } {
  const parse = (src: string): unknown => {
    try {
      return js.parseModule(src, { jsx: true });
    } catch {
      return js.parse(src, { jsx: true });
    }
  };
  let ast: unknown;
  try {
    ast = parse(source);
  } catch {
    try {
      // TS/TSX：去类型后重试（与 scripts/parser-utils.mjs 同法）
      ast = parse(transformSync(source, { loader: 'tsx', jsx: 'preserve' }).code);
    } catch {
      return { parsed: false, literals: [] };
    }
  }
  const literals: CodeLiteral[] = [];
  const visit = (node: unknown, callee: string | null = null): void => {
    if (!node || typeof node !== 'object') return;
    const n = node as {
      type?: string;
      value?: unknown;
      quasis?: unknown[];
      expressions?: unknown[];
      callee?: unknown;
      tag?: unknown;
    };
    if (n.type === 'Literal' && typeof n.value === 'string') {
      literals.push({ kind: 'string', text: n.value, dynamic: false, segments: [n.value], callee });
    } else if (n.type === 'TemplateLiteral') {
      const quasis = (n.quasis ?? []) as { value?: { cooked?: string } }[];
      const segments = quasis.map((q) => q.value?.cooked ?? '');
      literals.push({
        kind: 'template',
        text: segments.join(''),
        dynamic: (n.expressions ?? []).length > 0,
        segments,
        callee,
      });
    }
    // 调用 / 标签模板的实参位携带 callee 名：供「执行位」判定（字面量是否被喂给 DB / exec API）
    const inner =
      n.type === 'CallExpression' ? calleeName(n.callee) : n.type === 'TaggedTemplateExpression' ? calleeName(n.tag) : callee;
    for (const [k, v] of Object.entries(n)) {
      if (k === 'loc' || k === 'range' || k === 'callee' || k === 'tag') continue;
      if (Array.isArray(v)) for (const c of v) visit(c, inner);
      else if (v && typeof v === 'object') visit(v, inner);
    }
  };
  visit(ast);
  return { parsed: true, literals };
}

/** 取调用位名称：`Bun.sql` / `a.b.c` → 点分串；非标识符链 → null */
function calleeName(node: unknown): string | null {
  if (!node || typeof node !== 'object') return null;
  const n = node as { type?: string; name?: string; property?: unknown; object?: unknown };
  if (n.type === 'Identifier' && typeof n.name === 'string') return n.name;
  if (n.type === 'MemberExpression') {
    const obj = calleeName(n.object);
    const prop = calleeName(n.property);
    return obj && prop ? `${obj}.${prop}` : (prop ?? obj);
  }
  return null;
}

/**
 * 判定一段「代码内联」文本（eval 单元格 / 源文件片段）。
 * 只读内容恒返回 cause=null 且 hits 为空。
 */
export async function judgeInlineCode(
  source: string,
  opts: { lang: 'js' | 'py'; cwd: string; label?: string; execPositionOnly?: boolean },
): Promise<InlineJudge> {
  const result: InlineJudge = { parsed: true, literalCount: 0, payloads: [], opaque: [], hits: [], cause: null };

  if (opts.lang === 'py') {
    result.parsed = false; // 无解析器：通道级判定，非内容级
    const lower = source.toLowerCase();
    if (PY_DB_MARKERS.some((m) => lower.includes(m)) && PY_WRITE_KEYWORDS.some((k) => lower.includes(k))) {
      result.cause = 'channel';
    }
    return result;
  }

  const { parsed, literals } = extractJsLiterals(source);
  result.parsed = parsed;
  result.literalCount = literals.length;
  if (!parsed) return result;

  const rank: Record<string, number> = { channel: 5, anchor: 4, isahl: 3, unknown: 2, unresolvable: 1 };
  let best = 0;
  let cause: ScopeCause | 'channel' | null = null;
  const bump = (c: ScopeCause, source: string, text: string): void => {
    result.payloads.push({ text, source });
    if ((rank[c] ?? 0) > best) {
      best = rank[c] ?? 0;
      cause = c;
    }
  };

  for (const lit of literals) {
    const text = lit.text;
    if (!text.trim()) continue;
    // 执行位模式（提交面）：只有执行 API 名单内 callee 的实参 / 标签模板才可能承载待执行 SQL；
    // 名单外（数组语料、describe/it/expect 等测试 DSL）一律视为语料 —— 机器可判，无需人工标注
    if (opts.execPositionOnly && !(lit.callee !== null && EXEC_API[lit.callee] === true)) continue;
    const label = opts.label ? `${opts.label}:${lit.kind}` : lit.kind;

    // ① 命令形态字面量：**仅当 psql 处于命令行位真调用**时才套用 shell 语义。
    //    散文字面量里的 psql 提及（如失败提示 `psql（交互式，未提供 -c…）`）不构成调用，
    //    否则会把说明文本判成「不可读载荷」（实测误报主因，见 change proposal）。
    if (text.includes('psql') || text.includes('schema-info')) {
      const scan = scanDbCommand(text, opts.cwd);
      if (scan.psqlInvoked) {
        for (const p of scan.payloads) {
          const d = await decideSqlScope(p.text);
          for (const h of d.hits) result.hits.push({ ...h, source: `${label}(${p.source})` });
          if (d.cause) bump(d.cause, `${label}(${p.source})`, text);
        }
        // `opaque`（交互式 psql / 文件不可读）只在**动态**字面量上计入：
        // 静态散文串（如失败提示 `'psql（交互式，未提供 -c…）'`）不是不可读载荷（实测误报主因）
        if (lit.dynamic) for (const o of scan.opaque) result.opaque.push(`${label} → ${o}`);
      }
    }

    // ② SQL 形态字面量：只认**真解析出的命中**。
    //    正则兜底命中（`sql-scope.lexicalHits`，target 恒含占位 `?`）不参与字面量面——
    //    它命中的是关键字/格式串词表（`'CREATE TABLE'` 类），不是待执行语句（实测误报主因）。
    const d = await decideSqlScope(text);
    for (const h of d.hits) {
      // 正则兜底命中里「占位目标 + 未限定 schema」的条目不计入：那是关键字/格式串词表的签名，
      // 不是待执行语句（实测误报主因之一）；兜底命中里**已限定** isahl / 锚点的条目仍计入（目标明确）
      if (h.target.includes('?') && h.schemaClass === 'unknown') continue;
      result.hits.push({ ...h, source: label });
    }
    if (d.cause && result.hits.some((h) => h.source === label)) bump(d.cause, label, text);

    // ③ 动态模板（含插值）：按**固定段**逐段细分，不用「动态 + DDL 词」这一粗档
    //    ① 某固定段已限定目标（`ALTER TABLE isahl.x ADD COLUMN ${col} …`）→ 精确命中（无需猜）
    //    ② 只有结构动作头、目标落在插值处（`ALTER TABLE ${t} …`）→ 与 bash 面「目标未限定」同语义
    //    ③ 全部固定段都无结构动作头（生成器 / 格式串）→ 放行（原粗档的误报面）
    if (!d.cause && lit.dynamic) {
      let headOnly = false;
      for (const seg of lit.segments) {
        const s = seg.trim();
        if (!s) continue;
        const ps = await decideSqlScope(s);
        const q = ps.hits.find((h) => h.schemaClass === 'isahl' || h.schemaClass === 'anchor');
        if (q) {
          result.hits.push({ ...q, source: `${label}(固定段已限定目标)` });
          bump(q.schemaClass === 'anchor' ? 'anchor' : 'isahl', label, s);
          break;
        }
        if (looksLikeDdl(s)) headOnly = true;
      }
      if (!cause && headOnly) bump('unknown', label, text.trim().slice(0, 80));
    }
  }

  if (!cause && result.opaque.length > 0) cause = 'unresolvable';
  result.cause = cause;
  return result;
}
