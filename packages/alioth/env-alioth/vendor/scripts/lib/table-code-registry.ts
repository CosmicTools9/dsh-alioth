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

/** ns → 表 → 表码（null = 默认值形态不可解析） */
export type NsFacts = Record<string, Record<string, number | null>>;
export type Registry = Record<string, number>;

export interface Finding {
  ns: string;
  table: string;
  kind: 'mismatch' | 'unregistered' | 'unparsable';
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
  const path = process.env.NS_PORTS_CONF ?? join(root, 'Deploy', 'ports.conf');
  if (!existsSync(path)) return [];
  const out: string[] = [];
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    const s = line.trim();
    if (s.length === 0 || s[0] === '#') continue;
    let end = 0;
    while (end < s.length && s[end] !== ' ' && s[end] !== '\t') end++;
    const ns = s.slice(0, end);
    if (ns.length > 0) out.push(ns);
  }
  return out;
}

/** 平台库 DSN：env 优先，否则取 `Meta/backend/.mise.toml` 的 `DATABASE_URL.default` */
export function platformUrl(root: string): string {
  const fromEnv = process.env.PLATFORM_DATABASE_URL ?? process.env.DATABASE_URL;
  if (fromEnv) return fromEnv;
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
    const rows = psqlRows(
      url,
      "SELECT table_name, column_default FROM information_schema.columns WHERE table_schema='isahl' AND column_name='id' AND column_default LIKE '%gen_next_uid%' ORDER BY 1",
    );
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

/** 纯比对（不触碰 DB；`--self-test` 直接以此为断言对象） */
export function diffTableCodes(facts: NsFacts, registry: Registry): Finding[] {
  const out: Finding[] = [];
  for (const [ns, tables] of Object.entries(facts)) {
    for (const [table, code] of Object.entries(tables)) {
      if (code === null) {
        out.push({ ns, table, kind: 'unparsable', detail: 'id 默认值非 gen_next_uid(<字面量>) 形态' });
        continue;
      }
      const reg = registry[table];
      if (reg === undefined) {
        out.push({ ns, table, kind: 'unregistered', detail: `登记中无此表（字面量 code=${code}）` });
        continue;
      }
      if (reg !== code) {
        out.push({ ns, table, kind: 'mismatch', detail: `字面量 code=${code} ≠ 登记 code=${reg}` });
      }
    }
  }
  return out.sort((a, b) => (a.ns + a.table).localeCompare(b.ns + b.table));
}
