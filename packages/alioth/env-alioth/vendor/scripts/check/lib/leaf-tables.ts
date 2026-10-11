#!/usr/bin/env bun
/**
 * leaf-tables.ts — 叶表判决的**单一实现**（唯一继承载体 + 唯一豁免解释 + 唯一基线容忍策略）
 *
 * 规约:
 *   - docs/specs/ENVIRONMENT_SPEC.md §11.5（继承体系基础设施：继承链遍历 MUST 优先复用既有设施）
 *   - openspec/specs/reuse-first-spec：命中 `isahl_meta.gf_query_leafs` MUST 直接复用，
 *     MUST NOT 手写 `pg_inherits` 递归 CTE
 *   - docs/specs/ALIOTH_ONTOLOGY_SPEC.md §8.5（INSERT 必须落叶表；父表豁免唯一来源 =
 *     模型侧 `row_layer='carry'` 声明）
 *   - openspec/specs/spec-enforcement-substrate R1（带基线/豁免判据的门禁 MUST 提供含负例的
 *     `--self-test`）
 *
 * 消费方（MUST 同源，MUST NOT 各自内省继承结构或各自解释 row_layer/基线）:
 *   - scripts/check/check-leaf-insert.ts        （源文件面：.rs/.sql/.sh 全仓 walk / 暂存区）
 *   - scripts/check/check-seed-leaf-targets.ts  （种子面：种子载体 + libpg_query）
 *
 * 判决优先级（唯一）:
 *   row_layer='abstract' ⇒ 违规（无论目标是否叶表——abstract = 不直接实例化，豁免不放宽）
 *   row_layer='carry'    ⇒ 合规（实体/总表/字典条目承载，直接行 = 设计内产物）
 *   无子表               ⇒ 合规（叶表铁律本义）
 *   有子表且未声明        ⇒ 违规（待模型中心裁决 abstract|carry）
 *
 * 降级/容忍:
 *   - `leafTables()` 返回 null（DB 不可达 / isahl_meta 缺失）⇒ 各消费方打印明确 warn + exit 0
 *   - 继承面可用而 row_layer 声明面为空 ⇒ 按"无豁免"严格执行（单调收紧，不回退仓库侧名单）
 *   - 存量归线：共享登记 `scripts/check/.leaf-insert-baseline.txt`，读取唯一入口 =
 *     ./baseline-store 的 readBaselineKeys（含 INV-2 归属过滤）；键 = `<仓相对路径>:<表>[:<kind>]`
 */
import { spawnSync } from 'node:child_process';
import { readBaselineKeys } from './baseline-store';

export const DEFAULT_DATABASE_URL = 'postgres://isahl@localhost:5432/aliothstudio_dev';

/** 行层级声明值（模型侧契约：abstract=不直接实例化 / carry=可直接持行）。 */
export type RowLayer = 'abstract' | 'carry';
/** 模型侧豁免查询三态：abstract / carry / 未声明（含非法值，按未声明处理）。 */
export type ExemptionState = RowLayer | 'undeclared';

export interface LeafFacts {
  /** 继承父表集合（有子表 ⇒ 非叶）——isahl_meta.devv_inherits_view */
  nonLeaf: Set<string>;
  /** row_layer 声明：表 → 层（模型中心裁决） */
  rowLayer: Map<string, RowLayer>;
  /** 声明值 ∉ {abstract,carry} 的条目（显式 warn；按未声明处理，不参与豁免） */
  unknownLayer: string[];
}

export interface LeafDbOpts {
  databaseUrl?: string;
  timeoutMs?: number;
}

/**
 * 继承事实 + row_layer 声明的**唯一查询**（单次 psql 取回，避免逐表 spawn）。
 * 输出行：`P|<表>` = 继承父表；`L|<表>|<层>` = row_layer 声明（-F '|'；表名不含 '|'）。
 *
 * 载体唯一 = `isahl_meta.devv_inherits_view`（ENVIRONMENT_SPEC §11.5 设施表；子表侧
 * `nspname='isahl' AND relkind='r'`、父表侧不限定为其钉定口径）——MUST NOT 直读 pg_inherits。
 */
