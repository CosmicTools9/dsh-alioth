/**
 * baseline-freeze.test.ts — 冻结判定单源的常驻测试
 * （openspec: harden-check-all-baseline-freeze；spec: precommit-execution
 *  :: check-all-baseline-freeze-reproducibility）
 *
 * 经既有 T1 项 `scripts-unit-tests`（`cd scripts && bun test .`）执行。
 * 断言打在**判定语义**上：标记必须剥夺豁免力（否则本 change 只是加注释）。
 */
import { describe, expect, test } from 'bun:test';
import { classifyFreeze, entryExemptsBaseline, selfTest } from './baseline-freeze';

describe('classifyFreeze（冻结前脏面分类）', () => {
  test('空 porcelain ⇒ 干净面：允许冻结且不打标记', () => {
    const c = classifyFreeze([]);
    expect(c.dirtyFace).toBe(false);
    expect(c.count).toBe(0);
  });

  test('单行 ⇒ 脏面', () => {
    expect(classifyFreeze([' M scripts/check/check-all.sh']).dirtyFace).toBe(true);
  });

  test('空白行不计入（避免 \n 尾随造成的假脏）', () => {
    expect(classifyFreeze(['', '   ', '']).count).toBe(0);
  });

  test('样例行截断到 8 条且逐行限长（仅展示用）', () => {
    const many = Array.from({ length: 12 }, (_, i) => ` M f${i}.ts`);
    const c = classifyFreeze(many);
    expect(c.count).toBe(12);
    expect(c.sample.length).toBe(8);
    const long = classifyFreeze([` M ${'a'.repeat(200)}.ts`]);
    expect(long.sample[0].length).toBeLessThanOrEqual(121);
  });
});

describe('entryExemptsBaseline（豁免判定表）', () => {
  test('pass ⇒ 不豁免（pass→fail 即回归；既有语义不变，本 change 只加归因提示）', () => {
    expect(entryExemptsBaseline('pass', false)).toBe(false);
    expect(entryExemptsBaseline('pass', true)).toBe(false);
  });

  test('grandfathered + 无标记 ⇒ 豁免（存量 fail 冻结，既有语义逐字不变）', () => {
    expect(entryExemptsBaseline('grandfathered', false)).toBe(true);
  });

  test('grandfathered/skip + dirtyFace 标记 ⇒ **不豁免**（脏面存量不得掩盖提交面失败）', () => {
    expect(entryExemptsBaseline('grandfathered', true)).toBe(false);
    expect(entryExemptsBaseline('skip', true)).toBe(false);
  });

  test('缺席 ⇒ 不豁免（新项失败语义不变）', () => {
    expect(entryExemptsBaseline('', false)).toBe(false);
    expect(entryExemptsBaseline('', true)).toBe(false);
  });

  test('不变量：dirtyFace 只增加上报、绝不减少（否则它成为新的掩盖通道）', () => {
    const statuses = ['pass', 'grandfathered', 'skip', 'fail', ''];
    for (const s of statuses) {
      const clean = entryExemptsBaseline(s, false);
      const dirty = entryExemptsBaseline(s, true);
      expect(clean === true && dirty === true).toBe(false);
    }
  });
});

describe('--self-test 自证', () => {
  test('内置用例全过', () => {
    expect(selfTest()).toBe(0);
  });
});
