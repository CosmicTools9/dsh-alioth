#!/usr/bin/env bun
/**
 * gate-db-digest.test.ts — 共享缓存原语的确定性部分
 *
 * 覆盖：`cacheDisabled` 逃生开关、`cacheHit`/`cacheWrite` 往返与原子替换、null 键语义、
 * `cacheFile` 的 git-common-dir 解析（worktree 与主 checkout 共享同一份）。
 * **不覆盖** DB 摘要本身（需真库）：那部分由 check-context-fields / check-fk-index 的冷/温
 * 实测与「注入漂移必被检出」实测覆盖（见 change tighten-gate-scope-coverage 的 tasks 证据）。
 */
/// <reference types="bun" />
import { afterEach, beforeEach, describe, expect, it } from 'bun:test';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { cacheFile, cacheHit, cacheWrite, cacheDisabled } from './gate-db-digest';

let repo = '';

beforeEach(() => {
  repo = mkdtempSync(join(tmpdir(), 'gate-digest-'));
  spawnSync('git', ['init', '-q'], { cwd: repo });
});

afterEach(() => {
  rmSync(repo, { recursive: true, force: true });
  delete process.env.ALIOTH_NO_GATE_CACHE;
});

/** 在指定 cwd 下执行（cacheFile 依赖 cwd 的 git-common-dir 解析）。 */
async function inRepo<T>(fn: () => Promise<T>): Promise<T> {
  const prev = process.cwd();
  process.chdir(repo);
  try {
    return await fn();
  } finally {
    process.chdir(prev);
  }
}

describe('gate-db-digest · 缓存原语', () => {
  it('cacheWrite → cacheHit 往返；不同键不命中', async () => {
    await inRepo(async () => {
      expect(await cacheHit('t.key', 'k1')).toBe(false);
      await cacheWrite('t.key', 'k1');
      expect(await cacheHit('t.key', 'k1')).toBe(true);
      expect(await cacheHit('t.key', 'k2')).toBe(false);
      // 原子替换：写新键后旧键不再命中
      await cacheWrite('t.key', 'k2');
      expect(await cacheHit('t.key', 'k2')).toBe(true);
      expect(await cacheHit('t.key', 'k1')).toBe(false);
    });
  }, 30_000);

  it('null 键既不写也不命中（缓存不可用时的降级语义）', async () => {
    await inRepo(async () => {
      await cacheWrite('t.key', null);
      expect(await cacheHit('t.key', null)).toBe(false);
      const path = cacheFile('t.key');
      expect(path).not.toBeNull();
      expect(existsSync(String(path))).toBe(false);
    });
  }, 30_000);

  it('ALIOTH_NO_GATE_CACHE=1 关闭缓存（逃生开关）', () => {
    delete process.env.ALIOTH_NO_GATE_CACHE;
    expect(cacheDisabled()).toBe(false);
    process.env.ALIOTH_NO_GATE_CACHE = '1';
    expect(cacheDisabled()).toBe(true);
  });

  it('cacheFile 落在 <git-common-dir>/../.parallel/cache/（worktree 共享同一份）', async () => {
    await inRepo(async () => {
      const path = String(cacheFile('x.key'));
      // macOS 上 mkdtemp 的 /var/... 与 chdir 后 cwd 的 /private/var/... 是同一目录 ⇒ 用 realpath 比对
      expect(path).toBe(join(realpathSync(repo), '.parallel', 'cache', 'x.key'));
      await cacheWrite('x.key', 'abc');
      expect(readFileSync(path, 'utf8').trim()).toBe('abc');
    });
  }, 30_000);

  it('缓存目录不存在时自动创建（新克隆首次写缓存不失败）', async () => {
    await inRepo(async () => {
      rmSync(join(repo, '.parallel'), { recursive: true, force: true });
      await cacheWrite('deep.key', 'v');
      expect(await cacheHit('deep.key', 'v')).toBe(true);
    });
  }, 30_000);
});
