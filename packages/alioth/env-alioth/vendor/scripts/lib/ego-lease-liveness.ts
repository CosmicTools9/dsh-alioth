/**
 * ego-lease-liveness.ts — ego task space lease 的存活判据（唯一实现）
 *
 * lease 记录两组身份（都按「PID + 命令行 + 启动时刻」三要素指纹校验，防 PID 复用误判）：
 *   - **owner**（`touch` 自动解析）= PPID 链上第一个 omp 会话进程 —— **天级**存活；
 *   - **run**（`touch --run-pid <pid>` 显式传入）= 执行本次运行的进程（工具/脚本自身）
 *     —— **分钟级**存活，正常结束会 `unregister`。
 *
 * 两组指纹各有含义，任一组失配都是「可证明已放弃」，且**两条链独立成立**：
 *   - owner 失配 ⇒ 会话没了，空间无人认领（需 `ownerVerified` 才可自动回收）；
 *   - run   失配 ⇒ 本次运行进程已退出，却仍留着 lease（正常结束会注销）⇒ 被中断的僵尸
 *     （**不需要** owner 验证——终端直跑的工具同样适用）。
 *
 * 为什么需要 run：只认 owner 时，被 SIGKILL / 工具超时中断的运行会留下「owner 存活（会话仍在）
 * + 空间存在」的记录，被 watcher 与门禁双双判为活跃而永不回收（lease key 内嵌该次运行的
 * pid+时间戳，永不复用 ⇒ 永久僵尸）。实测 2026-10-05：7 个此类空间静置 18–22h、~1.3GB。
 *
 * 消费方（MUST 同源）：`scripts/ego/ego-space-watcher.ts`、`scripts/check/check-ego-task-spaces.sh`
 * —— 二者曾各写一套（门禁只做 `kill(pid,0)`），结论会分裂。
 */
import { execSync } from 'node:child_process';
import { PS_ENV } from './ego-ps-env';

export interface LeaseRecord {
  name?: string;
  ownerPid: number;
  /** true = PPID 链上找到明确 omp 会话进程（可安全自动回收的前提） */
  ownerVerified: boolean;
  ownerStart: string;
  ownerCmd: string;
  lastTouch?: number;
  /** 调用状态印记：starting=spawn 前；running=操作成功；error=异常/超时；killed=信号关停。
   *  **仅可观测性**——不参与回收判据（判据只认指纹）。旧记录无此字段。 */
  state?: string;
  /** 运行态身份（`touch --run-pid` 写入）；未登记 ⇒ 不作判据（退回 owner-only，保守）。 */
  runPid?: number;
  runCmd?: string;
  runStart?: string;
}

/** 指纹是否**可用**（三要素齐全）：缺任一要素即无证据，不得据此判存活或已死。 */
function hasFingerprint(pid: number | undefined, cmd: string, start: string): boolean {
  return Number.isInteger(pid) && (pid as number) > 1 && !!cmd && !!start;
}

/** 指纹校验：PID 存在 ∧ 命令行/启动时刻与记录逐字相同（cmd/start 为空 ⇒ 无证据 ⇒ false）。 */
function fingerprintAlive(pid: number | undefined, cmd: string, start: string): boolean {
  if (!hasFingerprint(pid, cmd, start)) return false;
  const p = pid as number;
  try {
    process.kill(p, 0);
  } catch {
    return false;
  }
  try {
    const curCmd = execSync(`ps -o command= -p ${p}`, { encoding: 'utf-8', env: PS_ENV }).trim();
    const curStart = execSync(`ps -o lstart= -p ${p}`, { encoding: 'utf-8', env: PS_ENV }).trim();
    return curCmd === cmd && curStart === start;
  } catch {
    return false;
  }
}

/** owner（omp 会话）是否仍存活。指纹不完整 ⇒ false（无证据，无法证明存活）。 */
export function ownerAlive(l: LeaseRecord): boolean {
  return fingerprintAlive(l.ownerPid, l.ownerCmd, l.ownerStart);
}

/** owner 是否**可证明**已退出（指纹可用但校验失配）。指纹不完整 ⇒ false（无证据）。 */
export function ownerProvablyGone(l: LeaseRecord): boolean {
  return hasFingerprint(l.ownerPid, l.ownerCmd, l.ownerStart) && !ownerAlive(l);
}

/** run 是否**可证明**已退出（未登记 runPid / 指纹不完整 ⇒ false：没有证据，不作判据）。 */
export function runProvablyDead(l: LeaseRecord): boolean {
  if (l.runPid === undefined || l.runPid === null) return false;
  const cmd = l.runCmd ?? '';
  const start = l.runStart ?? '';
  if (!hasFingerprint(l.runPid, cmd, start)) return false;
  return !fingerprintAlive(l.runPid, cmd, start);
}

/** run 身份是否**可用**（登记了 runPid 且指纹三要素齐全）。 */
export function hasRunIdentity(l: LeaseRecord): boolean {
  return l.runPid !== undefined && l.runPid !== null && hasFingerprint(l.runPid, l.runCmd ?? '', l.runStart ?? '');
}

/** 可证明已放弃（自动回收的唯一判据）：
 *  `(ownerVerified ∧ owner 已退出) ∨ run 可证明已退出`。
 *
 *  owner 与 run 是**两条独立证据链**——run 证据不需要 owner 验证即可成立：
 *  终端直跑的工具（无 omp 祖先 ⇒ `ownerVerified=false`）其 run 进程（工具自身）死掉时，
 *  同样可证明「没有人会再碰这条 lease」。仅当两条链都**无证据**时才保守不判死。 */
export function leaseAbandoned(l: LeaseRecord): boolean {
  return (l.ownerVerified && ownerProvablyGone(l)) || runProvablyDead(l);
}

/** 空间分类（watcher 与门禁共用的唯一实现）：
 *  - `abandoned`：可证明已放弃 ⇒ 立即回收；
 *  - `active`   ：仍有**可用证据**证明被活着持有（owner 已验证，或登记了 run 身份）⇒ 绝不触碰；
 *  - `unverified`：lease 存在但两条证据链都不可用（终端直跑且未登记 run）⇒ 无可判定信息，
 *                 退回「无 lease」语义（观察期/超限兜底），不得当成活跃而永久豁免。 */
export type LeaseClass = 'active' | 'abandoned' | 'unverified';

export function classifyLease(l: LeaseRecord): LeaseClass {
  if (leaseAbandoned(l)) return 'abandoned';
  return l.ownerVerified || hasRunIdentity(l) ? 'active' : 'unverified';
}
