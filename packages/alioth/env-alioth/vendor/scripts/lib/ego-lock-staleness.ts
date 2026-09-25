/**
 * ego-lock-staleness.ts — 视觉验证共享锁的**陈旧判据**（纯函数，单源 + 可单测）。
 *
 * 为什么需要心跳判据：原实现只在持有者 **PID 已死** 时回收锁；持有者**活着但卡死**
 * （CDP 无响应，CPU `0:00`、`elapsed` 数分钟）会**永久占锁**，使全仓视觉验证通道死锁
 * （2026-09-22 实测两次：pid 34169 / 85115）。故等待方在「心跳过期或缺失且锁年龄超阈值」时
 * MUST 同样走原子 quarantine 回收——与「PID 已死」并列，互不替代。
 *
 * 阈值依据：正常单档 capture 约 10–30s、单轮 verify 约 85s（2026-09-22 实测）；
 * 心跳间隔 5s、陈旧阈值 90s ⇒ 至少漏 18 拍才判陈旧，误杀风险远低于收益。
 */
export const HEARTBEAT_INTERVAL_MS = 5_000;
export const HEARTBEAT_STALE_MS = 90_000;
/** 共享锁默认等待窗口：MUST > 单轮 verify 时长（实测 ~85s），否则并发会话结构性饥饿。 */
export const LOCK_WAIT_DEFAULT_MS = 180_000;

export interface LockStalenessInput {
  /** 持有者 PID 是否存活（token 中解析） */
  pidAlive: boolean;
  /** 心跳文件年龄（ms）；`null` = 心跳文件不存在（旧版本持有者或刚获取锁的瞬间） */
  heartbeatAgeMs: number | null;
  /** 锁 token 的年龄（ms），用于「心跳缺失」时的兜底判据 */
  tokenAgeMs: number;
  /** 陈旧阈值（默认 HEARTBEAT_STALE_MS） */
  staleMs?: number;
}

export interface LockStalenessVerdict {
  stale: boolean;
  reason: 'holder-dead' | 'heartbeat-stale' | 'heartbeat-missing' | 'fresh';
}

/** 判定锁是否可被等待方回收（原子 quarantine）。 */
export function lockStaleness(input: LockStalenessInput): LockStalenessVerdict {
  const staleMs = input.staleMs ?? HEARTBEAT_STALE_MS;
  if (!input.pidAlive) return { stale: true, reason: 'holder-dead' };
  if (input.heartbeatAgeMs === null) {
    return input.tokenAgeMs > staleMs
      ? { stale: true, reason: 'heartbeat-missing' }
      : { stale: false, reason: 'fresh' };
  }
  return input.heartbeatAgeMs > staleMs
    ? { stale: true, reason: 'heartbeat-stale' }
    : { stale: false, reason: 'fresh' };
}
