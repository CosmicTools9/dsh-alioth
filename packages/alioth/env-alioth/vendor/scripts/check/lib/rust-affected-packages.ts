#!/usr/bin/env bun
/**
 * rust-affected-packages.ts — push 阶段编译门禁的**编译面判定**助手。
 *
 * 为什么需要（2026-09-15 实测）：`check-rust-tests-compile.sh` 原先对每个相关 workspace
 * 一律 `cargo check --workspace --tests`，实测单次 16–20 分钟；且根 workspace 已含
 * `Pre-Proc/{WZ,AVIC,SE}` 的包（重复编译 + 各持独立 target dir）。
 *
 * 本工具给出两件事：
 *   ① affected：变更 .rs → 所属包 → 依赖图**反向依赖闭包** ⇒ 只编受影响的包（`-p ...`）；
 *      不可安全限定时输出 `ALL`（调用方回落 `--workspace`）。
 *   ② covers：判断某子 workspace 的包是否已被父 workspace 全覆盖（用于去重跳过）。
 *
 * 用法:
 *   bun scripts/check/lib/rust-affected-packages.ts affected --dir <ws-dir> [--limit-ratio 0.5] <changed.rs...>
 *     → stdout: `ALL` 或 `-p foo -p bar`；stderr: 人类可读摘要
 *   bun scripts/check/lib/rust-affected-packages.ts covers --parent <ws-dir> --child <ws-dir>
 *     → exit 0 = 子 workspace 的包全在父内；exit 1 = 否（stdout 打印反例）
 */
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';

interface Pkg {
  name: string;
  manifest_path: string;
}

interface Metadata {
  packages: Pkg[];
  workspace_members: string[];
  resolve?: { nodes: { id: string; dependencies: string[] }[] };
}

function readMetadata(dir: string): Metadata | null {
  const res = spawnSync(
    'cargo',
    ['metadata', '--format-version', '1', '--manifest-path', join(dir, 'Cargo.toml')],
    { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, env: { ...process.env, CARGO_NET_OFFLINE: process.env.CARGO_NET_OFFLINE ?? 'true' } },
  );
  if (res.status !== 0) return null;
  try {
    return JSON.parse(res.stdout) as Metadata;
  } catch {
    return null;
  }
}

/** workspace 内包（按 manifest 所在目录索引，用于「文件 → 所属包」最长前缀匹配）。 */
function workspacePackages(meta: Metadata): Pkg[] {
  const memberIds = new Set(meta.workspace_members);
  return meta.packages.filter((p) => memberIds.has((p as Pkg & { id: string }).id));
}

/** 变更文件 → 所属包（最长 manifest 目录前缀）。 */
function ownerOf(file: string, pkgs: Pkg[]): Pkg | null {
  const abs = resolve(file);
  let best: Pkg | null = null;
  let bestLen = -1;
  for (const pkg of pkgs) {
    const pkgDir = dirname(resolve(pkg.manifest_path));
    if ((abs === pkgDir || abs.startsWith(pkgDir + sep)) && pkgDir.length > bestLen) {
      best = pkg;
      bestLen = pkgDir.length;
    }
  }
  return best;
}

