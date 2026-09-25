/**
 * ego-lock-staleness 单测 —— 覆盖「持有者活着但卡死」这一 2026-09-22 实测死锁场景。
 * 纯函数，无浏览器、无文件系统。
 */
import { describe, expect, test } from 'bun:test';
import {
  HEARTBEAT_STALE_MS,
  lockStaleness,
} from './ego-lock-staleness';

describe('lockStaleness', () => {
  test('持有者存活且心跳新鲜 → 不可回收', () => {
    const v = lockStaleness({ pidAlive: true, heartbeatAgeMs: 5_000, tokenAgeMs: 5_000 });
    expect(v.stale).toBe(false);
    expect(v.reason).toBe('fresh');
  });

  test('持有者存活但心跳过期 → 可回收（本次死锁场景）', () => {
    const v = lockStaleness({
      pidAlive: true,
      heartbeatAgeMs: HEARTBEAT_STALE_MS + 1,
      tokenAgeMs: HEARTBEAT_STALE_MS + 1,
    });
    expect(v.stale).toBe(true);
    expect(v.reason).toBe('heartbeat-stale');
  });

  test('持有者存活、心跳文件缺失且锁年龄超阈值 → 可回收', () => {
    const v = lockStaleness({
      pidAlive: true,
      heartbeatAgeMs: null,
      tokenAgeMs: HEARTBEAT_STALE_MS + 1,
    });
    expect(v.stale).toBe(true);
    expect(v.reason).toBe('heartbeat-missing');
  });

  test('刚获取锁（心跳尚未落盘）→ 不可回收', () => {
    const v = lockStaleness({ pidAlive: true, heartbeatAgeMs: null, tokenAgeMs: 200 });
    expect(v.stale).toBe(false);
  });

  test('持有者进程已死 → 可回收（既有语义不变）', () => {
    const v = lockStaleness({ pidAlive: false, heartbeatAgeMs: 1_000, tokenAgeMs: 1_000 });
    expect(v.stale).toBe(true);
    expect(v.reason).toBe('holder-dead');
  });

  test('心跳恰好等于阈值 → 仍视为新鲜（严格大于才回收）', () => {
    const v = lockStaleness({
      pidAlive: true,
      heartbeatAgeMs: HEARTBEAT_STALE_MS,
      tokenAgeMs: HEARTBEAT_STALE_MS,
    });
    expect(v.stale).toBe(false);
  });
});
