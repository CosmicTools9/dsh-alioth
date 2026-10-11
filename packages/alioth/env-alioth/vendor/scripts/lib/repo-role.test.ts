#!/usr/bin/env bun
/**
 * repo-role.test.ts — 仓角色判据与「本 ns DSN」解析的确定性部分
 *
 * 覆盖（判据面 = 拆分后两类仓的 DSN 分流，错误分流会让 ns 面门禁连平台共享库）：
 *   · 角色判据（`.alioth/platform.conf` 存在性）；
 *   · ns 仓解析到本 ns env 的 DSN；
 *   · ns 仓**缺 env 时不得回落**平台共享库缺省（`url = null`）；
 *   · 显式 `DATABASE_URL` 优先级最高。
 *
 * 不覆盖：真库可达性（由 `db_ready` 的 pg_isready 通道承担，需真库）。
 */
/// <reference types="bun" />
import { afterEach, beforeEach, describe, expect, it } from 'bun:test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { NS_REPO_MARKER, repoRoleOf, repoRootOf, resolveRepoDbUrl } from './repo-role';

let repo = '';

function makeNsRepo(withEnv: boolean, ns = 'WZ'): void {
  mkdirSync(join(repo, '.alioth'), { recursive: true });
  mkdirSync(join(repo, 'Pre-Proc', ns), { recursive: true });
  mkdirSync(join(repo, 'Deploy', ns), { recursive: true });
  writeFileSync(join(repo, '.alioth', 'platform.conf'), '# ns 仓标记\n');
  if (withEnv) {
    writeFileSync(
      join(repo, 'Deploy', ns, '.env'),
      `# 包 env\nDATABASE_URL=postgresql://isahl@localhost:5432/${ns.toLowerCase()}\n`,
    );
  }
}

beforeEach(() => {
  repo = mkdtempSync(join(tmpdir(), 'repo-role-'));
  spawnSync('git', ['init', '-q'], { cwd: repo });
  delete process.env.DATABASE_URL;
});

afterEach(() => {
  rmSync(repo, { recursive: true, force: true });
  delete process.env.DATABASE_URL;
});

describe('repo-role：角色判据', () => {
  it('无 .alioth/platform.conf ⇒ 平台仓', () => {
    expect(repoRoleOf(repo)).toBe('platform');
    expect(repoRoleOf(join(repo, 'no-such-dir'))).toBe('platform');
  });

  it('有 .alioth/platform.conf ⇒ ns 仓；仓根取自 git toplevel', () => {
    makeNsRepo(true);
    expect(repoRoleOf(repo)).toBe('ns');
    expect(repoRootOf(join(repo, 'Deploy', 'WZ'))).toBe(realpathSync(repo));
  });

  it('壳/TS 的 ns 仓标记单源一致（跨语言常量漂移会让 DSN 与档位判定分裂）', async () => {
    const shell = await Bun.file(join(import.meta.dirname, 'repo-role.sh')).text();
    const declared = /REPO_ROLE_NS_MARKER="([^"]+)"/.exec(shell)?.[1];
    expect(declared).toBeDefined();
    expect(declared).toBe(NS_REPO_MARKER);
  });
});

describe('resolveRepoDbUrl：按仓角色解析 DSN', () => {
  it('ns 仓 ⇒ 本 ns env 的 DSN', () => {
    makeNsRepo(true);
    const got = resolveRepoDbUrl(repo);
    expect(got.url).toBe('postgresql://isahl@localhost:5432/wz');
    expect(got.source).toContain('Deploy/WZ/.env');
  });

  it('ns 仓缺 env ⇒ null（MUST NOT 回落平台共享库缺省）', () => {
    makeNsRepo(false);
    expect(resolveRepoDbUrl(repo).url).toBeNull();
  });

  it('平台仓走组件通道（本仓无 Deploy/{ns}/.env 面）', () => {
    const got = resolveRepoDbUrl(repo);
    // 平台仓的解析锚定平台 checkout 的组件 `.mise.toml`（本仓无该面时可解析不到 ⇒ null）
    if (got.url !== null) expect(got.source).not.toContain('Deploy/');
  });

  it('显式 DATABASE_URL 优先级最高（两仓角色同）', () => {
    makeNsRepo(true);
    process.env.DATABASE_URL = 'postgresql://ci@example.invalid:5432/ci_db';
    expect(resolveRepoDbUrl(repo).url).toBe('postgresql://ci@example.invalid:5432/ci_db');
  });
});
