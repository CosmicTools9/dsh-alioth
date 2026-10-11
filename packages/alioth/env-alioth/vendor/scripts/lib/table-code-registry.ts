/**
 * table-code-registry.ts — 表码登记采集/比对（门禁 `check-table-code-registry` 与修复脚本共用）
 *
 * 背景：`gen_next_uid(table_code)` 的 table_code 单一真相源 = 平台库
 * `isahl_meta.meta_collections.config->'table_code'`；各 namespace 的 DDL 把表码物化为
 * 字面量（`gen_next_uid((476)::bigint)`）。两者不一致即为「漂移」。
 *
 * 判据方向（2026-09-22 实测确定）：**实际分配为准**——表码被数据固化在 id 高位
 * （`id >> 48`，实测 drift 表的 id 高位 == DDL 字面量 ≠ 登记值）⇒ 回填登记，绝不反向改 DDL
 * （反向改会让现存 id 的高位指向别的表码空间）。
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { pgDefault, toml } from './parsers';
import { portsTableRoot } from './ns-mounts';
import { UNRESOLVED_DSN, repoRoleOf, resolveRepoDbUrl } from './repo-role';

/** ns → 表 → 表码（null = 默认值形态不可解析） */
export type NsFacts = Record<string, Record<string, number | null>>;
export type Registry = Record<string, number>;

export interface Finding {
  ns: string;
  table: string;
  /** `layer-only` = 该层独有（仅平台层用；属预期差异，MUST 入基线而非报「不一致」） */
  kind: 'mismatch' | 'unregistered' | 'unparsable' | 'layer-only';
  detail: string;
}

/** psql → 制表符分列的行集；连接/查询失败返回 null（调用方据此 skip，不猜） */
export function psqlRows(url: string, sql: string): string[][] | null {
  const r = spawnSync('psql', [url, '-At', '-F', '\t', '-c', sql], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    timeout: 60_000,
  });
  if (r.status !== 0 || r.error) return null;
  return (r.stdout ?? '')
    .split('\n')
    .filter((l) => l.length > 0)
    .map((l) => l.split('\t'));
}

/** 逐字符取首个 `=` 切分 KEY=VALUE（本仓既有做法，见 check-sso-mode-conf.ts：无标准 .env 解析器） */
export function readEnvValue(path: string, key: string): string | null {
  if (!existsSync(path)) return null;
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    const s = line.trim();
    if (s.length === 0 || s[0] === '#') continue;
    let eq = -1;
    for (let i = 0; i < s.length; i++) {
      if (s[i] === '=') {
        eq = i;
        break;
      }
    }
    if (eq <= 0) continue;
    if (s.slice(0, eq).trim() !== key) continue;
    let v = s.slice(eq + 1).trim();
    if (v.length >= 2 && ((v[0] === '"' && v.endsWith('"')) || (v[0] === "'" && v.endsWith("'")))) {
      v = v.slice(1, -1);
    }
    return v;
  }
  return null;
}

/** 空白分隔表首列（`Deploy/ports.conf` 的既有读法；`#` 注释/空行忽略） */
export function nsFromPortsConf(root: string): string[] {
  return portSlotsFromPortsConf(root).map((r) => r.ns);
}

/** `Deploy/ports.conf` 的一行槽位（ns + FE/BE/SSO 三列）。 */
export interface PortSlot {
  ns: string;
  fe: number | null;
  be: number | null;
  sso: number | null;
}

/** `Deploy/ports.conf` 原始行（首列 + 原始列 + 行号）——本表解析的唯一实现，
 *  `portSlotsFromPortsConf` 的类型化视图与校验器（check-runtime-conf 报错定位）共用；
 *  行内 `#` 不截断（与 bash 正本 ns-ports.sh 同语义：仅行首 `#` 为注释）。 */
export interface PortsConfRow {
  ns: string;
  cols: string[];
  line: number;
}

export function portsConfRows(root: string, confPath?: string): PortsConfRow[] {
  const tableRoot = portsTableRoot(root);
  const path = confPath ?? process.env.NS_PORTS_CONF ?? (tableRoot === null ? '' : join(tableRoot, 'Deploy', 'ports.conf'));
  if (path === '' || !existsSync(path)) return [];
  const out: PortsConfRow[] = [];
  for (const [i, raw] of readFileSync(path, 'utf8').split('\n').entries()) {
    const s = raw.trim();
    if (s.length === 0 || s[0] === '#') continue;
    const cols = s.split(/\s+/);
    const ns = cols[0] ?? '';
    if (ns.length === 0) continue;
    out.push({ ns, cols, line: i + 1 });
  }
  return out;
}