const LEAF_FACTS_SQL = `SELECT 'P', parent::text FROM isahl_meta.devv_inherits_view WHERE parent IS NOT NULL
 UNION
 SELECT 'L', table_name::text || '|' || (config->>'row_layer')
   FROM isahl_meta.meta_collections WHERE config ? 'row_layer'`;

/** lifecycle 子树根（坐标写入契约 §6.12 的判定对象面）。 */
export const LIFECYCLE_ROOT = 'zc_id_lifecycle';

/** 子树叶表查询（唯一形态）：复用设施 `isahl_meta.gf_query_leafs`，MUST NOT 自建递归 CTE。 */
function leafsSql(root: string): string {
  return `SELECT unnest(isahl_meta.gf_query_leafs('${root.replace(/'/g, "''")}'::name))`;
}

function psqlLines(sql: string, opts: LeafDbOpts): string[] | null {
  const r = spawnSync('psql', [opts.databaseUrl ?? process.env.DATABASE_URL ?? DEFAULT_DATABASE_URL, '-tA', '-F', '|', '-c', sql], {
    encoding: 'utf-8',
    timeout: opts.timeoutMs ?? 30000,
  });
  if (r.status !== 0) return null;
  return (r.stdout ?? '').split('\n').map((l) => l.replace(/\r$/, '')).filter((l) => l !== '');
}

/** 解析 `leafTables()` 的原始输出（纯函数，自测覆盖）。 */
export function parseLeafFactsRows(lines: readonly string[]): LeafFacts {
  const facts: LeafFacts = { nonLeaf: new Set(), rowLayer: new Map(), unknownLayer: [] };
  for (const line of lines) {
    const kind = line.slice(0, 1);
    const rest = line.slice(2);
    if (kind === 'P') {
      if (rest) facts.nonLeaf.add(rest);
      continue;
    }
    if (kind !== 'L' || !rest) continue;
    const sep = rest.indexOf('|');
    const t = sep >= 0 ? rest.slice(0, sep) : rest;
    const v = sep >= 0 ? rest.slice(sep + 1) : '';
    if (!t) continue;
    if (v === 'abstract' || v === 'carry') facts.rowLayer.set(t, v);
    else facts.unknownLayer.push(`${t}=${v || 'NULL'}`);
  }
  return facts;
}

/**
 * 模型侧判定面（只读 DB）——父/叶结构 + row_layer 声明，单次取回。
 * DB 不可用 / isahl_meta 面缺失 ⇒ 返回 null（消费方打印明确 warn 后降级 exit 0）。
 */
export function leafTables(opts: LeafDbOpts = {}): LeafFacts | null {
  const lines = psqlLines(LEAF_FACTS_SQL, opts);
  if (lines === null) return null;
  return parseLeafFactsRows(lines);
}

/** 子树**叶表**集合（设施复用：`gf_query_leafs`）；DB 不可用 ⇒ null（消费方降级）。 */
export function leafDescendantsOf(root: string = LIFECYCLE_ROOT, opts: LeafDbOpts = {}): Set<string> | null {
  const lines = psqlLines(leafsSql(root), opts);
  if (lines === null) return null;
  return new Set(lines);
}

/** row_layer 豁免的**唯一解释**（含非法值按未声明）。 */
export function rowLayerExemption(facts: LeafFacts, table: string): ExemptionState {
  return facts.rowLayer.get(table) ?? 'undeclared';
}

export type LeafVerdict =
  | { kind: 'ok'; reason: string }
  | { kind: 'exempt'; layer: 'carry' }
  | { kind: 'violation'; layer: 'abstract' | 'undeclared'; reason: string };

