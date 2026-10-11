/// <reference types="bun" />
// baseline.ts（基线判定单一实现）单元测试 —— change deepen-check-substrate C3
// 判据自证的常驻面（`--self-test` 供 CLI；本文件供 `scripts-unit-tests`）
import { describe, expect, it } from 'bun:test';
import { exempts, partition, rot, selfTest } from './baseline';

describe('baseline.partition', () => {
  const keyOf = (v: { path: string }): string => v.path;

  it('基线内 ⇒ 存量；基线外 ⇒ 新增', () => {
    const { fresh, grandfathered } = partition([{ path: 'a' }, { path: 'b' }], new Set(['a']), keyOf);
    expect(grandfathered.map((v) => v.path)).toEqual(['a']);
    expect(fresh.map((v) => v.path)).toEqual(['b']);
  });

  it('负例：空基线 ⇒ 全部算新增（不得静默放行）', () => {
    expect(partition([{ path: 'a' }], new Set(), keyOf).fresh.length).toBe(1);
  });
});

describe('baseline.rot', () => {
  it('登记但不再命中 ⇒ 死条目', () => {
    expect(rot(['a', 'zombie'], new Set(['a']))).toEqual(['zombie']);
  });

  it('负例：全部命中 ⇒ 无死条目（不得误报）', () => {
    expect(rot(['a'], new Set(['a']))).toEqual([]);
  });
});

describe('baseline.exempts（冻结判定表）', () => {
  it('pass ⇒ 不豁免（回归不得被掩盖）', () => {
    expect(exempts('pass', false)).toBe(false);
  });

  it('缺席 ⇒ 不豁免', () => {
    expect(exempts('', false)).toBe(false);
  });

  it('grandfathered + 干净面 ⇒ 豁免', () => {
    expect(exempts('grandfathered', false)).toBe(true);
  });

  it('负例：脏面 ⇒ 一律不豁免（豁免不得放宽）', () => {
    expect(exempts('grandfathered', true)).toBe(false);
    expect(exempts('skip', true)).toBe(false);
  });
});

describe('baseline.selfTest', () => {
  it('CLI 自证脚手架全通过', () => {
    expect(selfTest()).toBe(0);
  });
});