/**
 * `Deploy/ports.conf` 全表读取（**唯一实现**：`nsFromPortsConf` 与 `portsConfRows` 亦委托到此层）。
 *
 * 列序 FE BE SSO（空白分隔；`#` 开头/空行忽略）；端口列非纯数字 ⇒ 该列为 `null`
 * （读取方自行决定 fail-loud）。`NS_PORTS_CONF` 可覆写表路径（测试/隔离）；显式
 * `confPath` 参数最高优先（校验器点名审计/自测夹具用）。
 */
export function portSlotsFromPortsConf(root: string, confPath?: string): PortSlot[] {
  const num = (s: string | undefined): number | null => {
    if (s === undefined) return null;
    return /^[0-9]+$/.test(s) ? Number(s) : null;
  };
  return portsConfRows(root, confPath).map((r) => ({
    ns: r.ns,
    fe: num(r.cols[1]),
    be: num(r.cols[2]),
    sso: num(r.cols[3]),
  }));
}

/**
 * 平台/本 ns 库 DSN：显式 `PLATFORM_DATABASE_URL`/`DATABASE_URL` 优先，否则取**仓角色**通道
 * （ns 仓 ⇒ 本 ns env；平台仓 ⇒ 组件 `.mise.toml` 的 `DATABASE_URL.default`，唯一实现 =
 * `scripts/lib/db-url-resolve.sh`）。ns 仓解析不到本 ns env ⇒ 哨兵 DSN（快速失败，MUST NOT 落到
 * 本机默认库）。
 */
export function platformUrl(root: string): string {
  const fromEnv = process.env.PLATFORM_DATABASE_URL ?? process.env.DATABASE_URL;
  if (fromEnv) return fromEnv;
  const resolved = resolveRepoDbUrl(root);
  if (resolved.url !== null) return resolved.url;
  if (repoRoleOf(root) === 'ns') return UNRESOLVED_DSN;
  const mise = join(root, 'Meta', 'backend', '.mise.toml');
  const doc = existsSync(mise) ? toml.parse(readFileSync(mise, 'utf8')) : {};
  const env = (doc as Record<string, Record<string, unknown>>).env ?? {};
  const entry = env.DATABASE_URL as { default?: string } | string | undefined;
  if (typeof entry === 'string') return entry;
  return entry?.default ?? 'postgres://localhost:5432/aliothstudio_dev';
}

/** 读平台登记：table_name → table_code（仅含带键者） */
export function collectRegistry(root: string): { registry: Registry; rows: number } | null {
  const rows = psqlRows(
    platformUrl(root),
    "SELECT table_name, config->>'table_code' FROM isahl_meta.meta_collections WHERE config ? 'table_code' ORDER BY 1",
  );
  if (rows === null) return null;
  const registry: Registry = {};
  for (const [table, code] of rows) {
    if (!table || !code) continue;
    const n = Number(code);
    if (Number.isSafeInteger(n)) registry[table] = n;
  }
  return { registry, rows: rows.length };
}

/** 平台库中登记是否含该表的行（缺行 ≠ 缺键） */
export function registryRowExists(root: string, table: string): boolean {
  const rows = psqlRows(
    platformUrl(root),
    `SELECT 1 FROM isahl_meta.meta_collections WHERE table_name = '${table.replace(/'/g, "''")}' LIMIT 1`,
  );
  return rows !== null && rows.length > 0;
}

/** 逐 ns 采集 DDL 面事实（漏 ns `.env` / 库不可达 ⇒ 记入 skipped） */
export function collectNsFacts(
  root: string,
  nsList: string[],
): { facts: NsFacts; introspected: string[]; skipped: string[] } {
  const facts: NsFacts = {};
  const introspected: string[] = [];
  const skipped: string[] = [];
  for (const ns of nsList) {
    const url = readEnvValue(join(root, 'Deploy', ns, '.env'), 'DATABASE_URL');
    if (!url) {
      skipped.push(`${ns}(无 .env)`);
      continue;
    }
    const rows = psqlRows(url, UID_LITERAL_SQL);
    if (rows === null) {
      skipped.push(`${ns}(库不可达)`);
      continue;
    }
    const perNs: Record<string, number | null> = {};
    for (const [table, def] of rows) if (table) perNs[table] = pgDefault.genNextUidCode(def ?? '');
    facts[ns] = perNs;
    introspected.push(ns);
  }
  return { facts, introspected, skipped };
}

/** 平台层（`extend-table-code-assertions`）：层名 → 库名 → env 覆写键。canonical = dev（不经此表）。 */
export interface PlatformLayer { name: string; db: string; env: string }
export const PLATFORM_LAYERS: readonly PlatformLayer[] = [
  { name: 'test', db: 'aliothstudio_test', env: 'ALIOTH_TEST_DATABASE_URL' },
  { name: 'pre', db: 'aliothstudio_pre', env: 'ALIOTH_PRE_DATABASE_URL' },
  { name: 'prod', db: 'aliothstudio', env: 'ALIOTH_PROD_DATABASE_URL' },
] as const;

