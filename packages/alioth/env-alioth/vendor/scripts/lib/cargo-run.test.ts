/// <reference types="bun" />
/**
 * cargo-run.sh 漂移守卫回归（fix-stale-build-detection）
 *
 * 事故：共享 checkout 下并行会话的 checkout/stash 会让编译读到「变化中的树」，
 * 产生「符号明明存在却报找不到」的瞬时假错（2026-09-15 实证：
 * identity-org 报 `cannot find common::leaf_relname`，串行重跑即通过）。
 *
 * 守卫语义（本测试逐条固化）：
 *   ① 编译期间源码变过 ⇒ 首轮结论不可信 ⇒ **自动重跑一次**，返回重跑结论；
 *   ② 源码未变 ⇒ **不重跑**（真实失败不得被重跑掩盖），原样返回其退出码；
 *   ③ 重跑期间源码仍在变 ⇒ 明确提示「结论仅供参照」。
 *
 * 用 PATH 前置的假 `cargo` 驱动（不触发真实编译），指纹根用 CARGO_RUN_FP_DIRS 收到夹具目录。
 */
import { describe, expect, test } from 'bun:test';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, chmodSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const REPO_ROOT = resolve(import.meta.dir, '..', '..');

interface Harness {
  dir: string;
  counterFile: string;
  srcFile: string;
  cleanup: () => void;
}

/**
 * 建夹具：假 `cargo`（行为由 scripts/fake-cargo.sh 的 mode 决定）+ 源码文件
 * @param mode `drift-then-ok`：首轮改源码并失败、次轮成功；`always-fail`：始终失败且不改源码；
 *             `drift-always`：每轮都改源码并失败
 */
function makeHarness(mode: 'drift-then-ok' | 'always-fail' | 'drift-always'): Harness {
  const dir = mkdtempSync(join(tmpdir(), 'cargo-run-guard-'));
  const binDir = join(dir, 'bin');
  const srcDir = join(dir, 'src');
  mkdirSync(binDir);
  mkdirSync(srcDir);
  const srcFile = join(srcDir, 'lib.rs');
  writeFileSync(srcFile, 'fn main() {}\n');
  const counterFile = join(dir, 'invocations');
  writeFileSync(counterFile, '0');

  const fakeCargo = join(binDir, 'cargo');
  writeFileSync(
    fakeCargo,
    [
      '#!/bin/bash',
      'set -u',
      `COUNTER="${counterFile}"`,
      `SRC="${srcFile}"`,
      `MODE="${mode}"`,
      'n=$(cat "$COUNTER"); n=$((n + 1)); printf "%s" "$n" > "$COUNTER"',
      'case "$MODE" in',
      '  drift-then-ok)',
      '    if [ "$n" -eq 1 ]; then printf "\\n// drift\\n" >> "$SRC"; echo "error: cannot find macro (transient)" >&2; exit 1; fi',
      '    echo "fake cargo ok (round $n)"; exit 0 ;;',
      '  always-fail)',
      '    echo "error[E0999]: real failure (source unchanged)" >&2; exit 1 ;;',
      '  drift-always)',
      '    printf "\\n// drift $n\\n" >> "$SRC"; echo "error: cannot find macro (transient)" >&2; exit 1 ;;',
      'esac',
    ].join('\n'),
  );
  chmodSync(fakeCargo, 0o755);

  return {
    dir,
    counterFile,
    srcFile,
    cleanup: () => rmSync(dir, { recursive: true, force: true }),
  };
}

/** 在夹具环境里跑守卫（bash + PATH 前置假 cargo + 指纹根收窄到夹具） */
function runGuard(h: Harness): { rc: number; stderr: string; invocations: number } {
  const script = [
    'set -uo pipefail',
    `export PROJECT_ROOT="${REPO_ROOT}"`,
    `export CARGO_RUN_FP_DIRS="${h.dir}/src"`,
    `export CARGO_TARGET_DIR="${h.dir}/target"`,
    `source "${REPO_ROOT}/scripts/lib/src-fingerprint.sh"`,
    `source "${REPO_ROOT}/scripts/lib/cargo-run.sh"`,
    'cargo_run_guarded check -p fake',
  ].join('\n');
  const proc = Bun.spawnSync(['bash', '-c', script], {
    env: { ...process.env, PATH: `${join(h.dir, 'bin')}:${process.env.PATH ?? ''}` },
    stdout: 'pipe',
    stderr: 'pipe',
  });
  return {
    rc: proc.exitCode ?? -1,
    stderr: proc.stderr.toString(),
    invocations: Number(readFileSync(h.counterFile, 'utf8')),
  };
}

describe('cargo-run 漂移守卫', () => {
  test('编译期间源码变化 → 自动重跑一次并返回重跑结论', () => {
    const h = makeHarness('drift-then-ok');
    try {
      const r = runGuard(h);
      expect(r.invocations).toBe(2); // 首轮 + 重跑
      expect(r.rc).toBe(0); // 重跑（源码稳定）的结论胜出
      expect(r.stderr).toContain('编译期间源码发生变化');
      expect(r.stderr).toContain('重跑完成');
    } finally {
      h.cleanup();
    }
  });

  test('源码未变的真实失败 → 不重跑，原样返回退出码', () => {
    const h = makeHarness('always-fail');
    try {
      const r = runGuard(h);
      expect(r.invocations).toBe(1); // 不得被重跑掩盖
      expect(r.rc).toBe(1);
      expect(r.stderr).not.toContain('编译期间源码发生变化');
    } finally {
      h.cleanup();
    }
  });

  test('重跑期间源码仍在变 → 明确提示结论仅供参照', () => {
    const h = makeHarness('drift-always');
    try {
      const r = runGuard(h);
      expect(r.invocations).toBe(2); // 有界：只重跑一次
      expect(r.rc).toBe(1);
      expect(r.stderr).toContain('重跑期间源码仍在变化');
    } finally {
      h.cleanup();
    }
  });
});