function cmdAffected(argv: string[]): number {
  let dir = '.';
  let limitRatio = 0.5;
  const files: string[] = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--dir') dir = argv[++i] ?? '.';
    else if (arg === '--limit-ratio') limitRatio = Number(argv[++i] ?? '0.5');
    else files.push(arg);
  }

  const changed = files.filter((f) => f.trim() !== '');
  if (changed.length === 0) {
    process.stdout.write('ALL\n');
    process.stderr.write('[affected] 无变更 .rs 清单 → 全量编译\n');
    return 0;
  }
  // 构建脚本/清单/lock 变更会改变依赖图或 cfg：无法安全限定 → 全量
  if (changed.some((f) => /Cargo\.(toml|lock)$|build\.rs$/.test(f))) {
    process.stdout.write('ALL\n');
    process.stderr.write('[affected] 变更含 Cargo.toml/lock/build.rs → 全量编译\n');
    return 0;
  }

  const meta = readMetadata(dir);
  if (meta === null || meta.resolve === undefined) {
    process.stdout.write('ALL\n');
    process.stderr.write('[affected] cargo metadata 不可用 → 全量编译\n');
    return 0;
  }
  const pkgs = workspacePackages(meta);
  const byId = new Map(meta.packages.map((p) => [(p as Pkg & { id: string }).id, p]));

  const seeds = new Set<string>();
  for (const f of changed) {
    const owner = ownerOf(f, pkgs);
    if (owner !== null) seeds.add((owner as Pkg & { id: string }).id);
  }
  if (seeds.size === 0) {
    process.stdout.write('ALL\n');
    process.stderr.write('[affected] 变更文件不属于本 workspace → 全量编译\n');
    return 0;
  }

  // 反向依赖闭包：谁依赖了受影响包，就要一起编（否则跨包破坏会被漏判）
  const dependents = new Map<string, Set<string>>();
  for (const node of meta.resolve.nodes) {
    for (const dep of node.dependencies) {
      const set = dependents.get(dep) ?? new Set<string>();
      set.add(node.id);
      dependents.set(dep, set);
    }
  }
  const closure = new Set(seeds);
  const queue = [...seeds];
  while (queue.length > 0) {
    const cur = queue.pop() as string;
    for (const next of dependents.get(cur) ?? []) {
      if (closure.has(next)) continue;
      closure.add(next);
      queue.push(next);
    }
  }

  const members = new Set(pkgs.map((p) => (p as Pkg & { id: string }).id));
  const inWs = [...closure].filter((id) => members.has(id));
  if (inWs.length > pkgs.length * limitRatio) {
    process.stdout.write('ALL\n');
    process.stderr.write(
      `[affected] 受影响闭包 ${inWs.length}/${pkgs.length} 超过阈值 ${Math.round(limitRatio * 100)}% → 全量编译\n`,
    );
    return 0;
  }
  const names = inWs.map((id) => byId.get(id)?.name).filter((n): n is string => typeof n === 'string').sort();
  process.stdout.write(`${names.map((n) => `-p ${n}`).join(' ')}\n`);
  process.stderr.write(
    `[affected] 变更文件 ${changed.length} → 种子包 ${seeds.size} → 闭包 ${names.length}/${pkgs.length}：${names.slice(0, 8).join(', ')}${names.length > 8 ? ' …' : ''}\n`,
  );
  return 0;
}

function cmdCovers(argv: string[]): number {
  let parent = '.';
  let child = '';
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--parent') parent = argv[++i] ?? '.';
    else if (argv[i] === '--child') child = argv[++i] ?? '';
  }
  if (child === '' || !existsSync(join(child, 'Cargo.toml'))) return 1;
  const parentMeta = readMetadata(parent);
  const childMeta = readMetadata(child);
  if (parentMeta === null || childMeta === null) return 1;
  const parentKeys = new Set(
    workspacePackages(parentMeta).map((p) => `${p.name}@${resolve(p.manifest_path)}`),
  );
  const missing = workspacePackages(childMeta)
    .map((p) => `${p.name}@${resolve(p.manifest_path)}`)
    .filter((k) => !parentKeys.has(k));
  if (missing.length > 0) {
    process.stdout.write(`未覆盖 ${missing.length} 包：${missing.slice(0, 5).join(', ')}\n`);
    return 1;
  }
  process.stdout.write(`已覆盖 ${childMeta.workspace_members.length} 包\n`);
  return 0;
}

const [sub, ...rest] = process.argv.slice(2);
if (sub === 'affected') process.exit(cmdAffected(rest));
else if (sub === 'covers') process.exit(cmdCovers(rest));
else {
  process.stderr.write('用法: rust-affected-packages.ts affected|--help ...\n');
  process.exit(2);
}