/** 叶表规则的**唯一判决**（消费方 MUST 只经此函数给出叶表结论）。 */
export function leafVerdict(facts: LeafFacts, table: string): LeafVerdict {
  const layer = rowLayerExemption(facts, table);
  if (layer === 'abstract') {
    return {
      kind: 'violation',
      layer,
      reason: 'row_layer=abstract——不直接实例化，禁 INSERT（abstract 优先于叶表判定）',
    };
  }
  if (layer === 'carry') return { kind: 'exempt', layer };
  if (!facts.nonLeaf.has(table)) return { kind: 'ok', reason: '叶表（无子表）' };
  return {
    kind: 'violation',
    layer: 'undeclared',
    reason: '继承父表未声明 row_layer——需模型中心裁决 abstract|carry',
  };
}

/** 非法 row_layer 声明值的统一告警（消费方调用；按未声明处理）。 */
export function printUnknownRowLayers(facts: LeafFacts, label: string): void {
  for (const u of facts.unknownLayer) {
    console.warn(`  ⚠️ ${label}: row_layer 声明值非法（应为 abstract|carry），按未声明处理: ${u}`);
  }
}

// ── 共享基线（唯一容忍策略）────────────────────────────────

/** 叶表归线登记（两门禁共享；只减不增）。 */
export const LEAF_BASELINE_REL = 'scripts/check/.leaf-insert-baseline.txt';

export type LeafViolationKind = 'leaf' | 'class' | 'coords';

/** 归线键 = `<仓相对路径>:<表>`；类/坐标契约违规加 `:<kind>` 后缀（与叶表违规互不干扰）。 */
export function leafBaselineKey(rel: string, table: string, kind: LeafViolationKind = 'leaf'): string {
  return kind === 'leaf' ? `${rel}:${table}` : `${rel}:${table}:${kind}`;
}

/** 读取共享归线（唯一读取入口 = baseline-store::readBaselineKeys，含归属过滤）。 */
export function loadLeafBaseline(root: string): Set<string> {
  return readBaselineKeys(root, LEAF_BASELINE_REL);
}

// ── 自测（纯逻辑：不读 DB / 不读文件系统）──────────────────

interface SelfTestCase {
  name: string;
  run: () => boolean;
  detail: () => string;
}

function factsOf(nonLeaf: string[], layer: Record<string, string> = {}): LeafFacts {
  const f: LeafFacts = { nonLeaf: new Set(nonLeaf), rowLayer: new Map(), unknownLayer: [] };
  for (const [t, v] of Object.entries(layer)) {
    if (v === 'abstract' || v === 'carry') f.rowLayer.set(t, v);
    else f.unknownLayer.push(`${t}=${v}`);
  }
  return f;
}

function expectVerdict(facts: LeafFacts, table: string, want: string): boolean {
  const v = leafVerdict(facts, table);
  const got = v.kind === 'violation' ? `violation:${v.layer}` : v.kind === 'exempt' ? 'exempt:carry' : 'ok';
  return got === want;
}

