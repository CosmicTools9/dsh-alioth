import { describe, expect, test } from 'bun:test';
import { commandOf, lstartOf } from '../ego/lease';
import {
  leaseAbandoned,
  ownerAlive,
  ownerProvablyGone,
  runProvablyDead,
  type LeaseRecord,
} from './ego-lease-liveness';

/** 一个「不存在的 pid」：kill(pid,0) 必失败 ⇒ 可证明已退出 */
const DEAD_PID = 4_000_000;

/** 本进程的真实指纹（活体样本） */
function liveFingerprint(pid: number): { pid: number; cmd: string; start: string } {
  const cmd = commandOf(pid);
  const start = lstartOf(pid);
  expect(cmd).not.toBe('');
  expect(start).not.toBe('');
  return { pid, cmd, start };
}

function base(over: Partial<LeaseRecord> = {}): LeaseRecord {
  const self = liveFingerprint(process.pid);
  return {
    name: 'probe',
    ownerPid: self.pid,
    ownerVerified: true,
    ownerStart: self.start,
    ownerCmd: self.cmd,
    lastTouch: Date.now(),
    ...over,
  };
}

describe('lease 存活判据 — 僵尸空间（owner 存活 + run 已退出）', () => {
  // 事故形态：运行被 SIGKILL / 工具超时中断，owner（omp 会话）仍存活数天，
  // 只认 owner 时该 lease 永久「活跃」⇒ 空间与其 renderer 永不回收。
  test('owner 存活 + run 已退出 ⇒ 可证明已放弃（本次变更的核心）', () => {
    const l = base({ runPid: DEAD_PID, runCmd: 'dead-run-cmd', runStart: 'Mon Jan  1 00:00:00 2024' });
    expect(ownerAlive(l)).toBe(true);
    expect(runProvablyDead(l)).toBe(true);
    expect(leaseAbandoned(l)).toBe(true);
  });

  test('owner 存活 + run 存活（真指纹）⇒ 活跃', () => {
    const run = liveFingerprint(process.pid);
    const l = base({ runPid: run.pid, runCmd: run.cmd, runStart: run.start });
    expect(runProvablyDead(l)).toBe(false);
    expect(leaseAbandoned(l)).toBe(false);
  });

  test('owner 已退出 ⇒ 可证明已放弃（run 未登记亦然）', () => {
    const l = base({
      ownerPid: DEAD_PID,
      ownerCmd: 'dead-owner-cmd',
      ownerStart: 'Mon Jan  1 00:00:00 2024',
    });
    expect(ownerProvablyGone(l)).toBe(true);
    expect(leaseAbandoned(l)).toBe(true);
  });

  test('旧记录（未登记 run 身份）⇒ 退回 owner-only 判据，不得误收', () => {
    expect(leaseAbandoned(base())).toBe(false);
    expect(runProvablyDead(base())).toBe(false);
  });

  test('ownerVerified=false ⇒ 无 run 证据时不回收（保守，交人工）', () => {
    expect(leaseAbandoned(base({ ownerVerified: false }))).toBe(false);
  });

  test('run 证据独立成立：ownerVerified=false + run 已退出 ⇒ 回收（终端直跑的工具亦覆盖）', () => {
    const l = base({
      ownerVerified: false,
      runPid: DEAD_PID,
      runCmd: 'dead-run-cmd',
      runStart: 'Mon Jan  1 00:00:00 2024',
    });
    expect(leaseAbandoned(l)).toBe(true);
  });

  test('run 活 ⇒ 即使 owner 未验证也不回收', () => {
    const run = liveFingerprint(process.pid);
    const l = base({ ownerVerified: false, runPid: run.pid, runCmd: run.cmd, runStart: run.start });
    expect(leaseAbandoned(l)).toBe(false);
  });

  test('指纹不可用（缺 cmd/start）⇒ 无证据，不判死', () => {
    const noOwnerFp = base({ ownerCmd: '', ownerStart: '' });
    expect(leaseAbandoned(noOwnerFp)).toBe(false);
    const noRunFp = base({ runPid: DEAD_PID, runCmd: '', runStart: '' });
    expect(runProvablyDead(noRunFp)).toBe(false);
    expect(leaseAbandoned(noRunFp)).toBe(false);
  });
});
