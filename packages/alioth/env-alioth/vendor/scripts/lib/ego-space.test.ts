/**
 * 回归：ego task space 创建表达式生成（`scripts/lib/ego-space.ts`）。
 *
 * 该模块的产物是喂给 `ego-browser nodejs` 的 heredoc 源码，被 5 个创建点共用；
 * 契约（任一条破坏都会静默改行为）：
 *  ① 未指定 profile ⇒ 逐字等于历史字面量 `useOrCreateTaskSpace("<name>")`（默认 profile 零漂移）；
 *  ② 指定 profile ⇒ 走 v2 `taskSpaceWithProfile` 并注入 prelude，且 prelude 含「已存在」回退复用；
 *  ③ v1 专有选项只在无 profile 路径出现（v2 `taskSpace()` 拒绝未知选项）；
 *  ④ profile 解析：显式参数 > `EGO_PROFILE_ID` > 未设置（空白视为未设置）。
 */
import { describe, expect, it } from 'bun:test';
import {
  EGO_PROFILE_ENV,
  EGO_SPACE_PRELUDE,
  egoCreateSpaceExpr,
  egoSpaceHeader,
  resolveEgoProfile,
} from './ego-space';

describe('ego 空间创建表达式', () => {
  it('未指定 profile 时与历史字面量逐字一致', () => {
    expect(egoCreateSpaceExpr('"alioth-x-1"')).toBe('useOrCreateTaskSpace("alioth-x-1")');
  });

  it('v1 专有选项只出现在无 profile 路径', () => {
    expect(egoCreateSpaceExpr("'n-' + Date.now()", undefined, '{ visible: false }')).toBe(
      "useOrCreateTaskSpace('n-' + Date.now(), { visible: false })",
    );
    expect(egoCreateSpaceExpr('"s"', 'Profile 4', '{ visible: false }')).toBe(
      'taskSpaceWithProfile("s", "Profile 4")',
    );
  });

  it('指定 profile 时注入含「已存在」回退的 prelude，head 用 v2 调用', () => {
    const header = egoSpaceHeader('"s"', 'Profile 4');
    expect(header.startsWith(EGO_SPACE_PRELUDE)).toBe(true);
    expect(header).toContain('const task = await taskSpaceWithProfile("s", "Profile 4")');
    expect(EGO_SPACE_PRELUDE).toContain('taskSpace(name, { profileId })');
    expect(EGO_SPACE_PRELUDE).toContain('only applies when creating');
  });

  it('未指定 profile 时不注入 prelude', () => {
    expect(egoSpaceHeader('"s"')).toBe('const task = await useOrCreateTaskSpace("s")\n');
  });
});

describe('profile 解析', () => {
  it('显式参数优先于环境变量，空白视为未设置', () => {
    const before = process.env[EGO_PROFILE_ENV];
    process.env[EGO_PROFILE_ENV] = 'From Env';
    try {
      expect(resolveEgoProfile('Explicit')).toBe('Explicit');
      expect(resolveEgoProfile()).toBe('From Env');
      expect(resolveEgoProfile('   ')).toBe('From Env');
      process.env[EGO_PROFILE_ENV] = '  ';
      expect(resolveEgoProfile()).toBeUndefined();
    } finally {
      if (before === undefined) delete process.env[EGO_PROFILE_ENV];
      else process.env[EGO_PROFILE_ENV] = before;
    }
  });
});
