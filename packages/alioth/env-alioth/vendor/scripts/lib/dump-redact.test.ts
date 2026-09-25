/**
 * dump-redact 单元测试 — 脱敏判据**双载体形态**（COPY / 具名列 INSERT）
 *
 * 契约：`RULES` 是唯一清单（脱敏与零机密门禁共用）；两种载体形态 MUST 得到**等效**脱敏结果
 * （凭据列清空、JSON 凭据键移除、drop-data 表整段移除）；同一文件混用形态 MUST fail-loud
 * （`redactDump` 抛错——静默按一种形态处理会漏掉另一种载体的凭据）。
 * 动机：2026-09-23「消除豁免（全树零 COPY）」把元数据面快照从 COPY 换成具名列 INSERT 后，
 * 脱敏链 MUST 在新载体上保持同一判据（实测坑：COPY 头列名不带引号 ⇒ 保留字列名入 INSERT 需引号化）。
 */
import { describe, expect, it } from 'bun:test';
import { NULL_TOKEN, redactDump, type RedactStats } from './dump-redact.ts';

const newStats = (): RedactStats => ({
  droppedTables: [],
  droppedRows: 0,
  redactedRows: 0,
  redactedFields: 0,
  warnings: [],
});

const COPY_DS = 'COPY isahl_meta.meta_data_source (id, config, connection_string, password_encrypted) FROM stdin;';
const COPY_INSERT_ROW = `4\t{"host":"h","password_encrypted":"ENC:A"}\tpostgres://u:p@h/db\tENC:B`;
const INSERT_DS =
  'INSERT INTO isahl_meta.meta_data_source ("id", "config", "connection_string", "password_encrypted") ' +
  'VALUES (\'4\', \'{"host":"h","password_encrypted":"ENC:A"}\', \'postgres://u:p@h/db\', \'ENC:B\');';

describe('redactDump — COPY 载体（既有形态，回归锁）', () => {
  it('凭据列清空 + JSON 凭据键移除 + drop-data 表整段移除', () => {
    const stats = newStats();
    const out = redactDump(
      [
        COPY_DS,
        COPY_INSERT_ROW,
        '\\.',
        'COPY isahl_meta.meta_llm_provider (id, api_key) FROM stdin;',
        "1\tsk-plaintext",
        '\\.',
      ].join('\n'),
      stats,
    );
    expect(out).toContain(`4\t{"host":"h"}\t${NULL_TOKEN}\t${NULL_TOKEN}`);
    expect(out).not.toContain('ENC:A');
    expect(out).not.toContain('sk-plaintext');
    expect(stats.redactedFields).toBe(3); // password_encrypted / connection_string / config JSON 键
    expect(stats.droppedTables).toContain('isahl_meta.meta_llm_provider');
  });
});

describe('redactDump — 具名列 INSERT 载体（元数据面新形态）', () => {
  it('数值列置 NULL、JSON 键移除、drop-data 语句整条删除', () => {
    const stats = newStats();
    const out = redactDump(
      [
        INSERT_DS,
        "INSERT INTO isahl_meta.meta_llm_provider (id, api_key) VALUES ('1', 'sk-plaintext');",
      ].join('\n'),
      stats,
    );
    expect(out).toContain("VALUES ('4', '{\"host\":\"h\"}', NULL, NULL);");
    expect(out).not.toContain('ENC:A');
    expect(out).not.toContain('sk-plaintext');
    expect(stats.redactedRows).toBe(1);
    expect(stats.droppedRows).toBe(1);
  });

  it('保留字列名（`window`）不破坏语句结构判定', () => {
    const stats = newStats();
    const out = redactDump(
      'INSERT INTO isahl_meta.meta_mise_services ("id", "window", "run_token") ' +
        "VALUES ('1', 'w-1', 'tok');",
      stats,
    );
    expect(out).toContain("VALUES ('1', 'w-1', NULL);");
    expect(stats.redactedFields).toBe(1);
  });

  it('跨行语句（值内含换行、行尾 `;`）不误切', () => {
    const stats = newStats();
    const out = redactDump(
      'INSERT INTO isahl_meta.meta_mise_env_vars ("id", "var_key", "var_value") ' +
        "VALUES ('1', 'SMTP;\nPASS', 's3cret');",
      stats,
    );
    expect(out).not.toContain("'s3cret'");
    expect(out).toContain('NULL);');
    expect(stats.redactedFields).toBe(1);
  });

  it('幂等：二次脱敏无变化', () => {
    const once = redactDump([INSERT_DS].join('\n'), newStats());
    const twice = redactDump(once, newStats());
    expect(twice).toBe(once);
  });
});

describe('redactDump — 形态边界', () => {
  it('混用 COPY 与 INSERT ⇒ fail-loud（拒绝产出半脱敏文件）', () => {
    const mixed = [COPY_DS, COPY_INSERT_ROW, '\\.', INSERT_DS].join('\n');
    expect(() => redactDump(mixed, newStats())).toThrow('混用 COPY 与 INSERT 形态');
  });
});
