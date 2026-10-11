// sql-scope 解析归一判定的 fail-closed 契约测试（2026-10-03 回归来源）
//
// 背景：`pg-query-emscripten` 在内存压力下会返回**无 `parse_tree` 的畸形结果**（封装每次 parse 泄漏
// 输入缓冲）。修复前 `parseStatements` 把它归一成「零语句 = 无违规」⇒ isahl 冻结面门禁 / OMP
// safety-guard / exec-sql 判定会把 `ALTER TABLE isahl.X DROP COLUMN …` 静默放行（check-all 并发实测）。
import { describe, expect, it } from 'bun:test';
import { interpretParseResult, parseStatements, stripComments } from './sql-scope.ts';

// 解析器可用性探针（真实解析依赖 `pg-query-emscripten`，来自平台 node_modules）：
// push 钩子的校验 worktree 是纯净检出（无 node_modules）⇒ 依赖不可用时解析路径断言**显式 skip 并
// 打印原因**（仓内惯例：不可用即显式 skip，MUST NOT 静默判过/静默判错）。
const parserOk = await (async () => {
  const r = await parseStatements('SELECT 1;');
  return 'stmts' in r && r.stmts.length === 1;
})();
if (!parserOk) {
  console.warn(
    '⚠ sql-scope.test.ts: pg-query-emscripten 不可用 —— 「真实 wasm 路径兜底」两条断言 skip（判据面 = 平台本机依赖）',
  );
}
const itParser = parserOk ? it : it.skip;

describe('interpretParseResult — fail closed 契约', () => {
  it('畸形输出（无 parse_tree）且输入含 SQL 实体 ⇒ error（不再静默放行）', () => {
    const r = interpretParseResult({}, 'ALTER TABLE isahl.zc_id_scene DROP COLUMN notice;');
    expect('error' in r).toBe(true);
  });

  it('空语句集但输入含 SQL 实体 ⇒ error', () => {
    const r = interpretParseResult({ parse_tree: { stmts: [] } }, 'DROP TABLE isahl.zc_id_scene;');
    expect('error' in r).toBe(true);
  });

  it('空语句集 + 纯注释/空白输入 ⇒ 合法零语句（不误报）', () => {
    for (const text of ['-- 只是注释\n', '/* 块注释 */', '', '   \n  ']) {
      const r = interpretParseResult({ parse_tree: { stmts: [] } }, text);
      expect('stmts' in r && r.stmts.length === 0).toBe(true);
    }
  });

  it('解析器 error 原样透传（含无 message 形态）', () => {
    expect(interpretParseResult({ error: { message: 'boom' } }, 'SELECT 1;')).toEqual({ error: 'boom' });
    const noMsg = interpretParseResult({ error: {} }, 'SELECT 1;');
    expect('error' in noMsg).toBe(true);
  });

  it('正常语句树原样返回', () => {
    const r = interpretParseResult({ parse_tree: { stmts: [{ stmt: {}, stmt_len: 9 }] } }, 'SELECT 1;');
    expect('stmts' in r && r.stmts.length === 1).toBe(true);
  });
});

describe('parseStatements — 真实 wasm 路径兜底', () => {
  itParser('DDL 语句解析出 1 语句（拦截面证据链的起点）', async () => {
    const r = await parseStatements('ALTER TABLE isahl.zc_id_scene DROP COLUMN notice;');
    expect('stmts' in r && r.stmts.length === 1).toBe(true);
  });

  itParser('纯注释输入不因 fail-closed 判据误报', async () => {
    const r = await parseStatements('-- 注释\n');
    expect('stmts' in r && r.stmts.length === 0).toBe(true);
  });

  it('超限输入走 error 分支（fail closed）', async () => {
    const big = `SELECT '${'x'.repeat(60_000)}';`;
    const r = await parseStatements(big);
    expect('error' in r).toBe(true);
  });
});

describe('stripComments — fail-closed 判据的内容口径', () => {
  it('注释与空白剥离后为空 ⇒ 零语句合法', () => {
    expect(stripComments('-- a\n/* b */\n  ').trim()).toBe('');
  });

  it('含 SQL 实体 ⇒ 非空', () => {
    expect(stripComments('/* c */ ALTER TABLE x;').trim().length > 0).toBe(true);
  });
});
