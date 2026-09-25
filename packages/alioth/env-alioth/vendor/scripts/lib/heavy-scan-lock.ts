#!/usr/bin/env bun
/**
 * heavy-scan-lock.ts — 重扫描面的**扫描级自串行**（`Backup/` 全量机密扫描、种子载荷三列门禁）
 *
 * 与 `scripts/pre/stage-lock.ts` 的分工：stage lock 是 push 阶段**执行器**互斥（区间级语义，
 * 归编排持有）；本锁只约束**同一扫描面**的并发驻留——2026-09-24 实测：5 个
 * `check-no-secrets-in-backup.ts` + 2 个 `check-seed-ak-transmission.ts` 同刻驻留，
 * 单进程 charged 4–10 GiB（RSS 被内核压缩到 1.7 GiB 而 footprint 不降），全机 JS 运行时
 * charged 合计 70 GiB / RSS 合计 48 GiB ⇒ 24 GiB 机器换页近满、low-swap jetsam 屠杀全机。
 * 峰值是**单进程**属性（逐文件堆高水位），多实例相乘只能由本锁消除。
 *
 * 锁文件：`<主 checkout>/.parallel/heavy-scan/<name>.lock`（`queueRoot` 同源 ⇒ worktree 与
 * 主 checkout 共享同一把锁；`.parallel/` 已被 `.git/info/exclude` 覆盖，不入库）。
 * 过期（staleMs）或持锁进程已退出 ⇒ 抢占；等待超时 ⇒ **fail-open 放行**并告警（内存护栏
 * MUST NOT 变成门禁死锁）；`ALIOTH_HEAVY_SCAN_NO_LOCK=1` 显式绕过（诊断用）。
 *
 * 第二件事：`runChunkedScan` —— 语料**分块换子进程**执行（块内进程内扫描，块间换新进程，
 * 退出即归还全部内存）。锁只能约束并发度，**峰值的语料相关性**由分块消除：实测单进程逐文件
 * 扫描 88 MB dump 时峰值 ≈ 1.4× 语料字节（2.3 GB 语料 = 785 MiB，若语料涨到 20 GB 会回涨到
 * GB 级）；分块后峰值 = max(单块工作集, 最大单文件)，与语料总量无关。
 */
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { pidAlive } from '../pre/proc.ts';
import { queueRoot } from '../pre/stage-lock.ts';

export interface HeavyScanLockOptions {
  /** 等待上界（默认 10 分钟；超时 fail-open 放行）。 */
  waitMs?: number;
  /** 持锁超龄判定（默认 15 分钟）。 */
  staleMs?: number;
}

const DEFAULT_WAIT_MS = Number(process.env.ALIOTH_HEAVY_SCAN_LOCK_WAIT_MS ?? 10 * 60_000);
const DEFAULT_STALE_MS = Number(process.env.ALIOTH_HEAVY_SCAN_LOCK_STALE_MS ?? 15 * 60_000);

function lockPathOf(root: string, name: string): string {
  return join(queueRoot(root), '.parallel', 'heavy-scan', `${name}.lock`);
}

function holderOf(path: string): { pid: number; at: number } | null {
  let raw: string;
  try {
    raw = readFileSync(path, 'utf8');
  } catch {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as { pid?: unknown; at?: unknown };
    const pid = typeof parsed.pid === 'number' ? parsed.pid : Number.NaN;
    const at = typeof parsed.at === 'number' ? parsed.at : 0;
    return Number.isFinite(pid) ? { pid, at } : null;
  } catch {
    return null;
  }
}

const sleep = (ms: number): Promise<void> => {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
};

/**
 * 取扫描锁执行 `fn`。返回 `fn` 的返回值（锁超时也照跑——放行优先于串行）。
 */
export async function withHeavyScanLock<T>(
  root: string,
  name: string,
  fn: () => Promise<T> | T,
  options: HeavyScanLockOptions = {},
): Promise<T> {
  if (process.env.ALIOTH_HEAVY_SCAN_NO_LOCK === '1') return await fn();

  const waitMs = options.waitMs ?? DEFAULT_WAIT_MS;
  const staleMs = options.staleMs ?? DEFAULT_STALE_MS;
  const path = lockPathOf(root, name);
  mkdirSync(dirname(path), { recursive: true });

  const mine = `${JSON.stringify({ pid: process.pid, at: Date.now() })}\n`;
  const deadline = Date.now() + waitMs;
  let notified = false;
  for (;;) {
    try {
      writeFileSync(path, mine, { flag: 'wx' });
      break;
    } catch {
      const holder = holderOf(path);
      const stale = holder === null || Date.now() - holder.at > staleMs || !pidAlive(holder.pid);
      if (stale) {
        rmSync(path, { force: true });
        continue;
      }
      if (Date.now() >= deadline) {
        process.stderr.write(
          `⚠️ heavy-scan ${name}: 等待扫描锁超过 ${Math.round(waitMs / 1000)}s（持有者 pid ${holder.pid}）`
          + '——fail-open 放行，内存护栏不阻塞门禁\n',
        );
        return await fn();
      }
      if (!notified) {
        notified = true;
        process.stderr.write(`⏳ heavy-scan ${name}: 另有扫描在跑（pid ${holder.pid}），串行等待……\n`);
      }
      await sleep(500);
    }
  }

  try {
    return await fn();
  } finally {
    const holder = holderOf(path);
    if (holder !== null && holder.pid === process.pid) rmSync(path, { force: true });
  }
}

