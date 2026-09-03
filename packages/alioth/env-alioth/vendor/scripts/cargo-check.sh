#!/bin/bash
# cargo-check.sh — 在独立 target 目录中执行 cargo check
#
# 特性:
#   - 默认 target /tmp/alioth-check，不跟 IDE/部署构建 抢锁
#   - 默认目录被其他 cargo 进程占用时，改用「会话稳定目录」
#     /tmp/alioth-check-sess-<hash>（同一会话复用增量缓存）；
#     会话目录仍被锁或无终端标识时，退为带时间戳的唯一冷目录
#   - 启动时机会式 GC：清理超龄 sess/fallback/gate 临时目录（占锁跳过）
#   - 可通过 CARGO_TARGET_DIR 覆盖
#
# 用法:
#   bash scripts/cargo-check.sh -p wz-service-isahl-db
#   bash scripts/cargo-check.sh -p wz-service-isahl-db --all-features
#   bash scripts/cargo-check.sh --workspace

set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── 锁检测 ──────────────────────────────────────────────────────────
# 检查 target 目录的 .cargo-lock 是否被活着的进程持有
check_cargo_lock() {
  local lock_file="$1/.cargo-lock"
  [ ! -f "$lock_file" ] && return 0       # 无锁文件 → 安全
  if lsof "$lock_file" 2>/dev/null | grep -qE '\bcargo\b'; then
    return 1                               # 被活 cargo 进程锁住
  fi
  rm -f "$lock_file"                       # 死锁（进程已退出）→ 清理
  return 0
}

# ── 会话标识 ────────────────────────────────────────────────────────
# 多 agent 并发时同一终端 tab 视为同一会话（iTerm TERM_SESSION_ID，
# 其次 tty 设备），hash 8 位作为目录后缀；无终端标识 → 返回非 0。
session_key() {
  local raw=""
  if [ -n "${TERM_SESSION_ID:-}" ]; then
    raw="$TERM_SESSION_ID"
  else
    local t
    t="$(tty 2>/dev/null || true)"
    case "$t" in /dev/*) raw="$t" ;; esac
  fi
  [ -z "$raw" ] && return 1
  printf '%s' "$raw" | shasum | cut -c1-8
}

# ── 机会式 GC ───────────────────────────────────────────────────────
# 清理超龄临时编译目录（/tmp 不会自动清，多 agent 场景增长很快）：
#   /tmp/alioth-check-sess-*            空闲 ≥3 天
#   /tmp/alioth-check-<ts>-<pid>        旧版冷 fallback 孤儿，≥1 天
#   /tmp/alioth-check-gate-*            空闲 ≥7 天（当前 worktree 除外）
#   gate 内嵌的 *-sess-* / *-fallback-* ≥3 天 / ≥1 天
# 被活 cargo 进程占锁的目录跳过；输出释放空间到 stderr。
gc_tmp_targets() {
  local freed_kb=0 removed=0
  _gc_sweep() { # $1=mtime 天 $2=glob
    local days="$1" d
    shift
    for d in $1; do
      [ -d "$d" ] || continue
      # 当前 worktree 的 gate 主目录保留
      case "$d" in "$GATE_KEEP") continue ;; esac
      find "$d" -maxdepth 0 -mtime "+$days" | grep -q . || continue
      check_cargo_lock "$d" || continue
      local kb
      kb="$(du -sk "$d" 2>/dev/null | cut -f1)"
      rm -rf "$d"
      freed_kb=$((freed_kb + ${kb:-0}))
      removed=$((removed + 1))
    done
  }
  local GATE_KEEP
  GATE_KEEP="/tmp/alioth-check-gate-$(printf '%s' "$PROJECT_ROOT" | shasum | cut -c1-8)"
  _gc_sweep 3 '/tmp/alioth-check-sess-*'
  _gc_sweep 1 '/tmp/alioth-check-[0-9]*-[0-9]*'
  _gc_sweep 7 '/tmp/alioth-check-gate-*'
  _gc_sweep 3 '/tmp/alioth-check-gate-*/*-sess-*'
  _gc_sweep 1 '/tmp/alioth-check-gate-*/*-fallback-*'
  if [ "$removed" -gt 0 ]; then
    echo "[cargo-check] 🧹 GC: 清理 $removed 个超龄目录，释放 $((freed_kb / 1024)) MB" >&2
  fi
}

gc_tmp_targets

# ── target 选择（两级 fallback）────────────────────────────────────
TARGET="${CARGO_TARGET_DIR:-/tmp/alioth-check}"
if ! check_cargo_lock "$TARGET"; then
  KEY="$(session_key || true)"
  SESS=""
  [ -n "$KEY" ] && SESS="/tmp/alioth-check-sess-$KEY"
  if [ -n "$SESS" ] && check_cargo_lock "$SESS"; then
    echo "[cargo-check] ⚠️  $TARGET 被其他 cargo 进程占用，改用会话目录 $SESS" >&2
    TARGET="$SESS"
  else
    FALLBACK="/tmp/alioth-check-$(date +%s)-$$"
    echo "[cargo-check] ⚠️  $TARGET 被其他 cargo 进程占用，改用冷目录 $FALLBACK" >&2
    TARGET="$FALLBACK"
  fi
fi

export CARGO_TARGET_DIR="$TARGET"
mkdir -p "$CARGO_TARGET_DIR"

cd "$PROJECT_ROOT"
exec cargo check "$@"