function selfTestCases(): SelfTestCase[] {
  return [
    {
      name: '负例：未声明非叶 ⇒ 违规（无隐式豁免）',
      run: () => expectVerdict(factsOf(['parent_t']), 'parent_t', 'violation:undeclared'),
      detail: () => '"parent_t" 应为 violation:undeclared',
    },
    {
      name: '负例：abstract 非叶 ⇒ 违规',
      run: () => expectVerdict(factsOf(['parent_t'], { parent_t: 'abstract' }), 'parent_t', 'violation:abstract'),
      detail: () => '"parent_t" 应为 violation:abstract',
    },
    {
      name: '负例：abstract 叶表 ⇒ 违规（不得因叶表放行）',
      run: () => expectVerdict(factsOf([], { leaf_abstract: 'abstract' }), 'leaf_abstract', 'violation:abstract'),
      detail: () => '"leaf_abstract" 应为 violation:abstract',
    },
    {
      name: '正例：carry 非叶 ⇒ 豁免',
      run: () => expectVerdict(factsOf(['carry_t'], { carry_t: 'carry' }), 'carry_t', 'exempt:carry'),
      detail: () => '"carry_t" 应为 exempt:carry',
    },
    {
      name: '正例：叶表（未声明）⇒ 合规',
      run: () => expectVerdict(factsOf(['other_parent']), 'leaf_t', 'ok'),
      detail: () => '"leaf_t" 应为 ok',
    },
    {
      name: '负例：非法层值 ⇒ 按未声明（非叶即违规）',
      run: () =>
        expectVerdict(factsOf(['x_t'], { x_t: 'bogus' }), 'x_t', 'violation:undeclared') &&
        factsOf(['x_t'], { x_t: 'bogus' }).unknownLayer.length === 1,
      detail: () => '"x_t=bogus" 应为 violation:undeclared 且计入 unknownLayer',
    },
    {
      name: '解析：P/L 行 → nonLeaf/rowLayer/unknownLayer',
      run: () => {
        const f = parseLeafFactsRows(['P|p1', 'P|p1', 'L|t1|carry', 'L|t2|nope', '', 'L|t3|abstract']);
        return (
          f.nonLeaf.size === 1 &&
          f.nonLeaf.has('p1') &&
          f.rowLayer.get('t1') === 'carry' &&
          f.rowLayer.get('t3') === 'abstract' &&
          f.unknownLayer.join(',') === 't2=nope'
        );
      },
      detail: () => '期望 nonLeaf={p1}、rowLayer={t1:carry,t3:abstract}、unknownLayer=[t2=nope]',
    },
    {
      name: '载体唯一：facts 查询读 devv_inherits_view 且不含 pg_inherits',
      run: () => LEAF_FACTS_SQL.includes('isahl_meta.devv_inherits_view') && !LEAF_FACTS_SQL.includes('pg_inherits'),
      detail: () => 'LEAF_FACTS_SQL MUST 只读 devv_inherits_view',
    },
    {
      name: '设施复用：子树叶表经 gf_query_leafs 且不含 pg_inherits',
      run: () => leafsSql(LIFECYCLE_ROOT).includes('isahl_meta.gf_query_leafs') && !leafsSql(LIFECYCLE_ROOT).includes('pg_inherits'),
      detail: () => 'leafDescendantsOf MUST 复用 gf_query_leafs',
    },
    {
      name: '基线键形：leaf / class / coords',
      run: () =>
        leafBaselineKey('a/b.sql', 't') === 'a/b.sql:t' &&
        leafBaselineKey('a/b.sql', 't', 'class') === 'a/b.sql:t:class' &&
        leafBaselineKey('a/b.sql', 't', 'coords') === 'a/b.sql:t:coords',
      detail: () => '键 MUST 为 <rel>:<table>[:<kind>]',
    },
    {
      name: '基线单源：共享登记 scripts/check/.leaf-insert-baseline.txt',
      run: () => LEAF_BASELINE_REL === 'scripts/check/.leaf-insert-baseline.txt',
      detail: () => `LEAF_BASELINE_REL=${LEAF_BASELINE_REL}`,
    },
  ];
}

/** `--self-test` 唯一执行器（含负例；rc=0=全过）。 */
export function runLeafSelfTest(label = 'leaf-tables'): number {
  let failed = 0;
  const cases = selfTestCases();
  for (const c of cases) {
    let ok = false;
    try {
      ok = c.run();
    } catch {
      ok = false;
    }
    if (!ok) failed++;
    console.log(`${ok ? '✔' : '✘'} self-test ${c.name}${ok ? '' : ` —— ${c.detail()}`}`);
  }
  const rc = failed === 0 ? 0 : 1;
  console.log(
    rc === 0
      ? `✅ ${label} --self-test 通过（${cases.length} 例，含负例：豁免不放宽）`
      : `❌ ${label} --self-test 失败 ${failed}/${cases.length}`,
  );
  return rc;
}