/** 平台的 **id 默认值字面量** introspection（与 ns 面同一条 SQL，单一实现）。 */
const UID_LITERAL_SQL =
  "SELECT table_name, column_default FROM information_schema.columns WHERE table_schema='isahl' AND column_name='id' AND column_default LIKE '%gen_next_uid%' ORDER BY 1";

/**
 * 平台层 DSN：`ALIOTH_{TEST,PRE,PROD}_DATABASE_URL` 优先，否则由 **canonical DSN 换库名**派生
 * （4 层隔离 `aliothstudio(_test|_pre)` 为 `ENVIRONMENT_SPEC` 的既定约定）。canonical 未解析 ⇒ null。
 */
export function platformLayerUrl(root: string, layer: PlatformLayer): string | null {
  const fromEnv = process.env[layer.env];
  if (fromEnv !== undefined && fromEnv !== '') return fromEnv;
  const base = platformUrl(root);
  if (base === UNRESOLVED_DSN) return null;
  const m = /^([^/]*\/\/[^/]*\/)([^/?#]+)(.*)$/.exec(base);
  if (!m) return null;
  return `${m[1]}${layer.db}${m[3]}`;
}

/**
 * 逐平台层采集 DDL 面事实（键 = 层名）。层不可达 / 无 `isahl` 面 ⇒ 记入 skipped（调用方打印，
 * MUST NOT 静默通过）。ns 库无 `isahl_meta` 是设计，不影响本采集（只读 `information_schema`）。
 */
export function collectPlatformLayerFacts(root: string): {
  facts: NsFacts;
  introspected: string[];
  skipped: string[];
} {
  const facts: NsFacts = {};
  const introspected: string[] = [];
  const skipped: string[] = [];
  for (const layer of PLATFORM_LAYERS) {
    const url = platformLayerUrl(root, layer);
    if (url === null) {
      skipped.push(`${layer.name}(无 DSN)`);
      continue;
    }
    const rows = psqlRows(url, UID_LITERAL_SQL);
    if (rows === null) {
      skipped.push(`${layer.name}(库不可达)`);
      continue;
    }
    const per: Record<string, number | null> = {};
    for (const [table, def] of rows) if (table) per[table] = pgDefault.genNextUidCode(def ?? '');
    facts[layer.name] = per;
    introspected.push(layer.name);
  }
  return { facts, introspected, skipped };
}

/** 纯判据：表码位宽预算 —— MUST ∈ [0, 2^15)（越界 = 64 位域静默溢出，确定性缺陷）。 */
export function budgetViolations(entries: Array<{ scope: string; table: string; code: number | null }>): Finding[] {
  const out: Finding[] = [];
  for (const e of entries) {
    if (e.code === null) {
      out.push({ ns: e.scope, table: e.table, kind: 'unparsable', detail: 'id 默认值非 gen_next_uid(<字面量>) 形态' });
      continue;
    }
    if (!Number.isInteger(e.code) || e.code < 0 || e.code >= 32768) {
      out.push({ ns: e.scope, table: e.table, kind: 'mismatch', detail: `表码 ${e.code} 越界（MUST ∈ [0, 32767]）` });
    }
  }
  return out.sort((a, b) => (a.ns + a.table).localeCompare(b.ns + b.table));
}

/** 注册表自身的位宽预算判据（登记面）。 */
export function registryBudgetViolations(registry: Registry): Finding[] {
  return budgetViolations(Object.entries(registry).map(([table, code]) => ({ scope: 'registry', table, code })));
}

/** 纯比对（不触碰 DB；`--self-test` 直接以此为断言对象）。
 *  `opts.unregisteredAsLayerOnly`：平台层面用——「登记中无此表」= 层独有（预期差异），
 *  归 `layer-only` 而非 `unregistered`（语义不同：前者是层级滞后，后者是登记缺失缺陷）。 */
export function diffTableCodes(
  facts: NsFacts,
  registry: Registry,
  opts: { unregisteredAsLayerOnly?: boolean } = {},
): Finding[] {
  const out: Finding[] = [];
  for (const [ns, tables] of Object.entries(facts)) {
    for (const [table, code] of Object.entries(tables)) {
      if (code === null) {
        out.push({ ns, table, kind: 'unparsable', detail: 'id 默认值非 gen_next_uid(<字面量>) 形态' });
        continue;
      }
      const reg = registry[table];
      if (reg === undefined) {
        out.push(
          opts.unregisteredAsLayerOnly
            ? { ns, table, kind: 'layer-only', detail: `本层独有（登记中无此表；字面量 code=${code}）` }
            : { ns, table, kind: 'unregistered', detail: `登记中无此表（字面量 code=${code}）` },
        );
        continue;
      }
      if (reg !== code) {
        out.push({ ns, table, kind: 'mismatch', detail: `字面量 code=${code} ≠ 登记 code=${reg}` });
      }
    }
  }
  return out.sort((a, b) => (a.ns + a.table).localeCompare(b.ns + b.table));
}
