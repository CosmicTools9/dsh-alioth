/**
 * psql-invocation / sql-scope 单元测试 — 守卫判定契约
 *
 * 契约（bug 由用户实测报出）：**只读路径 MUST NOT 进入拦截/询问面**，判定依据 = 执行内容，
 * 而非命令文本形态；文件与管道载荷 MUST 被读取后判定；内容确实不可读才归入人工裁决面。
 * 被测实现 = `.omp/extensions/{db-pipeline-guard,safety-guard}.ts` 依赖的两个共享库。
 */
import { afterAll, describe, expect, it } from 'bun:test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { scanDbCommand } from './psql-invocation.ts';
import { decideSqlScope } from './sql-scope.ts';

const DIR = mkdtempSync(join(tmpdir(), 'db-guard-unit-'));
writeFileSync(join(DIR, 'read.sql'), 'SELECT count(*) FROM isahl.zc_id_scene;\n', 'utf8');
writeFileSync(join(DIR, 'drop.sql'), 'DROP TABLE isahl.zc_id_scene;\n', 'utf8');
writeFileSync(join(DIR, 'anchor.sql'), 'INSERT INTO isahl_meta.meta_fields (id) VALUES (1);\n', 'utf8');
writeFileSync(join(DIR, 'allowed.sql'), 'CREATE TABLE wz_fssc.probe (id int);\n', 'utf8');
afterAll(() => rmSync(DIR, { recursive: true, force: true }));

const scan = (command: string) => scanDbCommand(command, DIR);

describe('载荷提取：只读路径不得落入不可读面', () => {
  const readOnly = [
    'psql "$DATABASE_URL" -c "SELECT 1"',
    'psql "$DATABASE_URL" -tAc "SELECT count(*) FROM isahl.zc_id_scene"',
    'psql -X -q -d db --command "SELECT 1"',
    "psql \"$DATABASE_URL\" <<'SQL'\nSELECT 1;\nSQL",
    'psql -f read.sql',
    'psql --file=read.sql',
    'psql "$DATABASE_URL" < read.sql',
    'cat read.sql | psql',
    'psql "$DATABASE_URL" -c "$(cat read.sql)"',
  ];
  for (const command of readOnly) {
    it(`内容可读：${command.replace(/\n/g, '⏎').slice(0, 58)}`, () => {
      const r = scan(command);
      expect(r.opaque).toEqual([]);
      expect(r.payloads.length).toBeGreaterThan(0);
      expect(r.payloads.every((p) => p.text.trim().length > 0)).toBe(true);
    });
  }

  it('文件载荷内容被真实读取（含冻结面 DDL）', () => {
    expect(scan('psql -f drop.sql').payloads[0]!.text).toContain('DROP TABLE isahl.zc_id_scene');
    expect(scan('cat drop.sql | psql').payloads[0]!.text).toContain('DROP TABLE isahl.zc_id_scene');
  });
});

describe('载荷提取：不可读面与不触发面', () => {
  it('文件不可读 → opaque', () => {
    expect(scan('psql -f nope.sql').opaque.join()).toContain('不可读');
  });

  it('变换流管道 → opaque（内容已变形，不可判定）', () => {
    expect(scan('grep -v x read.sql | psql').opaque.join()).toContain('变换流');
  });

  it('裸交互式 psql → opaque', () => {
    expect(scan('psql "$DATABASE_URL"').opaque.join()).toContain('交互式');
  });

  it('非执行形态不触发（version / which / 写文件 / 非 psql 命令）', () => {
    for (const command of [
      'psql --version',
      'which psql',
      'grep -rn psql scripts/',
      "cat > note.sql <<'EOF'\npsql -c \"DROP TABLE isahl.x\"\nEOF",
      'git log --oneline -3 -- scripts/db/sync-db.sh',
    ]) {
      const r = scan(command);
      expect({ command, psqlInvoked: r.psqlInvoked, payloads: r.payloads.length, opaque: r.opaque.length }).toEqual({
        command,
        psqlInvoked: false,
        payloads: 0,
        opaque: 0,
      });
    }
  });

  it('sh -c 包装递归（包装不构成不可见通道）', () => {
    const r = scan('bash -c "psql \\"$DATABASE_URL\\" -c \'DROP TABLE isahl.w\'"');
    expect(r.psqlInvoked).toBe(true);
    expect(r.payloads[0]!.text).toContain('DROP TABLE isahl.w');
  });
});

describe('内容判定：冻结面 / 声明面 / 允许面 / 只读', () => {
  const causes: [string, string, string | null][] = [
    ['只读', 'SELECT count(*) FROM isahl.zc_id_scene', null],
    ['只读含展开', 'SELECT $cols FROM t', null],
    ['只读含 DDL 字面量', "SELECT 'ALTER TABLE isahl.x ADD COLUMN y int'", null],
    ['只读含注释 DDL', '-- DROP TABLE isahl.x\nSELECT 1', null],
    ['isahl 建表', 'CREATE TABLE isahl.x (id bigint)', 'isahl'],
    ['isahl CTAS', 'CREATE TABLE isahl.x AS SELECT 1', 'isahl'],
    ['isahl SELECT INTO', 'SELECT * INTO isahl.z FROM t', 'isahl'],
    ['isahl 加列', 'ALTER TABLE isahl.t ADD COLUMN c int', 'isahl'],
    ['isahl 改名', 'ALTER TABLE isahl.t RENAME COLUMN a TO b', 'isahl'],
    ['isahl 删表', 'DROP TABLE isahl.x', 'isahl'],
    ['DO 块体内联建表', 'DO $$ BEGIN CREATE TABLE isahl.q (id int); END $$', 'isahl'],
    ['允许子句（约束）', "ALTER TABLE isahl.t ADD CONSTRAINT ck CHECK (x <> '')", null],
    ['允许面 schema', 'CREATE TABLE wz_fssc.p (id int)', null],
    ['临时表', 'CREATE TEMP TABLE t (id int)', null],
    ['非表族对象', 'CREATE VIEW isahl.v AS SELECT 1', null],
    ['TRUNCATE（归提交面）', 'TRUNCATE isahl.x', null],
    ['锚点写入', 'INSERT INTO isahl_meta.meta_fields (id) VALUES (1)', 'anchor'],
    ['锚点 COPY', 'COPY isahl_meta.meta_fields FROM stdin', 'anchor'],
    ['非锚点 meta 写', 'INSERT INTO isahl_meta.evo_episodes (a) VALUES (1)', null],
    ['未限定目标', 'CREATE TABLE zc_x (id int)', 'unknown'],
    ['展开载荷', '$SQL', 'unresolvable'],
    // 回归（2026-09-20 实测误报）：DO 块内的注释与限定名解析
    [
      'DO 块内注释提及 DROP TABLE',
      'DO $$ BEGIN\n  -- ③ `DROP TABLE` 无 CASCADE ⇒ 失败\n  DROP TABLE IF EXISTS isahl_auth._ag_label_vertex CASCADE;\nEND $$',
      null,
    ],
    ['DO 块内块注释提及 DROP TABLE', 'DO $$ BEGIN /* DROP TABLE isahl.x */ NULL; END $$', null],
    ['DO 块内未限定删表', 'DO $$ BEGIN DROP TABLE zc_x; END $$', 'unknown'],
    ['DO 块内 isahl 删表', 'DO $$ BEGIN DROP TABLE isahl.x CASCADE; END $$', 'isahl'],
  ];
  for (const [name, sql, cause] of causes) {
    it(`${name} → ${cause ?? '放行'}`, async () => {
      expect((await decideSqlScope(sql)).cause).toBe(cause);
    });
  }
});