/** 子进程协议：`--chunk-json=<临时文件>` 载入 `{files:[…]}`，逐项输出 `@@ITEM@@<json>`。 */
export const CHUNK_JSON_FLAG = '--chunk-json';
export const ITEM_PREFIX = '@@ITEM@@';

export interface ChunkedScanSpec<T> {
  /** 锁名（与 `withHeavyScanLock` 同源）。 */
  readonly name: string;
  readonly root: string;
  /** 判定面（绝对路径；顺序即输出顺序）。 */
  readonly files: readonly string[];
  /** 子进程命令行（自门禁自身路径 + 转发参数；最后一项为本块的 `--chunk-json=<path>`）。 */
  readonly childCommand: (chunkFile: string) => readonly string[];
  /** 解析子进程的一行 `@@ITEM@@` 输出；非本前缀行由调用方忽略。 */
  readonly parseItem: (line: string) => T | undefined;
  /** 进程内扫描（小块路径与子进程失败时的兜底）。 */
  readonly scanLocal: (files: readonly string[]) => readonly T[];
  /** 单块输入预算（字节）；默认 64 MiB，`ALIOTH_HEAVY_SCAN_CHUNK_BYTES` 可覆盖。 */
  readonly chunkBytes?: number;
}

/** 按累计字节切块：单文件超预算时自成一块（不切文件内部——切分会破坏语句/COPY 段边界）。 */
function chunkByBytes(files: readonly string[], budget: number): string[][] {
  const chunks: string[][] = [];
  let current: string[] = [];
  let bytes = 0;
  for (const file of files) {
    let size = 0;
    try {
      size = statSync(file).size;
    } catch {
      size = 0; // 已删除/不可读：交给扫描阶段按原语义跳过
    }
    if (current.length > 0 && bytes + size > budget) {
      chunks.push(current);
      current = [];
      bytes = 0;
    }
    current.push(file);
    bytes += size;
  }
  if (current.length > 0) chunks.push(current);
  return chunks;
}

/**
 * 分块执行重扫描。≤1 块 ⇒ 与旧行为完全一致（单进程、持锁）。多块 ⇒ 逐块换子进程，
 * 峰值与语料总量解耦。**子进程异常非零退出 ⇒ 该块回退进程内扫描并告警**（安全门禁不得
 * 静默通过；回退本身失败则抛出，门禁 fail-loud）。
 */
export async function runChunkedScan<T>(spec: ChunkedScanSpec<T>): Promise<T[]> {
  const budget = spec.chunkBytes
    ?? Number(process.env.ALIOTH_HEAVY_SCAN_CHUNK_BYTES ?? 64 * 1024 * 1024);
  const chunks = chunkByBytes(spec.files, budget);
  if (chunks.length <= 1) {
    return await withHeavyScanLock(spec.root, spec.name, () => [...spec.scanLocal(spec.files)]);
  }
  return await withHeavyScanLock(spec.root, spec.name, () => {
    const collected: T[] = [];
    chunks.forEach((chunk, index) => {
      const chunkFile = join(tmpdir(), `heavy-scan-${spec.name}-${process.pid}-${index}.json`);
      writeFileSync(chunkFile, JSON.stringify({ files: chunk }));
      try {
        const result = spawnSync(process.execPath, [...spec.childCommand(chunkFile)], {
          encoding: 'utf8',
          maxBuffer: 256 * 1024 * 1024,
          env: { ...process.env, ALIOTH_HEAVY_SCAN_NO_LOCK: '1' },
        });
        if (result.status !== 0) {
          throw new Error(`分块子进程退出 ${String(result.status)}：${(result.stderr ?? '').slice(0, 200)}`);
        }
        for (const line of (result.stdout ?? '').split('\n')) {
          if (!line.startsWith(ITEM_PREFIX)) continue;
          const item = spec.parseItem(line.slice(ITEM_PREFIX.length));
          if (item !== undefined) collected.push(item);
        }
      } catch (error) {
        process.stderr.write(
          `⚠️ heavy-scan ${spec.name}: 第 ${index + 1}/${chunks.length} 块子进程失败（${String(error)}）`
          + '——回退进程内扫描（内存峰值可能升高，结果不受影响）\n',
        );
        collected.push(...spec.scanLocal(chunk));
      } finally {
        rmSync(chunkFile, { force: true });
      }
    });
    return collected;
  });
}
